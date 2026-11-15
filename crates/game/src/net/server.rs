use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Buf, BytesMut};
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::codec::FramedRead;
use zphira_protocol::{
    CodecError, DEFAULT_MAX_FRAME_LENGTH, HandshakeError, PRE_AUTH_MAX_FRAME_LENGTH,
    PipelineAction, ServerBoundPacket, ServerCodec, decode_handshake,
};

use super::addr::{ip_port, normalize_ip};
use super::conn::{Conn, ConnectState};
use super::proxy::{self, ProxyHeader};
use crate::context::GameContext;

pub const PACKETS_PER_SECOND: u32 = 200;

pub const AUTHENTICATED_READ_TIMEOUT_FLOOR: u64 = 600;

const HANDLER_QUEUE: usize = 64;

const WRITER_GRACE: Duration = Duration::from_secs(10);

const CLOSE_DRAIN: Duration = Duration::from_secs(2);

const CLOSE_DRAIN_BYTES: usize = 64 * 1024;

struct Slots<'a> {
    ctx: &'a GameContext,
    channel: u64,
    limited: Option<std::net::IpAddr>,
}

impl Drop for Slots<'_> {
    fn drop(&mut self) {
        if let Some(ip) = self.limited.take() {
            self.ctx.inner.limiter.release(ip);
        }
        self.ctx.unregister_channel(self.channel);
    }
}

fn io_error_text(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::ConnectionReset => "Connection reset".to_owned(),
        io::ErrorKind::BrokenPipe => "Broken pipe".to_owned(),
        io::ErrorKind::ConnectionAborted => "Software caused connection abort".to_owned(),
        io::ErrorKind::TimedOut => "Connection timed out".to_owned(),
        _ => {
            let text = e.to_string();
            match text.find(" (os error") {
                Some(i) => text[..i].to_owned(),
                None => text,
            }
        }
    }
}

async fn read_more<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut BytesMut,
    deadline: Instant,
    kill: &tokio_util::sync::CancellationToken,
) -> Early {
    tokio::select! {
        biased;
        () = kill.cancelled() => Early::Killed,
        read = reader.read_buf(buf) => Early::Read(read.map(|n| n > 0)),
        () = tokio::time::sleep_until(deadline) => Early::Deadline,
    }
}

enum Early {
    Read(io::Result<bool>),
    Deadline,
    Killed,
}

/// Closing a socket with unread input sends a reset, so the peer's leftovers are read first.
async fn drain_input<R: AsyncRead + Unpin>(reader: &mut R) {
    let deadline = Instant::now() + CLOSE_DRAIN;
    let mut scratch = vec![0u8; 4096];
    let mut total = 0;
    while total < CLOSE_DRAIN_BYTES {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => return,
            read = reader.read(&mut scratch) => match read {
                Ok(0) | Err(_) => return,
                Ok(n) => total += n,
            },
        }
    }
}

async fn close_cleanly<R, W>(mut reader: R, mut writer: W)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let _ = tokio::time::timeout(Duration::from_secs(1), writer.shutdown()).await;
    drain_input(&mut reader).await;
}

impl GameContext {
    pub async fn serve_connection<S>(&self, stream: S, peer: SocketAddr)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (channel, kill) = self.register_channel();
        let mut slots = Slots {
            ctx: self,
            channel,
            limited: None,
        };
        let svc = self.svc().clone();
        let accepted_at = Instant::now();
        let pre_auth_deadline = accepted_at + svc.options.pre_auth_deadline;
        let (mut reader, writer) = tokio::io::split(stream);
        let mut buf = BytesMut::with_capacity(512);

        let opened = self
            .open(&mut reader, &mut buf, peer, accepted_at, &kill, &mut slots)
            .await;
        let Some((addr, version)) = opened else {
            drop(slots);
            close_cleanly(reader, writer).await;
            return;
        };
        tracing::info!("Receive client version {} from {}", version, addr);

        let conn = Arc::new(Conn::new(self.next_id(), addr, kill.child_token()));
        let writer_task = tokio::spawn(conn.clone().run_writer(writer));
        let (packets, mut queue) = mpsc::channel::<ServerBoundPacket>(HANDLER_QUEUE);
        let handler = {
            let ctx = self.clone();
            let conn = conn.clone();
            tokio::spawn(async move {
                while let Some(packet) = queue.recv().await {
                    if conn.is_closed() {
                        break;
                    }
                    ctx.handle_packet(&conn, packet).await;
                }
            })
        };

        let leftover = std::io::Cursor::new(buf.to_vec());
        let mut framed = FramedRead::new(
            AsyncReadExt::chain(leftover, reader),
            ServerCodec::with_max_frame_length(PRE_AUTH_MAX_FRAME_LENGTH),
        );
        let read_timeout = Duration::from_secs(u64::from(svc.args.read_timeout_seconds));
        let authenticated_timeout = Duration::from_secs(
            u64::from(svc.args.read_timeout_seconds).max(AUTHENTICATED_READ_TIMEOUT_FLOOR),
        );
        let mut authenticated = false;
        let mut authenticate_received = false;
        let mut last_packet = Instant::now();
        let mut rate_window = Instant::now();
        let mut rate_count = 0u32;

        loop {
            if !authenticated && conn.is_authenticated() {
                authenticated = true;
                framed
                    .decoder_mut()
                    .set_max_frame_length(DEFAULT_MAX_FRAME_LENGTH);
            }
            let idle = if authenticated {
                authenticated_timeout
            } else {
                read_timeout
            };
            let mut deadline = last_packet + idle;
            if !authenticated && !authenticate_received {
                deadline = deadline.min(pre_auth_deadline);
            }
            tokio::select! {
                biased;
                () = conn.closed() => break,
                // Login raises the frame cap and the idle timeout before the next frame is decoded.
                () = conn.authenticated_changed(), if !authenticated => continue,
                next = framed.next() => match next {
                    None => break,
                    Some(Ok(packet)) => {
                        last_packet = Instant::now();
                        let now = Instant::now();
                        if now.duration_since(rate_window) >= Duration::from_secs(1) {
                            rate_window = now;
                            rate_count = 0;
                        }
                        if rate_count >= PACKETS_PER_SECOND {
                            tracing::warn!(
                                "Closing {}, packet rate exceeded {} per second",
                                conn.addr,
                                PACKETS_PER_SECOND
                            );
                            conn.close();
                            break;
                        }
                        rate_count += 1;
                        if matches!(packet, ServerBoundPacket::Authenticate { .. }) {
                            authenticate_received = true;
                        }
                        tokio::select! {
                            sent = packets.send(packet) => if sent.is_err() { break },
                            () = conn.closed() => break,
                        }
                    }
                    Some(Err(e)) => {
                        self.on_read_error(&conn, e);
                        break;
                    }
                },
                () = tokio::time::sleep_until(deadline) => {
                    tracing::error!("{}: read timed out", conn.addr);
                    conn.close_as(ConnectState::Timeout);
                    break;
                }
            }
        }

        conn.close();
        tracing::info!("Client disconnected: {} ({})", conn.addr, conn.state());
        drop(slots);
        drop(packets);
        let mut reader = framed.into_inner();
        // The packet in progress completes before the close handler runs.
        let _ = handler.await;
        self.on_connection_closed(&conn);
        let mut writer_task = writer_task;
        if tokio::time::timeout(WRITER_GRACE, &mut writer_task)
            .await
            .is_err()
        {
            writer_task.abort();
        }
        drain_input(&mut reader).await;
    }

    /// The PROXY header, the connection limit and the version byte; `None` once the socket is to be closed.
    async fn open<R: AsyncRead + Unpin>(
        &self,
        reader: &mut R,
        buf: &mut BytesMut,
        peer: SocketAddr,
        accepted_at: Instant,
        kill: &tokio_util::sync::CancellationToken,
        slots: &mut Slots<'_>,
    ) -> Option<(String, u8)> {
        let svc = self.svc();
        let pre_auth_deadline = accepted_at + svc.options.pre_auth_deadline;
        let mut real = SocketAddr::new(normalize_ip(peer.ip()), peer.port());
        if svc.args.proxy_protocol {
            let original = ip_port(peer);
            let deadline = (accepted_at + svc.options.proxy_header_timeout).min(pre_auth_deadline);
            loop {
                match proxy::parse(buf) {
                    Ok(ProxyHeader::Source { source, consumed }) => {
                        buf.advance(consumed);
                        real = SocketAddr::new(normalize_ip(source.ip()), source.port());
                        break;
                    }
                    Ok(ProxyHeader::Incomplete) => {}
                    Err(message) => {
                        tracing::warn!(
                            "Disconnecting {} on HAProxy handshaking: {}",
                            original,
                            message
                        );
                        return None;
                    }
                }
                let message = match read_more(reader, buf, deadline, kill).await {
                    Early::Read(Ok(true)) => continue,
                    Early::Read(Ok(false)) => {
                        "Connection closed before proxy protocol handshake".to_owned()
                    }
                    Early::Read(Err(e)) => io_error_text(&e),
                    Early::Deadline => "Proxy protocol handshake timeout".to_owned(),
                    Early::Killed => return None,
                };
                tracing::warn!(
                    "Disconnecting {} on HAProxy handshaking: {}",
                    original,
                    message
                );
                return None;
            }
        }

        let addr = ip_port(real);
        if let Err(reason) = self.inner.limiter.accept(real.ip()) {
            tracing::warn!("Refusing connection from {}: {}", addr, reason);
            return None;
        }
        slots.limited = Some(real.ip());
        tracing::info!("Establishing a connection from {}", addr);

        let handshake_deadline = if svc.args.proxy_protocol {
            pre_auth_deadline
        } else {
            (accepted_at + svc.options.handshake_timeout).min(pre_auth_deadline)
        };
        let version = loop {
            match decode_handshake(buf) {
                Ok(Some(version)) => break version,
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("Disconnecting {} on Phira handshaking: {}", addr, e);
                    return None;
                }
            }
            let message = match read_more(reader, buf, handshake_deadline, kill).await {
                Early::Read(Ok(true)) => continue,
                Early::Read(Ok(false)) => HandshakeError::ConnectionClosed.to_string(),
                Early::Read(Err(e)) => io_error_text(&e),
                Early::Deadline => HandshakeError::Timeout.to_string(),
                Early::Killed => return None,
            };
            tracing::warn!("Disconnecting {} on Phira handshaking: {}", addr, message);
            return None;
        };
        Some((addr, version))
    }

    fn on_read_error(&self, conn: &Arc<Conn>, error: CodecError) {
        match error {
            CodecError::Protocol(e) => match e.pipeline_action() {
                PipelineAction::Close => {
                    tracing::debug!("{}: closing on malformed packet ({})", conn.addr, e);
                    conn.close();
                }
                PipelineAction::WarnAndClose => {
                    tracing::warn!("Closing {}, {}", conn.addr, e);
                    conn.close();
                }
            },
            CodecError::Io(e) => {
                tracing::info!("{}: {}", conn.addr, io_error_text(&e));
                conn.close_as(ConnectState::Error);
            }
            CodecError::Handshake(e) => {
                tracing::debug!("{}: closing on malformed packet ({})", conn.addr, e);
                conn.close();
            }
        }
    }
}
