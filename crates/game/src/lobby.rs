use std::sync::Arc;

use zphira_common::error::GameError;
use zphira_protocol::{ClientBoundPacket, PacketResult, ServerBoundPacket};

use crate::context::GameContext;
use crate::net::conn::Conn;
use crate::player::Player;
use crate::room::{
    ChatOutcome, OpResult, PlayedCheck, Room, RoomChartPool, RoomOpError, RoomSetting, Tx,
};
use crate::text;
use crate::util::mask_token;

type Reply = fn(PacketResult<()>) -> ClientBoundPacket;

pub fn is_valid_room_id(room_id: &str) -> bool {
    (1..=20).contains(&room_id.len())
        && room_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl GameContext {
    pub(crate) async fn handle_packet(&self, conn: &Arc<Conn>, packet: ServerBoundPacket) {
        // Keepalive is answered in every phase, the lobby included.
        if matches!(packet, ServerBoundPacket::Ping) {
            conn.send(&ClientBoundPacket::Pong);
            return;
        }
        let Some(player) = conn.player() else {
            match packet {
                ServerBoundPacket::Authenticate { token } => self.authenticate(conn, token).await,
                _ => conn.close(),
            }
            return;
        };
        if !player.conn_is(conn) {
            return;
        }
        match player.room() {
            Some(room) => self.handle_room_packet(conn, &player, &room, packet).await,
            None => self.handle_lobby_packet(conn, &player, packet),
        }
    }

    async fn authenticate(&self, conn: &Arc<Conn>, token: String) {
        tracing::info!("{} sent his token [{}]", conn.addr, mask_token(&token));
        match self.svc().upstream.user_by_token(&token).await {
            Ok(user) => self.login(conn, &user),
            Err(e) => {
                conn.send(&ClientBoundPacket::Authenticate(PacketResult::failed(
                    e.to_string(),
                )));
                conn.close();
            }
        }
    }

    fn localized(&self, player: &Player, error: &GameError) -> String {
        let svc = self.svc();
        error.localized(&svc.i18n, player.language(svc.default_language()))
    }

    fn handle_lobby_packet(
        &self,
        conn: &Arc<Conn>,
        player: &Arc<Player>,
        packet: ServerBoundPacket,
    ) {
        match packet {
            ServerBoundPacket::CreateRoom { room_id } => self.lobby_create(conn, player, &room_id),
            ServerBoundPacket::JoinRoom { room_id, monitor } => {
                self.lobby_join(conn, player, &room_id, monitor)
            }
            _ => self.kick(player),
        }
    }

    fn lobby_create(&self, conn: &Arc<Conn>, player: &Arc<Player>, room_id: &str) {
        if room_id == text::RANK_ROOM_ID {
            conn.send(&ClientBoundPacket::CreateRoom(PacketResult::failed(
                text::RANKING_SENT,
            )));
            self.send_point_ranking(conn, player);
            return;
        }
        let room = match self.create_player_room(room_id) {
            Ok(room) => room,
            Err(e) => {
                conn.send(&ClientBoundPacket::CreateRoom(PacketResult::failed(
                    self.localized(player, &e),
                )));
                return;
            }
        };
        conn.send(&ClientBoundPacket::CreateRoom(PacketResult::success_void()));
        let joined = room.run_spawn(|tx| tx.join(player, false, false));
        if let Err(e) = joined {
            conn.send(&ClientBoundPacket::CreateRoom(PacketResult::failed(
                self.localized(player, &e),
            )));
        }
    }

    fn create_player_room(&self, room_id: &str) -> Result<Arc<Room>, GameError> {
        if !is_valid_room_id(room_id) {
            return Err(GameError::new(text::ROOM_ID_RULE));
        }
        if self.inner.rooms.find(room_id).is_some() {
            return Err(GameError::room_already_exists());
        }
        let pools = self.svc().catalog.get_default_pools();
        if pools.is_empty() {
            return Err(GameError::new(text::NO_DEFAULT_POOL));
        }
        let pool =
            RoomChartPool::new(self.svc().catalog.clone(), &pools).map_err(GameError::new)?;
        self.register_room(room_id, RoomSetting::player_created(), pool)
    }

    fn lobby_join(&self, conn: &Arc<Conn>, player: &Arc<Player>, room_id: &str, monitor: bool) {
        let Some(room) = self.inner.rooms.find(room_id) else {
            self.send_room_not_found_hint(conn, room_id);
            conn.send(&ClientBoundPacket::JoinRoom(PacketResult::failed(
                self.localized(player, &GameError::room_not_found()),
            )));
            return;
        };
        let joined = room.run_spawn(|tx| {
            tx.join(player, monitor, true)?;
            // The client expects the join reply after the join broadcast and the vote board.
            conn.send(&tx.join_success());
            tx.fix_client_room_state(player);
            conn.send_chat(text::ROOM_GUIDE_1);
            conn.send_chat(text::ROOM_GUIDE_2);
            let summary = tx.svc().points.summary(player.id, &player.name);
            conn.send_chat(&text::points_line(summary.points, summary.rank));
            Ok::<(), GameError>(())
        });
        if let Err(e) = joined {
            conn.send(&ClientBoundPacket::JoinRoom(PacketResult::failed(
                self.localized(player, &e),
            )));
        }
    }

    fn send_room_not_found_hint(&self, conn: &Arc<Conn>, room_id: &str) {
        conn.send_chat(&text::room_not_found(room_id));
        let rooms = self.inner.rooms.all();
        if rooms.is_empty() {
            conn.send_chat(text::NO_ROOMS);
            return;
        }
        conn.send_chat(text::AVAILABLE_ROOMS);
        for room in rooms {
            let (online, total) = room
                .run(|tx| {
                    (
                        tx.s.members.online_player_count(),
                        tx.s.members.players.len(),
                    )
                })
                .0;
            conn.send_chat(&text::room_list_entry(&room.id, online, total));
        }
    }

    fn send_point_ranking(&self, conn: &Arc<Conn>, player: &Arc<Player>) {
        let points = &self.svc().points;
        let summary = points.summary(player.id, &player.name);
        conn.send_chat(text::SEPARATOR);
        conn.send_chat(text::RANKING_TITLE);
        conn.send_chat(text::RANKING_REFRESH);
        for line in points.top_ranking_lines(10) {
            conn.send_chat(&line);
        }
        conn.send_chat(&text::your_points(summary.points, summary.rank));
        conn.send_chat(text::SEPARATOR);
    }

    async fn handle_room_packet(
        &self,
        conn: &Arc<Conn>,
        player: &Arc<Player>,
        room: &Arc<Room>,
        packet: ServerBoundPacket,
    ) {
        type Op = fn(&mut Tx<'_>, &Arc<Player>) -> OpResult<()>;
        let simple: Option<(Op, Reply)> = match &packet {
            ServerBoundPacket::LockRoom { .. } => {
                Some((|tx, p| tx.lock_room(p), ClientBoundPacket::LockRoom))
            }
            ServerBoundPacket::CycleRoom { .. } => {
                Some((|tx, p| tx.cycle_room(p), ClientBoundPacket::CycleRoom))
            }
            ServerBoundPacket::RequestStart => {
                Some((|tx, p| tx.require_start(p), ClientBoundPacket::RequestStart))
            }
            ServerBoundPacket::Ready => Some((|tx, p| tx.ready(p), ClientBoundPacket::Ready)),
            ServerBoundPacket::CancelReady => {
                Some((|tx, p| tx.cancel_ready(p), ClientBoundPacket::CancelReady))
            }
            ServerBoundPacket::Abort => Some((|tx, p| tx.abort(p), ClientBoundPacket::Abort)),
            _ => None,
        };
        if let Some((op, reply)) = simple {
            self.room_op(conn, player, room, packet, reply, |tx| op(tx, player))
                .await;
            return;
        }
        match packet {
            ServerBoundPacket::Chat { message } => {
                let original = ServerBoundPacket::Chat {
                    message: message.clone(),
                };
                let mut cleared = Vec::new();
                self.room_op(
                    conn,
                    player,
                    room,
                    original,
                    ClientBoundPacket::Chat,
                    |tx| {
                        tx.chat(player, &message).map(|outcome| {
                            if let ChatOutcome::Cleared(removed) = outcome {
                                tracing::info!(
                                    "Room {} cleared by {} (#{}): {} member(s) removed",
                                    tx.room.id,
                                    player.name,
                                    player.id,
                                    removed.len()
                                );
                                cleared = removed;
                            }
                        })
                    },
                )
                .await;
                self.drop_sessions(&cleared);
            }
            ServerBoundPacket::LeaveRoom => {
                self.room_op(
                    conn,
                    player,
                    room,
                    ServerBoundPacket::LeaveRoom,
                    ClientBoundPacket::LeaveRoom,
                    |tx| {
                        tx.leave(player.id);
                        Ok(())
                    },
                )
                .await;
            }
            ServerBoundPacket::SelectChart { id } => {
                self.room_op(
                    conn,
                    player,
                    room,
                    ServerBoundPacket::SelectChart { id },
                    ClientBoundPacket::SelectChart,
                    |tx| tx.select_chart(player, id),
                )
                .await;
            }
            ServerBoundPacket::Played { id } => {
                let result = self.played(player, room, id).await;
                let reply = match result {
                    Ok(()) => PacketResult::success_void(),
                    Err(e) => PacketResult::failed(self.localized(player, &e)),
                };
                conn.send(&ClientBoundPacket::Played(reply));
            }
            ServerBoundPacket::Touches { frames } => {
                let packet = ClientBoundPacket::Touches {
                    id: player.id,
                    frames,
                };
                room.run_spawn(|tx| tx.relay_to_monitors(player, packet));
            }
            ServerBoundPacket::Judges { judges } => {
                let packet = ClientBoundPacket::Judges {
                    id: player.id,
                    judges,
                };
                room.run_spawn(|tx| tx.relay_to_monitors(player, packet));
            }
            ServerBoundPacket::Authenticate { .. }
            | ServerBoundPacket::CreateRoom { .. }
            | ServerBoundPacket::JoinRoom { .. } => self.kick(player),
            ServerBoundPacket::Ping => conn.send(&ClientBoundPacket::Pong),
            ServerBoundPacket::LockRoom { .. }
            | ServerBoundPacket::CycleRoom { .. }
            | ServerBoundPacket::RequestStart
            | ServerBoundPacket::Ready
            | ServerBoundPacket::CancelReady
            | ServerBoundPacket::Abort => {}
        }
    }

    async fn room_op(
        &self,
        conn: &Arc<Conn>,
        player: &Arc<Player>,
        room: &Arc<Room>,
        original: ServerBoundPacket,
        reply: Reply,
        op: impl FnOnce(&mut Tx<'_>) -> OpResult<()>,
    ) {
        let mut answered = false;
        // Answered under the room lock before zero-delay timers, or after its settlement.
        let (result, settle) = room.run_then(op, |result, settling| {
            if !settling && !matches!(result, Err(RoomOpError::NotMember)) {
                self.answer(conn, player, result, reply);
                answered = true;
            }
        });
        if let Some(job) = settle {
            job.run().await;
        }
        if !answered {
            match result {
                Err(RoomOpError::NotMember) => self.handle_lobby_packet(conn, player, original),
                other => self.answer(conn, player, &other, reply),
            }
        }
    }

    fn answer(&self, conn: &Arc<Conn>, player: &Arc<Player>, result: &OpResult<()>, reply: Reply) {
        match result {
            Ok(()) => conn.send(&reply(PacketResult::success_void())),
            Err(RoomOpError::Game(e)) => {
                conn.send(&reply(PacketResult::failed(self.localized(player, e))))
            }
            Err(RoomOpError::NotMember) => {}
        }
    }

    async fn played(
        &self,
        player: &Arc<Player>,
        room: &Arc<Room>,
        record_id: i32,
    ) -> Result<(), GameError> {
        let (check, _) = room.run(|tx| tx.played_check(player));
        let epoch = match check? {
            PlayedCheck::Ignore => return Ok(()),
            PlayedCheck::Fetch { epoch } => epoch,
        };
        let fetched = self.svc().upstream.record(record_id).await;
        let (result, settle) = room.run(|tx| tx.played_apply(player, epoch, fetched));
        if let Some(job) = settle {
            job.run().await;
        }
        result
    }
}
