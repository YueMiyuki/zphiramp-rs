use std::sync::{Arc, Mutex};

use bytes::Bytes;
use tokio::task::AbortHandle;
use zphira_protocol::ClientBoundPacket;

use crate::net::conn::{Conn, chat_frame};
use crate::room::Room;
use crate::text::SYSTEM_SENDER;
use crate::util::lock;

#[derive(Debug, Default)]
pub(crate) struct Session {
    pub(crate) removed: bool,
    pub(crate) suspended: Option<Suspension>,
}

#[derive(Debug)]
pub(crate) struct Suspension {
    pub(crate) token: u64,
    pub(crate) timer: AbortHandle,
}

pub(crate) struct Player {
    pub(crate) id: i32,
    pub(crate) name: String,
    pub(crate) language: Option<String>,
    conn: Mutex<Option<Arc<Conn>>>,
    room: Mutex<Option<Arc<Room>>>,
    pub(crate) session: Mutex<Session>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Player {
    pub(crate) fn new(id: i32, name: String, language: Option<String>) -> Self {
        Self {
            id,
            name,
            language,
            conn: Mutex::new(None),
            room: Mutex::new(None),
            session: Mutex::new(Session::default()),
        }
    }

    pub(crate) fn is_online(&self) -> bool {
        lock(&self.conn).as_ref().is_some_and(|c| !c.is_closed())
    }

    pub(crate) fn conn(&self) -> Option<Arc<Conn>> {
        lock(&self.conn).clone()
    }

    pub(crate) fn conn_is(&self, conn: &Arc<Conn>) -> bool {
        lock(&self.conn)
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(c, conn))
    }

    pub(crate) fn set_conn(&self, conn: Option<Arc<Conn>>) {
        *lock(&self.conn) = conn;
    }

    pub(crate) fn send(&self, packet: &ClientBoundPacket) {
        if let Some(conn) = self.open_conn() {
            conn.send(packet);
        }
    }

    pub(crate) fn send_frame(&self, frame: &Bytes) {
        if let Some(conn) = self.open_conn() {
            conn.send_frame(frame.clone());
        }
    }

    pub(crate) fn send_chat(&self, text: &str) {
        if let Some(conn) = self.open_conn() {
            conn.send_frame(chat_frame(SYSTEM_SENDER, text));
        }
    }

    fn open_conn(&self) -> Option<Arc<Conn>> {
        lock(&self.conn).clone().filter(|c| !c.is_closed())
    }

    pub(crate) fn room(&self) -> Option<Arc<Room>> {
        lock(&self.room).clone()
    }

    pub(crate) fn set_room(&self, room: Option<Arc<Room>>) {
        *lock(&self.room) = room;
    }

    pub(crate) fn language<'a>(&'a self, default: &'a str) -> &'a str {
        self.language.as_deref().unwrap_or(default)
    }
}
