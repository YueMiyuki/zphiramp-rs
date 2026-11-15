mod playing;
mod pool;
mod ready;
pub(crate) mod registry;
mod select;
mod setting;
pub(crate) mod store;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use bytes::Bytes;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use zphira_common::error::GameError;
use zphira_protocol::{
    ClientBoundPacket, FullUserProfile, GameState, JoinRoomOk, Message, RoomInfo, UserProfile,
};

use crate::net::conn::{ConnectState, chat_frame, frame_of};
use crate::player::Player;
use crate::services::Services;
use crate::text;
use crate::util::lock;

pub(crate) use playing::{PlayState, PlayedCheck, SettleJob};
pub use pool::{EMPTY_POOL_LIST, RoomChartPool, Rotation};
pub(crate) use ready::ReadyState;
pub(crate) use registry::RoomRegistry;
pub(crate) use select::SelectState;
pub use setting::{
    DEFAULT_ALL_READY_DELAY_SECONDS, MAX_ALL_READY_DELAY_SECONDS, RoomSetting,
    effective_force_finish_seconds,
};

const WAIT_FOR_READY_RESEND: Duration = Duration::from_millis(100);

pub(crate) enum Phase {
    Select(SelectState),
    Ready(ReadyState),
    Playing(PlayState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhaseKind {
    Select,
    Ready,
    Playing,
}

impl Phase {
    pub(crate) fn kind(&self) -> PhaseKind {
        match self {
            Phase::Select(_) => PhaseKind::Select,
            Phase::Ready(_) => PhaseKind::Ready,
            Phase::Playing(_) => PhaseKind::Playing,
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Phase::Select(_) => "SelectChart",
            Phase::Ready(_) => "WaitForReady",
            Phase::Playing(_) => "Playing",
        }
    }

    pub(crate) fn chart(&self) -> Option<&Arc<zphira_phira::ChartInfo>> {
        match self {
            Phase::Select(_) => None,
            Phase::Ready(r) => Some(&r.chart),
            Phase::Playing(p) => Some(&p.chart),
        }
    }

    fn reported_state(&self) -> GameState {
        match self {
            // The client expects SelectChart while playing, SelectChart(chart) in the ready phase.
            Phase::Select(_) | Phase::Playing(_) => GameState::SelectChart(None),
            Phase::Ready(r) => GameState::SelectChart(Some(r.chart.id)),
        }
    }
}

pub(crate) struct Member {
    pub(crate) player: Arc<Player>,
    seq: u64,
}

#[derive(Default)]
pub(crate) struct Members {
    pub(crate) players: BTreeMap<i32, Member>,
    pub(crate) monitors: BTreeMap<i32, Arc<Player>>,
    next_seq: u64,
}

impl Members {
    pub(crate) fn broadcast_frame(&self, frame: &Bytes) {
        for member in self.players.values() {
            member.player.send_frame(frame);
        }
        for monitor in self.monitors.values() {
            monitor.send_frame(frame);
        }
    }

    pub(crate) fn broadcast(&self, packet: &ClientBoundPacket) {
        self.broadcast_frame(&frame_of(packet));
    }

    pub(crate) fn broadcast_chat(&self, text: &str) {
        self.broadcast_frame(&chat_frame(text::SYSTEM_SENDER, text));
    }

    pub(crate) fn broadcast_to_monitors(&self, packet: &ClientBoundPacket) {
        let frame = frame_of(packet);
        for monitor in self.monitors.values() {
            monitor.send_frame(&frame);
        }
    }

    pub(crate) fn is_player(&self, id: i32) -> bool {
        self.players.contains_key(&id)
    }

    pub(crate) fn is_monitor(&self, id: i32) -> bool {
        self.monitors.contains_key(&id)
    }

    pub(crate) fn is_member(&self, id: i32) -> bool {
        self.is_player(id) || self.is_monitor(id)
    }

    pub(crate) fn player(&self, id: i32) -> Option<&Arc<Player>> {
        self.players.get(&id).map(|m| &m.player)
    }

    pub(crate) fn member(&self, id: i32) -> Option<&Arc<Player>> {
        self.player(id).or_else(|| self.monitors.get(&id))
    }

    pub(crate) fn online_player_count(&self) -> usize {
        self.players
            .values()
            .filter(|m| m.player.is_online())
            .count()
    }

    pub(crate) fn online_players(&self) -> impl Iterator<Item = &Arc<Player>> {
        self.players
            .values()
            .map(|m| &m.player)
            .filter(|p| p.is_online())
    }

    pub(crate) fn online_monitors(&self) -> impl Iterator<Item = &Arc<Player>> {
        self.monitors.values().filter(|p| p.is_online())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.players.is_empty() && self.monitors.is_empty()
    }

    pub(crate) fn player_profiles(&self) -> Vec<UserProfile> {
        self.players
            .values()
            .map(|m| UserProfile::new(m.player.id, m.player.name.clone()))
            .collect()
    }

    pub(crate) fn monitor_profiles(&self) -> Vec<UserProfile> {
        self.monitors
            .values()
            .map(|p| UserProfile::new(p.id, p.name.clone()))
            .collect()
    }

    fn earliest_player(&self) -> Option<i32> {
        self.players
            .iter()
            .min_by_key(|(_, m)| m.seq)
            .map(|(id, _)| *id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerKind {
    SelectNotice(i32),
    SelectFinish,
    ReadyNotice(i32),
    ReadyFinish,
    ForceNotice,
    ForceFinish,
    ResendWaitForReady(i32),
    AllReadyStart(u64),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TimerEvent {
    epoch: u64,
    countdown: Option<u64>,
    kind: TimerKind,
}

#[derive(Default)]
pub(crate) struct Timers {
    epoch: u64,
    countdown_gen: u64,
    phase: Vec<AbortHandle>,
    countdown: Vec<AbortHandle>,
    immediate: VecDeque<TimerEvent>,
}

impl Timers {
    fn new_phase(&mut self) {
        for h in self.phase.drain(..).chain(self.countdown.drain(..)) {
            h.abort();
        }
        self.immediate.clear();
        self.epoch += 1;
        self.countdown_gen += 1;
    }

    fn cancel_countdown(&mut self) {
        for h in self.countdown.drain(..) {
            h.abort();
        }
        self.immediate.retain(|e| e.countdown.is_none());
        self.countdown_gen += 1;
    }

    fn abort_phase_timers(&mut self) {
        for h in self.phase.drain(..) {
            h.abort();
        }
        self.immediate.retain(|e| e.countdown.is_some());
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
}

pub(crate) struct RoomInner {
    pub(crate) destroyed: bool,
    pub(crate) frozen: bool,
    pub(crate) setting: RoomSetting,
    pub(crate) members: Members,
    pub(crate) host: Option<i32>,
    pub(crate) pool: RoomChartPool,
    pub(crate) phase: Phase,
    pub(crate) timers: Timers,
    pub(crate) empty_since: Option<Instant>,
}

pub(crate) struct Room {
    pub(crate) id: String,
    me: Weak<Room>,
    pub(crate) svc: Arc<Services>,
    registry: Weak<RoomRegistry>,
    inner: Mutex<RoomInner>,
}

impl std::fmt::Debug for Room {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Room")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RoomOpError {
    Game(GameError),
    NotMember,
}

impl From<GameError> for RoomOpError {
    fn from(e: GameError) -> Self {
        RoomOpError::Game(e)
    }
}

pub(crate) type OpResult<T> = Result<T, RoomOpError>;

pub(crate) enum ChatOutcome {
    Sent,
    Cleared(Vec<Arc<Player>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuspendOutcome {
    Suspended,
    NotInRoom,
}

impl Room {
    pub(crate) fn new(
        id: String,
        svc: Arc<Services>,
        registry: Weak<RoomRegistry>,
        setting: RoomSetting,
        pool: RoomChartPool,
    ) -> Arc<Room> {
        let select = SelectState::new(&pool, &svc.catalog);
        Arc::new_cyclic(|me| Room {
            id,
            me: me.clone(),
            svc,
            registry,
            inner: Mutex::new(RoomInner {
                destroyed: false,
                frozen: false,
                setting,
                members: Members::default(),
                host: None,
                pool,
                phase: Phase::Select(select),
                timers: Timers::default(),
                empty_since: Some(Instant::now()),
            }),
        })
    }

    pub(crate) fn arc(&self) -> Option<Arc<Room>> {
        self.me.upgrade()
    }

    pub(crate) fn run<R>(&self, f: impl FnOnce(&mut Tx<'_>) -> R) -> (R, Option<SettleJob>) {
        self.run_then(f, |_, _| {})
    }

    pub(crate) fn run_then<R>(
        &self,
        f: impl FnOnce(&mut Tx<'_>) -> R,
        then: impl FnOnce(&R, bool),
    ) -> (R, Option<SettleJob>) {
        let mut tx = Tx {
            room: self,
            s: lock(&self.inner),
            destroy: false,
            settle: None,
        };
        let result = f(&mut tx);
        then(&result, tx.settle.is_some());
        tx.drain_immediate();
        let Tx {
            s, destroy, settle, ..
        } = tx;
        drop(s);
        if destroy && let Some(registry) = self.registry.upgrade() {
            registry.remove_if_same(&self.id, self);
        }
        (result, settle)
    }

    pub(crate) fn run_spawn<R>(&self, f: impl FnOnce(&mut Tx<'_>) -> R) -> R {
        let (result, settle) = self.run(f);
        if let Some(job) = settle {
            job.spawn();
        }
        result
    }

    fn fire(&self, event: TimerEvent) {
        self.run_spawn(|tx| tx.on_timer(event));
    }

    pub(crate) fn is_destroyed(&self) -> bool {
        lock(&self.inner).destroyed
    }

    pub(crate) fn can_control(&self, user_id: i32) -> bool {
        let s = lock(&self.inner);
        can_control(&s, user_id)
    }

    pub(crate) fn reclaim_if_idle(&self, ttl: Duration, now: Instant) -> bool {
        self.run(|tx| {
            if tx.s.destroyed || tx.s.frozen || !tx.s.members.is_empty() {
                return false;
            }
            let Some(since) = tx.s.empty_since else {
                return false;
            };
            let empty_for = now.saturating_duration_since(since);
            if empty_for < ttl {
                return false;
            }
            tracing::info!(
                "Reclaiming room {} after {} minutes with nobody in it",
                tx.room.id,
                empty_for.as_secs() / 60
            );
            tx.destroy_now()
        })
        .0
    }

    pub(crate) fn freeze(&self) {
        self.run(|tx| {
            tx.s.frozen = true;
            tx.s.timers.cancel_countdown();
            tx.s.timers.abort_phase_timers();
            if let Phase::Select(sel) = &mut tx.s.phase {
                sel.stop();
            }
            if let Phase::Ready(ready) = &mut tx.s.phase {
                ready.stop();
            }
        });
    }
}

pub(crate) fn can_control(s: &RoomInner, user_id: i32) -> bool {
    s.host == Some(user_id) || s.setting.admin_ids.contains(&user_id)
}

pub(crate) struct Tx<'a> {
    pub(crate) room: &'a Room,
    pub(crate) s: MutexGuard<'a, RoomInner>,
    destroy: bool,
    settle: Option<SettleJob>,
}

impl Tx<'_> {
    pub(crate) fn svc(&self) -> &Arc<Services> {
        &self.room.svc
    }

    pub(crate) fn schedule(&mut self, delay: Duration, kind: TimerKind, countdown: bool) {
        if self.s.destroyed || self.s.frozen {
            return;
        }
        let event = TimerEvent {
            epoch: self.s.timers.epoch,
            countdown: countdown.then_some(self.s.timers.countdown_gen),
            kind,
        };
        if delay.is_zero() {
            self.s.timers.immediate.push_back(event);
            return;
        }
        let weak = self.room.me.clone();
        let deadline = Instant::now() + delay;
        let handle = self.room.svc.runtime.spawn(async move {
            tokio::time::sleep_until(deadline).await;
            if let Some(room) = weak.upgrade() {
                room.fire(event);
            }
        });
        let list = if countdown {
            &mut self.s.timers.countdown
        } else {
            &mut self.s.timers.phase
        };
        list.retain(|h| !h.is_finished());
        list.push(handle.abort_handle());
    }

    fn drain_immediate(&mut self) {
        // Bounded: a zero-delay timer cannot schedule another without a phase change.
        for _ in 0..64 {
            let Some(event) = self.s.timers.immediate.pop_front() else {
                return;
            };
            self.on_timer(event);
        }
        self.s.timers.immediate.clear();
    }

    fn on_timer(&mut self, event: TimerEvent) {
        // A timer of an earlier phase or countdown does nothing.
        if self.s.destroyed || self.s.frozen || event.epoch != self.s.timers.epoch {
            return;
        }
        if let Some(generation) = event.countdown
            && generation != self.s.timers.countdown_gen
        {
            return;
        }
        match event.kind {
            TimerKind::SelectNotice(seconds) => self.select_notice(seconds),
            TimerKind::SelectFinish => self.select_finish(),
            TimerKind::ReadyNotice(seconds) => self.ready_notice(seconds),
            TimerKind::ReadyFinish => self.ready_finish(),
            TimerKind::ForceNotice => self.force_finish_notice(),
            TimerKind::ForceFinish => self.finish_round(),
            TimerKind::AllReadyStart(token) => self.ready_wait_over(token),
            TimerKind::ResendWaitForReady(id) => {
                if matches!(self.s.phase, Phase::Ready(_))
                    && let Some(player) = self.s.members.member(id)
                {
                    player.send(&ClientBoundPacket::ChangeState(GameState::WaitForReady));
                }
            }
        }
    }

    pub(crate) fn set_phase(&mut self, phase: Phase) {
        self.s.timers.new_phase();
        self.s.phase = phase;
    }

    pub(crate) fn cancel_countdown_timers(&mut self) {
        self.s.timers.cancel_countdown();
    }

    pub(crate) fn abort_phase_timers(&mut self) {
        self.s.timers.abort_phase_timers();
    }

    pub(crate) fn start_settlement_job(&mut self, job: SettleJob) {
        self.settle = Some(job);
    }

    pub(crate) fn destroy_now(&mut self) -> bool {
        if self.s.destroyed {
            return false;
        }
        self.s.destroyed = true;
        self.s.timers.new_phase();
        self.destroy = true;
        self.room.svc.mark_rooms_dirty();
        true
    }

    pub(crate) fn can_control(&self, user_id: i32) -> bool {
        can_control(&self.s, user_id)
    }

    pub(crate) fn join(
        &mut self,
        player: &Arc<Player>,
        monitor: bool,
        broadcast: bool,
    ) -> Result<(), GameError> {
        if self.s.destroyed {
            return Err(GameError::room_not_found());
        }
        if !monitor && self.s.members.players.len() >= self.s.setting.max_player.max(0) as usize {
            return Err(GameError::room_full());
        }
        if !monitor && self.s.setting.locked && !self.s.members.players.is_empty() {
            return Err(GameError::room_locked());
        }
        if self.s.members.is_member(player.id) {
            return Ok(());
        }
        if monitor {
            self.s.members.monitors.insert(player.id, player.clone());
        } else {
            let seq = self.s.members.next_seq;
            self.s.members.next_seq += 1;
            self.s.members.players.insert(
                player.id,
                Member {
                    player: player.clone(),
                    seq,
                },
            );
            if self.s.host.is_none() {
                self.s.host = Some(player.id);
            }
        }
        player.set_room(self.room.arc());
        self.s.empty_since = None;

        // Order expected by the client: OnJoinRoom, the JoinRoom message, then the chat line.
        if broadcast {
            self.s
                .members
                .broadcast(&ClientBoundPacket::OnJoinRoom(FullUserProfile::new(
                    player.id,
                    player.name.clone(),
                    monitor,
                )));
            self.s
                .members
                .broadcast(&ClientBoundPacket::Message(Message::JoinRoom {
                    user: player.id,
                    name: player.name.clone(),
                }));
            self.s
                .members
                .broadcast_chat(&text::welcome_to_room(&player.name, player.id, monitor));
        }
        self.handle_join(player);
        Ok(())
    }

    fn handle_join(&mut self, player: &Arc<Player>) {
        match self.s.phase {
            Phase::Select(_) => self.select_handle_join(player),
            Phase::Ready(_) => self.ready_handle_join(player),
            Phase::Playing(_) => player.send_chat(text::ROOM_IS_PLAYING),
        }
    }

    pub(crate) fn leave(&mut self, user_id: i32) -> bool {
        let removed = match self.s.members.players.remove(&user_id) {
            Some(member) => Some(member.player),
            None => self.s.members.monitors.remove(&user_id),
        };
        let Some(player) = removed else {
            return false;
        };
        player.set_room(None);

        let mut promoted = None;
        if self.s.host == Some(user_id) {
            self.s.host = self.s.members.earliest_player();
            promoted = self.s.host;
        }
        let mut should_destroy = false;
        if self.s.members.is_empty() {
            self.s.empty_since = Some(Instant::now());
            should_destroy = self.s.setting.auto_destroy && !self.s.frozen;
        }
        if let Some(promoted) = promoted.and_then(|id| self.s.members.player(id).cloned()) {
            promoted.send(&ClientBoundPacket::ChangeHost { host: true });
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::LeaveRoom {
                user: user_id,
                name: player.name.clone(),
            }));
        if !self.s.frozen && !self.s.destroyed {
            match self.s.phase {
                Phase::Select(_) => self.select_handle_leave(user_id),
                Phase::Ready(_) => self.ready_handle_leave(user_id),
                Phase::Playing(_) => self.playing_handle_leave(&player),
            }
        }
        if should_destroy {
            self.destroy_now();
        }
        true
    }

    pub(crate) fn reevaluate(&mut self) {
        if self.s.frozen || self.s.destroyed {
            return;
        }
        match &self.s.phase {
            Phase::Select(_) => {
                self.update_countdown_state();
                self.shorten_if_all_voted();
            }
            Phase::Ready(_) => self.ready_update_state(),
            Phase::Playing(play) => {
                if !play.settling && self.all_online_active_done() {
                    self.finish_round();
                }
            }
        }
    }

    pub(crate) fn room_info(&self) -> RoomInfo {
        RoomInfo::from_lists(
            self.room.id.clone(),
            self.s.phase.reported_state(),
            self.s.setting.live,
            self.s.setting.locked,
            self.s.setting.cycle,
            true,
            false,
            &self.s.members.player_profiles(),
            &self.s.members.monitor_profiles(),
        )
    }

    pub(crate) fn join_success(&self) -> ClientBoundPacket {
        ClientBoundPacket::JoinRoom(zphira_protocol::PacketResult::success(
            JoinRoomOk::from_lists(
                self.s.phase.reported_state(),
                &self.s.members.player_profiles(),
                &self.s.members.monitor_profiles(),
                self.s.setting.live,
            ),
        ))
    }

    pub(crate) fn fix_client_room_state(&mut self, player: &Arc<Player>) {
        if !self.s.members.is_member(player.id) {
            return;
        }
        // The client needs WaitForReady again 100 ms after joining a ready phase, and every
        // member is told it is host so the client shows the voting controls.
        if matches!(self.s.phase, Phase::Ready(_)) {
            self.schedule(
                WAIT_FOR_READY_RESEND,
                TimerKind::ResendWaitForReady(player.id),
                false,
            );
        }
        player.send(&ClientBoundPacket::ChangeHost { host: true });
    }

    fn require_member(&self, user_id: i32) -> OpResult<()> {
        if self.s.members.is_member(user_id) {
            Ok(())
        } else {
            Err(RoomOpError::NotMember)
        }
    }

    pub(crate) fn chat(&mut self, player: &Arc<Player>, message: &str) -> OpResult<ChatOutcome> {
        self.require_member(player.id)?;
        if is_clear_command(message) {
            if !self.can_control(player.id) {
                return Err(GameError::not_host().into());
            }
            let removed = self.clear(Some(player.id));
            return Ok(ChatOutcome::Cleared(removed));
        }
        if !self.s.setting.chat {
            return Err(GameError::chat_not_enabled().into());
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::Chat {
                user: player.id,
                content: message.to_owned(),
            }));
        Ok(ChatOutcome::Sent)
    }

    pub(crate) fn clear(&mut self, keep: Option<i32>) -> Vec<Arc<Player>> {
        let targets: Vec<Arc<Player>> = self
            .s
            .members
            .players
            .values()
            .map(|m| m.player.clone())
            .chain(self.s.members.monitors.values().cloned())
            .filter(|p| Some(p.id) != keep)
            .collect();
        for target in &targets {
            if target.is_online() {
                target.send_chat(text::ROOM_CLEARED);
                if let Some(conn) = target.conn() {
                    conn.close_as(ConnectState::Kick);
                }
            }
        }
        for target in &targets {
            self.leave(target.id);
        }
        targets
    }

    pub(crate) fn lock_room(&mut self, player: &Arc<Player>) -> OpResult<()> {
        self.require_member(player.id)?;
        // The lock button is the client's only way to show the vote board.
        if matches!(self.s.phase, Phase::Select(_)) {
            self.send_vote_board(player);
            Ok(())
        } else {
            Err(GameError::chart_pool_unavailable().into())
        }
    }

    pub(crate) fn cycle_room(&mut self, player: &Arc<Player>) -> OpResult<()> {
        self.require_member(player.id)?;
        if !self.can_control(player.id) {
            return Err(GameError::not_host().into());
        }
        self.s.setting.cycle = !self.s.setting.cycle;
        self.room.svc.mark_rooms_dirty();
        let cycle = self.s.setting.cycle;
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::CycleRoom { cycle }));
        Ok(())
    }

    pub(crate) fn select_chart(&mut self, player: &Arc<Player>, chart_id: i32) -> OpResult<()> {
        self.require_member(player.id)?;
        if matches!(self.s.phase, Phase::Select(_)) {
            Ok(self.vote(player, chart_id)?)
        } else {
            Err(GameError::invalid_state().into())
        }
    }

    pub(crate) fn require_start(&mut self, player: &Arc<Player>) -> OpResult<()> {
        self.require_member(player.id)?;
        if !self.can_control(player.id) {
            return Err(GameError::not_host().into());
        }
        match self.s.phase {
            Phase::Select(_) => Ok(self.select_require_start(player)?),
            Phase::Ready(_) | Phase::Playing(_) => Err(GameError::permission_denied().into()),
        }
    }

    pub(crate) fn ready(&mut self, player: &Arc<Player>) -> OpResult<()> {
        self.require_member(player.id)?;
        match self.s.phase {
            Phase::Select(_) => self.select_ready(player),
            Phase::Ready(_) => self.ready_ready(player),
            Phase::Playing(_) => return Err(GameError::invalid_state().into()),
        }
        Ok(())
    }

    pub(crate) fn cancel_ready(&mut self, player: &Arc<Player>) -> OpResult<()> {
        self.require_member(player.id)?;
        match self.s.phase {
            Phase::Select(_) => self.select_cancel_ready(player)?,
            Phase::Ready(_) => self.ready_cancel(player.id),
            Phase::Playing(_) => return Err(GameError::invalid_state().into()),
        }
        Ok(())
    }

    pub(crate) fn abort(&mut self, player: &Arc<Player>) -> OpResult<()> {
        self.require_member(player.id)?;
        match self.s.phase {
            // A late abort after the round settled is accepted silently.
            Phase::Select(_) => Ok(()),
            Phase::Ready(_) => Err(GameError::invalid_state().into()),
            Phase::Playing(_) => {
                self.playing_abort(player);
                Ok(())
            }
        }
    }

    pub(crate) fn relay_to_monitors(&mut self, player: &Arc<Player>, packet: ClientBoundPacket) {
        if let Phase::Playing(play) = &self.s.phase
            && play.active.contains(&player.id)
        {
            self.s.members.broadcast_to_monitors(&packet);
        }
    }

    pub(crate) fn suspend(&mut self, player: &Arc<Player>) -> SuspendOutcome {
        if self.s.members.is_monitor(player.id) {
            self.leave(player.id);
        }
        if !self.s.members.is_player(player.id) {
            return SuspendOutcome::NotInRoom;
        }
        if self.s.frozen || self.s.destroyed {
            return SuspendOutcome::Suspended;
        }
        match self.s.phase {
            Phase::Ready(_) => self.ready_cancel(player.id),
            Phase::Playing(_) => self.playing_abort(player),
            Phase::Select(_) => {}
        }
        self.reevaluate();
        SuspendOutcome::Suspended
    }
}

pub(crate) fn is_clear_command(message: &str) -> bool {
    message
        .trim_matches(|c: char| c <= ' ')
        .eq_ignore_ascii_case(text::CLEAR_COMMAND)
}
