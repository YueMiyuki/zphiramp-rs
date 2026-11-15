use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use zphira_catalog::{ChartPool, PoolSnapshot};
use zphira_common::error::GameError;
use zphira_phira::ChartInfo;
use zphira_protocol::{ClientBoundPacket, GameState, Message};

use super::{Phase, PhaseKind, ReadyState, RoomChartPool, TimerKind, Tx};
use crate::player::Player;
use crate::text;

const NOTICE_SECONDS: [i32; 10] = [150, 120, 90, 60, 30, 10, 5, 3, 2, 1];
pub(crate) const ALL_VOTED_COUNTDOWN_SECONDS: i32 = 10;

pub(crate) struct SelectState {
    pub(crate) pool_info: Arc<PoolSnapshot>,
    pub(crate) charts: Vec<Arc<ChartInfo>>,
    pub(crate) votes: BTreeMap<i32, i32>,
    pub(crate) running: bool,
    pub(crate) deadline: Option<Instant>,
    pub(crate) ready_intents: BTreeSet<i32>,
    pub(crate) locked: Option<Arc<ChartInfo>>,
}

impl SelectState {
    pub(crate) fn new(pool: &RoomChartPool, catalog: &ChartPool) -> Self {
        let pool_info = pool.current_pool_snapshot();
        let charts = pool_info
            .chart_ids
            .iter()
            .filter_map(|&id| catalog.chart_info(id))
            .collect();
        Self {
            pool_info,
            charts,
            votes: BTreeMap::new(),
            running: false,
            deadline: None,
            ready_intents: BTreeSet::new(),
            locked: None,
        }
    }

    pub(crate) fn stop(&mut self) {
        self.running = false;
    }

    fn count(&self, chart_id: i32) -> usize {
        self.votes.values().filter(|&&v| v == chart_id).count()
    }

    fn board_lines(&self) -> Vec<String> {
        self.charts
            .iter()
            .map(|chart| text::vote_board_line(self.count(chart.id), chart))
            .collect()
    }
}

impl Tx<'_> {
    fn sel(&mut self) -> Option<&mut SelectState> {
        match &mut self.s.phase {
            Phase::Select(sel) => Some(sel),
            _ => None,
        }
    }

    fn sel_ref(&self) -> Option<&SelectState> {
        match &self.s.phase {
            Phase::Select(sel) => Some(sel),
            _ => None,
        }
    }

    pub(crate) fn transition_to_select(&mut self) {
        let sel = SelectState::new(&self.s.pool, &self.svc().catalog);
        self.set_phase(Phase::Select(sel));
        self.s
            .members
            .broadcast(&ClientBoundPacket::ChangeState(GameState::SelectChart(
                None,
            )));
    }

    pub(crate) fn select_handle_join(&mut self, player: &Arc<Player>) {
        self.send_vote_board(player);
        self.update_countdown_state();
    }

    pub(crate) fn select_handle_leave(&mut self, user_id: i32) {
        if let Some(sel) = self.sel() {
            sel.votes.remove(&user_id);
            sel.ready_intents.remove(&user_id);
        }
        self.broadcast_vote_board_hint();
        self.update_countdown_state();
        self.shorten_if_all_voted();
    }

    pub(crate) fn select_require_start(&mut self, player: &Arc<Player>) -> Result<(), GameError> {
        if self.s.members.online_player_count() == 0 {
            return Err(GameError::permission_denied());
        }
        self.stop_countdown();
        let locked = self.sel_ref().and_then(|s| s.locked.clone());
        let Some(chart) = locked.or_else(|| self.select_winning_chart()) else {
            return Err(GameError::invalid_state());
        };
        self.probe_duration(&chart);
        self.s
            .members
            .broadcast_chat(&text::early_start(&player.name, &chart));
        self.enter_ready(chart);
        Ok(())
    }

    pub(crate) fn select_ready(&mut self, player: &Arc<Player>) {
        if let Some(sel) = self.sel() {
            sel.ready_intents.insert(player.id);
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::Ready {
                user: player.id,
            }));
        self.update_countdown_state();
    }

    pub(crate) fn select_cancel_ready(&mut self, player: &Arc<Player>) -> Result<(), GameError> {
        if !self.s.setting.cancel_ready_in_vote {
            return Err(GameError::invalid_state());
        }
        if let Some(sel) = self.sel() {
            sel.ready_intents.remove(&player.id);
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::CancelReady {
                user: player.id,
            }));
        Ok(())
    }

    pub(crate) fn vote(&mut self, player: &Arc<Player>, chart_id: i32) -> Result<(), GameError> {
        if self.s.members.is_monitor(player.id) {
            tracing::info!(
                "Vote rejected in {}: {} is a monitor",
                self.room.id,
                player.name
            );
            return Err(GameError::permission_denied());
        }
        let Some(sel) = self.sel_ref() else {
            return Err(GameError::invalid_state());
        };
        if let Some(locked) = &sel.locked {
            tracing::info!(
                "Vote rejected in {}: chart already locked to {}",
                self.room.id,
                text::chart_label(locked)
            );
            player.send_chat(text::VOTE_LOCKED);
            return Err(GameError::invalid_state());
        }
        let Some(chart) = sel.charts.iter().find(|c| c.id == chart_id).cloned() else {
            tracing::info!(
                "Vote rejected in {}: chart {} is not in the current pool ({} charts)",
                self.room.id,
                chart_id,
                sel.charts.len()
            );
            return Err(GameError::chart_not_found());
        };
        if let Some(sel) = self.sel() {
            sel.votes.insert(player.id, chart_id);
        }
        tracing::info!(
            "Vote in {}: {} (#{}) -> {}",
            self.room.id,
            player.name,
            player.id,
            text::chart_label(&chart)
        );
        self.s
            .members
            .broadcast_chat(&text::voted(&player.name, &chart));
        self.broadcast_leading_chart();
        self.broadcast_vote_board_hint();
        self.shorten_if_all_voted();
        Ok(())
    }

    fn broadcast_leading_chart(&mut self) {
        let Some(sel) = self.sel_ref() else {
            return;
        };
        let mut leading: Option<(&Arc<ChartInfo>, usize)> = None;
        for chart in &sel.charts {
            let count = sel.count(chart.id);
            if count == 0 {
                continue;
            }
            // Ties keep the first chart in pool order.
            if leading.is_none_or(|(_, best)| count > best) {
                leading = Some((chart, count));
            }
        }
        let Some((chart, _)) = leading else {
            return;
        };
        let first_voter = sel
            .votes
            .iter()
            .filter(|(_, v)| **v == chart.id)
            .map(|(k, _)| *k)
            .min()
            .unwrap_or(text::SYSTEM_SENDER);
        // Without a selected chart the client refuses the start button locally.
        let message = ClientBoundPacket::Message(Message::SelectChart {
            user: first_voter,
            name: chart.name_or_null().to_owned(),
            id: chart.id,
        });
        let state = ClientBoundPacket::ChangeState(GameState::SelectChart(Some(chart.id)));
        let frames = [
            crate::net::conn::frame_of(&message),
            crate::net::conn::frame_of(&state),
        ];
        for player in self
            .s
            .members
            .players
            .values()
            .map(|m| &m.player)
            .chain(self.s.members.monitors.values())
        {
            for frame in &frames {
                player.send_frame(frame);
            }
        }
    }

    pub(crate) fn broadcast_vote_board(&mut self) {
        let Some(sel) = self.sel_ref() else {
            return;
        };
        let mut lines = vec![
            text::SEPARATOR.to_owned(),
            text::pool_header(&self.room.id, sel.pool_info.id),
        ];
        if let Some(favorite) = sel.pool_info.favorite_id {
            lines.push(text::favorite_line(favorite));
            lines.push(text::FAVORITE_IMPORT_HINT.to_owned());
        }
        lines.push(text::VOTE_INSTRUCTIONS.to_owned());
        lines.extend(sel.board_lines());
        lines.push(text::SEPARATOR.to_owned());
        let frames: Vec<_> = lines
            .iter()
            .map(|l| crate::net::conn::chat_frame(text::SYSTEM_SENDER, l))
            .collect();
        for player in self
            .s
            .members
            .players
            .values()
            .map(|m| &m.player)
            .chain(self.s.members.monitors.values())
        {
            for frame in &frames {
                player.send_frame(frame);
            }
        }
    }

    pub(crate) fn send_vote_board(&mut self, player: &Arc<Player>) {
        let summary = self.svc().points.summary(player.id, &player.name);
        let Some(sel) = self.sel_ref() else {
            return;
        };
        player.send_chat(text::SEPARATOR);
        player.send_chat(&text::points_line(summary.points, summary.rank));
        player.send_chat(&text::pool_header(&self.room.id, sel.pool_info.id));
        if let Some(favorite) = sel.pool_info.favorite_id {
            player.send_chat(&text::favorite_line(favorite));
            player.send_chat(text::FAVORITE_IMPORT_HINT);
        }
        player.send_chat(text::VOTE_INSTRUCTIONS);
        for line in sel.board_lines() {
            player.send_chat(&line);
        }
        player.send_chat(text::SEPARATOR);
    }

    pub(crate) fn broadcast_vote_board_hint(&mut self) {
        self.s.members.broadcast_chat(text::VOTE_BOARD_HINT);
    }

    pub(crate) fn update_countdown_state(&mut self) {
        if self.s.phase.kind() != PhaseKind::Select {
            return;
        }
        if self.s.members.online_player_count() >= self.s.setting.min_player.max(0) as usize {
            self.start_countdown();
        } else {
            self.cancel_countdown();
        }
    }

    fn start_countdown(&mut self) {
        let all_voted = self.have_all_online_players_voted();
        let countdown_seconds = self.s.setting.select_chart_countdown_seconds;
        let Some(sel) = self.sel() else {
            return;
        };
        if sel.running {
            return;
        }
        sel.running = true;
        let seconds = if all_voted {
            ALL_VOTED_COUNTDOWN_SECONDS.min(countdown_seconds)
        } else {
            countdown_seconds
        };
        self.s
            .members
            .broadcast_chat(&text::countdown_started(seconds));
        self.schedule_countdown(seconds);
    }

    pub(crate) fn shorten_if_all_voted(&mut self) {
        let all_voted = self.have_all_online_players_voted();
        let Some(sel) = self.sel_ref() else {
            return;
        };
        if !sel.running || sel.locked.is_some() || !all_voted {
            return;
        }
        let Some(deadline) = sel.deadline else {
            return;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining <= Duration::from_secs(ALL_VOTED_COUNTDOWN_SECONDS as u64) {
            return;
        }
        self.cancel_countdown_timers();
        self.s
            .members
            .broadcast_chat(&text::countdown_shortened(ALL_VOTED_COUNTDOWN_SECONDS));
        self.schedule_countdown(ALL_VOTED_COUNTDOWN_SECONDS);
    }

    fn schedule_countdown(&mut self, seconds: i32) {
        // At least one second, so a hand-edited rooms.json cannot make the room loop.
        let length = seconds.max(1);
        if let Some(sel) = self.sel() {
            sel.deadline = Some(Instant::now() + Duration::from_secs(length as u64));
        }
        for notice in NOTICE_SECONDS {
            if notice < seconds {
                self.schedule(
                    Duration::from_secs((seconds - notice) as u64),
                    TimerKind::SelectNotice(notice),
                    true,
                );
            }
        }
        self.schedule(
            Duration::from_secs(length as u64),
            TimerKind::SelectFinish,
            true,
        );
    }

    fn have_all_online_players_voted(&self) -> bool {
        let Some(sel) = self.sel_ref() else {
            return false;
        };
        let mut online = self.s.members.online_players().peekable();
        if online.peek().is_none() {
            return false;
        }
        online.all(|p| sel.votes.contains_key(&p.id))
    }

    fn stop_countdown(&mut self) {
        if let Some(sel) = self.sel() {
            sel.running = false;
        }
        self.cancel_countdown_timers();
    }

    fn cancel_countdown(&mut self) {
        if !self.sel_ref().is_some_and(|s| s.running) {
            return;
        }
        self.stop_countdown();
        self.s.members.broadcast_chat(text::COUNTDOWN_CANCELLED);
        self.release_lock();
    }

    fn release_lock(&mut self) {
        if self.sel().and_then(|s| s.locked.take()).is_some() {
            self.s.members.broadcast_chat(text::VOTE_UNLOCKED);
        }
    }

    pub(crate) fn select_notice(&mut self, seconds: i32) {
        let running = self.sel_ref().is_some_and(|s| s.running);
        if !running
            || self.s.members.online_player_count() < self.s.setting.min_player.max(0) as usize
        {
            return;
        }
        self.s
            .members
            .broadcast_chat(&text::vote_countdown_notice(seconds));
        if seconds == 1 {
            let Some(chart) = self.select_winning_chart() else {
                return;
            };
            if let Some(sel) = self.sel() {
                sel.locked = Some(chart.clone());
            }
            self.s.members.broadcast_chat(text::VOTE_LOCKED);
            self.s.members.broadcast_chat(text::SEPARATOR);
            self.s
                .members
                .broadcast(&ClientBoundPacket::ChangeState(GameState::SelectChart(
                    Some(chart.id),
                )));
        }
    }

    pub(crate) fn select_finish(&mut self) {
        let Some(sel) = self.sel() else {
            return;
        };
        if !sel.running {
            return;
        }
        sel.running = false;
        if self.s.members.online_player_count() < self.s.setting.min_player.max(0) as usize {
            self.s.members.broadcast_chat(text::ROUND_CANCELLED_SHORT);
            self.release_lock();
            return;
        }
        let locked = self.sel_ref().and_then(|s| s.locked.clone());
        let Some(chart) = locked.or_else(|| self.select_winning_chart()) else {
            tracing::warn!("Room {} has no chart to vote on", self.room.id);
            return;
        };
        self.probe_duration(&chart);
        self.s.members.broadcast_chat(&text::vote_finished(&chart));
        self.enter_ready(chart);
    }

    fn select_winning_chart(&mut self) -> Option<Arc<ChartInfo>> {
        let sel = self.sel_ref()?;
        let highest = sel.charts.iter().map(|c| sel.count(c.id)).max()?;
        let candidates: Vec<Arc<ChartInfo>> = sel
            .charts
            .iter()
            .filter(|c| sel.count(c.id) == highest)
            .cloned()
            .collect();
        if candidates.len() > 1 {
            self.s.members.broadcast_chat(text::TIE_BREAK);
        }
        let pick = self.svc().random_below(candidates.len());
        candidates.into_iter().nth(pick)
    }

    pub(crate) fn probe_duration(&self, chart: &Arc<ChartInfo>) {
        if chart.duration_seconds().is_none()
            && let Some(index) = &self.svc().index
        {
            index.probe_duration_async(chart);
        }
    }

    fn enter_ready(&mut self, chart: Arc<ChartInfo>) {
        let intents = self
            .sel_ref()
            .map(|s| s.ready_intents.clone())
            .unwrap_or_default();
        let carried: Vec<i32> = intents
            .iter()
            .copied()
            .filter(|id| self.s.members.is_player(*id))
            .collect();
        self.set_phase(Phase::Ready(ReadyState::new(chart, intents)));
        for id in carried {
            self.s
                .members
                .broadcast(&ClientBoundPacket::Message(Message::Ready { user: id }));
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::ChangeState(GameState::WaitForReady));
        self.start_ready_countdown();
        self.ready_evaluate_start();
    }
}
