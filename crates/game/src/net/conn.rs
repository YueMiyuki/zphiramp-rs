use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zphira_protocol::{ClientBoundPacket, Message, encode_frame};

use crate::player::Player;
use crate::text::SYSTEM_SENDER;
use crate::util::lock;

pub const HIGH_WATER_MARK: usize = 2 * 1024 * 1024;
pub const LOW_WATER_MARK: usize = 512 * 1024;

const CLOSE_FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectState {
    Active,
    Kick,
    Timeout,
    Duplicate,
    Error,
}

impl fmt::Display for ConnectState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ConnectState::Active => "ACTIVE",
            ConnectState::Kick => "KICK",
            ConnectState::Timeout => "TIMEOUT",
            ConnectState::Duplicate => "DUPLICATE",
            ConnectState::Error => "ERROR",
        })
    }
}

#[derive(Debug, Default)]
struct Outbound {
    queue: VecDeque<Bytes>,
    pending: usize,
    unwritable: bool,
    dead: bool,
}

pub(crate) struct Conn {
    pub(crate) id: u64,
    pub(crate) addr: String,
    out: Mutex<Outbound>,
    writer_wake: Notify,
    closed: CancellationToken,
    state: Mutex<ConnectState>,
    authenticated: AtomicBool,
    auth_wake: Notify,
    player: Mutex<Option<Arc<Player>>>,
}

impl fmt::Debug for Conn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conn")
            .field("id", &self.id)
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

pub(crate) fn frame_of(packet: &ClientBoundPacket) -> Bytes {
    let mut payload = BytesMut::new();
    packet.encode(&mut payload);
    let mut out = BytesMut::with_capacity(payload.len() + 5);
    let _ = encode_frame(&payload, &mut out);
    out.freeze()
}

pub(crate) fn chat_frame(sender: i32, text: &str) -> Bytes {
    frame_of(&ClientBoundPacket::Message(Message::Chat {
        user: sender,
        content: text.to_owned(),
    }))
}

impl Conn {
    pub(crate) fn new(id: u64, addr: String, closed: CancellationToken) -> Self {
        Self {
            id,
            addr,
            out: Mutex::new(Outbound::default()),
            writer_wake: Notify::new(),
            closed,
            state: Mutex::new(ConnectState::Active),
            authenticated: AtomicBool::new(false),
            auth_wake: Notify::new(),
            player: Mutex::new(None),
        }
    }

    pub(crate) fn send(&self, packet: &ClientBoundPacket) {
        if self.is_closed() {
            return;
        }
        self.send_frame(frame_of(packet));
    }

    pub(crate) fn send_chat(&self, text: &str) {
        if self.is_closed() {
            return;
        }
        self.send_frame(chat_frame(SYSTEM_SENDER, text));
    }

    pub(crate) fn send_frame(&self, frame: Bytes) {
        if self.is_closed() {
            return;
        }
        let mut out = lock(&self.out);
        if out.dead {
            return;
        }
        if out.unwritable {
            drop(out);
            tracing::warn!(
                "Dropping outbound packet to {}, buffer is backed up",
                self.addr
            );
            return;
        }
        out.pending += frame.len();
        out.queue.push_back(frame);
        if out.pending > HIGH_WATER_MARK {
            out.unwritable = true;
        }
        drop(out);
        self.writer_wake.notify_one();
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.is_cancelled()
    }

    pub(crate) async fn closed(&self) {
        self.closed.cancelled().await
    }

    pub(crate) fn close(&self) {
        let _state = lock(&self.state);
        self.closed.cancel();
        self.writer_wake.notify_one();
    }

    pub(crate) fn close_as(&self, reason: ConnectState) {
        let mut state = lock(&self.state);
        if !self.closed.is_cancelled() {
            *state = reason;
            self.closed.cancel();
        }
        drop(state);
        self.writer_wake.notify_one();
    }

    pub(crate) fn state(&self) -> ConnectState {
        *lock(&self.state)
    }

    pub(crate) fn is_authenticated(&self) -> bool {
        self.authenticated.load(Ordering::Acquire)
    }

    pub(crate) fn set_authenticated(&self) {
        self.authenticated.store(true, Ordering::Release);
        self.auth_wake.notify_one();
    }

    pub(crate) async fn authenticated_changed(&self) {
        self.auth_wake.notified().await
    }

    pub(crate) fn player(&self) -> Option<Arc<Player>> {
        lock(&self.player).clone()
    }

    pub(crate) fn bind_player(&self, player: Arc<Player>) {
        *lock(&self.player) = Some(player);
    }

    fn take_batch(&self) -> Option<Vec<Bytes>> {
        let mut out = lock(&self.out);
        if out.queue.is_empty() {
            None
        } else {
            Some(out.queue.drain(..).collect())
        }
    }

    fn written(&self, bytes: usize) {
        let mut out = lock(&self.out);
        out.pending = out.pending.saturating_sub(bytes);
        if out.unwritable && out.pending < LOW_WATER_MARK {
            out.unwritable = false;
        }
    }

    fn mark_dead(&self) {
        let mut out = lock(&self.out);
        out.dead = true;
        out.queue.clear();
        out.pending = 0;
    }

    pub(crate) async fn run_writer<W: AsyncWrite + Unpin>(self: Arc<Self>, mut socket: W) {
        loop {
            match self.take_batch() {
                Some(batch) => {
                    let mut total = 0;
                    let mut failed = false;
                    for frame in &batch {
                        total += frame.len();
                        let write = socket.write_all(frame);
                        let result = if self.is_closed() {
                            tokio::time::timeout(CLOSE_FLUSH_TIMEOUT, write)
                                .await
                                .unwrap_or_else(|_| {
                                    Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
                                })
                        } else {
                            write.await
                        };
                        if result.is_err() {
                            failed = true;
                            break;
                        }
                    }
                    if !failed && socket.flush().await.is_err() {
                        failed = true;
                    }
                    self.written(total);
                    if failed {
                        self.mark_dead();
                        break;
                    }
                }
                None => {
                    if self.is_closed() {
                        break;
                    }
                    self.writer_wake.notified().await;
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.shutdown()).await;
    }
}
