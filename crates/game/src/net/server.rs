use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Buf, BytesMut};
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::codec::FramedRead;
use zphira_protocol::{
    CodecError, DEFAULT_MAX_FRAME_LENGTH, HandshakeError, PRE_AUTH_MAX_FRAME_LENGTH,
    PipelineAction, ServerBoundPacket, ServerCodec, decode_handshake,
};

use super::addr::{ip_port, java_socket_address, normalize_ip};
use super::conn::{Conn, ConnectState};
use super::proxy::{self, ProxyHeader};
use crate::context::GameContext;

pub const PACKETS_PER_SECOND: u32 = 200;

pub const AUTHENTICATED_READ_TIMEOUT_FLOOR: u64 = 600;

const HANDLER_QUEUE: usize = 64;

const WRITER_GRACE: Duration = Duration::from_secs(10);

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

fn java_io_message(e: &io::Error) -> String {
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

        let mut real = SocketAddr::new(normalize_ip(peer.ip()), peer.port());
        if svc.args.proxy_protocol {
            let original = java_socket_address(peer);
            let deadline = (accepted_at + svc.options.proxy_header_timeout).min(pre_auth_deadline);
            loop {
                match proxy::parse(&buf) {
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
                        return;
                    }
                }
                let message = match read_more(&mut reader, &mut buf, deadline, &kill).await {
                    Early::Read(Ok(true)) => continue,
                    Early::Read(Ok(false)) => {
                        "Connection closed before proxy protocol handshake".to_owned()
                    }
                    Early::Read(Err(e)) => java_io_message(&e),
                    Early::Deadline => "Proxy protocol handshake timeout".to_owned(),
                    Early::Killed => return,
                };
                tracing::warn!(
                    "Disconnecting {} on HAProxy handshaking: {}",
                    original,
                    message
                );
                return;
            }
        }

        let addr = ip_port(real);
        if let Err(reason) = self.inner.limiter.accept(real.ip()) {
            tracing::warn!("Refusing connection from {}: {}", addr, reason);
            return;
        }
        slots.limited = Some(real.ip());
        tracing::info!("Establishing a connection from {}", addr);

        let handshake_deadline = if svc.args.proxy_protocol {
            pre_auth_deadline
        } else {
            (accepted_at + svc.options.handshake_timeout).min(pre_auth_deadline)
        };
        let version = loop {
            match decode_handshake(&mut buf) {
                Ok(Some(version)) => break version,
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("Disconnecting {} on Phira handshaking: {}", addr, e);
                    return;
                }
            }
            let message = match read_more(&mut reader, &mut buf, handshake_deadline, &kill).await {
                Early::Read(Ok(true)) => continue,
                Early::Read(Ok(false)) => HandshakeError::ConnectionClosed.to_string(),
                Early::Read(Err(e)) => java_io_message(&e),
                Early::Deadline => HandshakeError::Timeout.to_string(),
                Early::Killed => return,
            };
            tracing::warn!("Disconnecting {} on Phira handshaking: {}", addr, message);
            return;
        };
        tracing::info!("Receive client version {} from {}", version, addr);

        let conn = Arc::new(Conn::new(self.next_id(), addr.clone(), kill.child_token()));
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
        drop(framed);
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
    }

    fn on_read_error(&self, conn: &Arc<Conn>, error: CodecError) {
        match error {
            CodecError::Protocol(e) => match e.pipeline_action() {
                PipelineAction::CloseSilently => {
                    tracing::debug!("{}: closing on malformed packet ({})", conn.addr, e);
                    conn.close();
                }
                PipelineAction::ExceptionCaught => {
                    tracing::error!(error = %e, "{}: exception encountered", conn.addr);
                    conn.close_as(ConnectState::Error);
                }
                PipelineAction::WarnAndClose => {
                    tracing::warn!("Closing {}, {}", conn.addr, e);
                    conn.close();
                }
            },
            CodecError::Io(e) => {
                tracing::info!("{}: {}", conn.addr, java_io_message(&e));
                conn.close_as(ConnectState::Error);
            }
            CodecError::Handshake(e) => {
                tracing::error!(error = %e, "{}: exception encountered", conn.addr);
                conn.close_as(ConnectState::Error);
            }
        }
    }
}
