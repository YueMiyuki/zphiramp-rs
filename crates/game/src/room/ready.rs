use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use zphira_phira::ChartInfo;
use zphira_protocol::{ClientBoundPacket, GameState, Message};

use super::{Phase, PhaseKind, PlayState, TimerKind, Tx};
use crate::player::Player;
use crate::text;

const NOTICE_SECONDS: [i32; 7] = [60, 30, 10, 5, 3, 2, 1];

pub(crate) struct ReadyState {
    pub(crate) chart: Arc<ChartInfo>,
    pub(crate) ready: BTreeSet<i32>,
    running: bool,
    pending_start: Option<u64>,
    next_token: u64,
}

impl ReadyState {
    pub(crate) fn new(chart: Arc<ChartInfo>, initially_ready: BTreeSet<i32>) -> Self {
        Self {
            chart,
            ready: initially_ready,
            running: false,
            pending_start: None,
            next_token: 0,
        }
    }

    pub(crate) fn stop(&mut self) {
        self.running = false;
    }
}

impl Tx<'_> {
    fn rdy(&mut self) -> Option<&mut ReadyState> {
        match &mut self.s.phase {
            Phase::Ready(r) => Some(r),
            _ => None,
        }
    }

    fn rdy_ref(&self) -> Option<&ReadyState> {
        match &self.s.phase {
            Phase::Ready(r) => Some(r),
            _ => None,
        }
    }

    pub(crate) fn ready_handle_join(&mut self, player: &Arc<Player>) {
        if let Some(r) = self.rdy_ref() {
            let chart_id = r.chart.id;
            // The client needs the chart before the ready state.
            player.send(&ClientBoundPacket::ChangeState(GameState::SelectChart(
                Some(chart_id),
            )));
            player.send(&ClientBoundPacket::ChangeState(GameState::WaitForReady));
        }
        self.ready_update_state();
    }

    pub(crate) fn ready_handle_leave(&mut self, user_id: i32) {
        if let Some(r) = self.rdy() {
            r.ready.remove(&user_id);
        }
        self.ready_update_state();
    }

    pub(crate) fn ready_ready(&mut self, player: &Arc<Player>) {
        if let Some(r) = self.rdy() {
            r.ready.insert(player.id);
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::Ready {
                user: player.id,
            }));
        self.ready_update_state();
    }

    pub(crate) fn ready_cancel(&mut self, user_id: i32) {
        if let Some(r) = self.rdy() {
            r.ready.remove(&user_id);
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::CancelReady {
                user: user_id,
            }));
        self.ready_evaluate_start();
    }

    pub(crate) fn start_ready_countdown(&mut self) {
        let seconds = self.s.setting.ready_countdown_seconds;
        let Some(r) = self.rdy() else {
            return;
        };
        if r.running {
            return;
        }
        r.running = true;
        self.s
            .members
            .broadcast_chat(&text::ready_phase_started(seconds));
        for notice in NOTICE_SECONDS {
            if notice <= seconds {
                self.schedule(
                    Duration::from_secs((seconds - notice) as u64),
                    TimerKind::ReadyNotice(notice),
                    true,
                );
            }
        }
        self.schedule(
            Duration::from_secs(seconds.max(1) as u64),
            TimerKind::ReadyFinish,
            true,
        );
    }

    pub(crate) fn ready_notice(&mut self, seconds: i32) {
        if self.rdy_ref().is_some_and(|r| r.running) {
            self.s
                .members
                .broadcast_chat(&text::ready_countdown_notice(seconds));
        }
    }

    pub(crate) fn ready_update_state(&mut self) {
        if self.s.phase.kind() != PhaseKind::Ready {
            return;
        }
        if self.s.members.online_player_count() == 0 {
            self.cancel_ready_countdown();
            self.s.members.broadcast_chat(text::READY_NOBODY_ONLINE);
            self.transition_to_select();
            self.update_countdown_state();
            return;
        }
        self.ready_evaluate_start();
    }

    pub(crate) fn ready_evaluate_start(&mut self) {
        if self.s.phase.kind() != PhaseKind::Ready {
            return;
        }
        let all_ready = self.all_online_players_ready();
        let pending = self.rdy_ref().is_some_and(|r| r.pending_start.is_some());
        if pending {
            if !all_ready {
                if let Some(r) = self.rdy() {
                    r.pending_start = None;
                }
                self.s.members.broadcast_chat(text::ALL_READY_CALLED_OFF);
            }
            return;
        }
        if !all_ready || !self.s.setting.start_when_all_ready {
            return;
        }
        let delay = self.s.setting.all_ready_delay_seconds;
        if delay <= 0 {
            self.cancel_ready_countdown();
            self.enter_playing();
            return;
        }
        let Some(r) = self.rdy() else {
            return;
        };
        r.next_token += 1;
        let token = r.next_token;
        r.pending_start = Some(token);
        self.s.members.broadcast_chat(&text::all_ready_start(delay));
        self.schedule(
            Duration::from_secs(delay as u64),
            TimerKind::AllReadyStart(token),
            false,
        );
    }

    pub(crate) fn ready_wait_over(&mut self, token: u64) {
        let Some(r) = self.rdy() else {
            return;
        };
        if r.pending_start != Some(token) {
            return;
        }
        r.pending_start = None;
        if !self.all_online_players_ready() {
            self.s.members.broadcast_chat(text::ALL_READY_CALLED_OFF);
            return;
        }
        self.cancel_ready_countdown();
        self.enter_playing();
    }

    fn all_online_players_ready(&self) -> bool {
        let Some(r) = self.rdy_ref() else {
            return false;
        };
        // Spectators do not count.
        let mut online = self.s.members.online_players().peekable();
        if online.peek().is_none() {
            return false;
        }
        online.all(|p| r.ready.contains(&p.id))
    }

    pub(crate) fn ready_finish(&mut self) {
        let Some(r) = self.rdy() else {
            return;
        };
        if !r.running {
            return;
        }
        r.running = false;
        self.enter_playing();
    }

    fn cancel_ready_countdown(&mut self) {
        if let Some(r) = self.rdy() {
            r.running = false;
        }
        self.cancel_countdown_timers();
    }

    fn enter_playing(&mut self) {
        let Some(r) = self.rdy_ref() else {
            return;
        };
        let chart = r.chart.clone();
        let ready = r.ready.clone();
        let select_chart = ClientBoundPacket::ChangeState(GameState::SelectChart(Some(chart.id)));
        let host = ClientBoundPacket::ChangeHost { host: true };

        let mut active = BTreeSet::new();
        for player in self.s.members.online_players() {
            if ready.contains(&player.id) {
                active.insert(player.id);
            } else {
                // Sitting out: back to the chart, and told it is host to keep its controls.
                player.send(&host);
                player.send(&select_chart);
            }
        }
        // Every online spectator watches, ready or not.
        let watchers: Vec<Arc<Player>> = self.s.members.online_monitors().cloned().collect();

        if active.is_empty() {
            self.s.members.broadcast_chat(text::READY_NOBODY_READY);
            self.transition_to_select();
            self.update_countdown_state();
            return;
        }

        let started_at = self.svc().now_millis();
        let active_players: Vec<Arc<Player>> = active
            .iter()
            .filter_map(|id| self.s.members.player(*id).cloned())
            .collect();
        self.set_phase(Phase::Playing(PlayState::new(chart, active, started_at)));
        self.start_force_finish_countdown();
        let playing = ClientBoundPacket::ChangeState(GameState::Playing);
        let start = ClientBoundPacket::Message(Message::StartPlaying);
        for player in &active_players {
            player.send(&playing);
        }
        for monitor in &watchers {
            monitor.send(&playing);
        }
        for player in &active_players {
            player.send(&start);
        }
        for monitor in &watchers {
            monitor.send(&start);
        }
    }
}
