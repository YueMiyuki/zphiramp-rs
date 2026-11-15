use std::sync::Arc;

use zphira_phira::UserInfo;
use zphira_protocol::{AuthenticateOk, ClientBoundPacket, FullUserProfile, PacketResult};

use crate::context::GameContext;
use crate::net::conn::{Conn, ConnectState};
use crate::player::{Player, Session, Suspension};
use crate::room::SuspendOutcome;
use crate::text;
use crate::util::lock;

impl GameContext {
    pub(crate) fn login(&self, conn: &Arc<Conn>, user: &UserInfo) {
        let profile = FullUserProfile::new(user.id, user.name_or_null(), false);
        loop {
            let player = {
                let mut players = lock(&self.inner.players);
                players
                    .entry(user.id)
                    .or_insert_with(|| {
                        Arc::new(Player::new(
                            user.id,
                            user.name_or_null().to_owned(),
                            user.language.clone(),
                        ))
                    })
                    .clone()
            };
            // Lock order: session, then room, then leaf locks; room code never locks a session.
            let mut session = lock(&player.session);
            if session.removed {
                continue;
            }
            // The socket closed during the token check: bind nothing, take nothing over.
            if conn.is_closed() {
                if player.conn().is_none() && session.suspended.is_none() {
                    self.remove_player(&player, &mut session);
                }
                return;
            }
            if let Some(suspension) = session.suspended.take() {
                suspension.timer.abort();
            }
            let previous = player.conn().filter(|c| !Arc::ptr_eq(c, conn));

            let mut answered = false;
            if let Some(room) = player.room() {
                answered = room.run_spawn(|tx| {
                    if !tx.s.members.is_member(player.id) {
                        player.set_room(None);
                        return false;
                    }
                    self.bind_connection(conn, &player, previous.as_ref());
                    conn.send(&ClientBoundPacket::Authenticate(PacketResult::success(
                        AuthenticateOk {
                            user_profile: profile.clone(),
                            room_info: Some(tx.room_info()),
                        },
                    )));
                    self.send_welcome(conn, &player);
                    tx.fix_client_room_state(&player);
                    tx.reevaluate();
                    true
                });
            }
            if !answered {
                self.bind_connection(conn, &player, previous.as_ref());
                conn.send(&ClientBoundPacket::Authenticate(PacketResult::success(
                    AuthenticateOk {
                        user_profile: profile.clone(),
                        room_info: None,
                    },
                )));
                self.send_welcome(conn, &player);
            }
            drop(session);
            tracing::info!(
                "{} has logged in as [{}] {}",
                conn.addr,
                user.id,
                user.name_or_null()
            );
            return;
        }
    }

    fn bind_connection(
        &self,
        conn: &Arc<Conn>,
        player: &Arc<Player>,
        previous: Option<&Arc<Conn>>,
    ) {
        player.set_conn(Some(conn.clone()));
        conn.bind_player(player.clone());
        conn.set_authenticated();
        // Bound before the old connection closes, so its teardown sees it is no longer current.
        if let Some(old) = previous {
            let svc = self.svc();
            let message = svc.i18n.message(
                player.language(svc.default_language()),
                text::KEY_LOGGED_IN_ELSEWHERE,
            );
            old.send_chat(&message);
            old.close_as(ConnectState::Duplicate);
        }
    }

    fn send_welcome(&self, conn: &Arc<Conn>, player: &Arc<Player>) {
        let summary = self.svc().points.summary(player.id, &player.name);
        conn.send_chat(text::SEPARATOR);
        conn.send_chat(text::WELCOME);
        conn.send_chat(&text::welcome_player(&player.name, player.id));
        conn.send_chat(&text::points_line(summary.points, summary.rank));
        conn.send_chat(text::WELCOME_ROOM_HINT);
        conn.send_chat(text::SEPARATOR);
    }

    pub(crate) fn on_connection_closed(&self, conn: &Arc<Conn>) {
        let Some(player) = conn.player() else {
            return;
        };
        let mut session = lock(&player.session);
        // Only the connection that is still current may suspend or remove the player.
        if session.removed || !player.conn_is(conn) {
            return;
        }
        let outcome = match player.room() {
            None => SuspendOutcome::NotInRoom,
            Some(room) => room.run_spawn(|tx| {
                if tx.s.members.is_member(player.id) {
                    tx.suspend(&player)
                } else {
                    SuspendOutcome::NotInRoom
                }
            }),
        };
        match outcome {
            SuspendOutcome::NotInRoom => self.remove_player(&player, &mut session),
            SuspendOutcome::Suspended => {
                let token = self.next_id();
                let timeout = self.svc().options.suspend_timeout;
                let weak = Arc::downgrade(&self.inner);
                let target = player.clone();
                let timer = self.svc().runtime.spawn(async move {
                    tokio::time::sleep(timeout).await;
                    if let Some(inner) = weak.upgrade() {
                        GameContext { inner }.on_suspend_timeout(&target, token);
                    }
                });
                session.suspended = Some(Suspension {
                    token,
                    timer: timer.abort_handle(),
                });
            }
        }
    }

    fn on_suspend_timeout(&self, player: &Arc<Player>, token: u64) {
        let mut session = lock(&player.session);
        // A suspension that was resumed or replaced meanwhile is left alone.
        if session.removed || session.suspended.as_ref().map(|s| s.token) != Some(token) {
            return;
        }
        session.suspended = None;
        self.remove_player(player, &mut session);
        if let Some(room) = player.room() {
            room.run_spawn(|tx| {
                tx.leave(player.id);
            });
        }
    }

    pub(crate) fn remove_player(&self, player: &Arc<Player>, session: &mut Session) {
        session.removed = true;
        let mut players = lock(&self.inner.players);
        if players
            .get(&player.id)
            .is_some_and(|p| Arc::ptr_eq(p, player))
        {
            players.remove(&player.id);
        }
    }

    pub(crate) fn kick(&self, player: &Arc<Player>) {
        if let Some(room) = player.room() {
            room.run_spawn(|tx| {
                tx.leave(player.id);
            });
        }
        if let Some(conn) = player.conn() {
            conn.close_as(ConnectState::Kick);
        }
    }

    pub(crate) fn drop_sessions(&self, removed: &[Arc<Player>]) {
        for player in removed {
            let mut session = lock(&player.session);
            if session.removed {
                continue;
            }
            if let Some(suspension) = session.suspended.take() {
                suspension.timer.abort();
                self.remove_player(player, &mut session);
            }
        }
    }

    pub fn online_count(&self) -> usize {
        lock(&self.inner.players)
            .values()
            .filter(|p| p.is_online())
            .count()
    }

    pub fn registered_count(&self) -> usize {
        lock(&self.inner.players).len()
    }
}
