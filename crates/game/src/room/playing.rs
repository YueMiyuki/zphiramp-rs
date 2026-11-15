use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use zphira_common::error::GameError;
use zphira_ledger::{PlayerResult, PointAward, RoundRecord};
use zphira_phira::{ChartInfo, GameRecord};
use zphira_protocol::{ClientBoundPacket, GameState, Message};

use super::{Phase, PhaseKind, Room, TimerKind, Tx, effective_force_finish_seconds};
use crate::player::Player;
use crate::services::Services;
use crate::text;
use crate::upstream::UpstreamError;
use crate::util::{java_double_compare, lock};

const FORCE_FINISH_NOTICE_SECONDS: i32 = 10;

pub(crate) const RECORD_TIME_TOLERANCE_MILLIS: i64 = 5 * 60 * 1000;

pub(crate) struct PlayState {
    pub(crate) chart: Arc<ChartInfo>,
    pub(crate) active: BTreeSet<i32>,
    pub(crate) done: BTreeSet<i32>,
    pub(crate) records: BTreeMap<i32, Arc<GameRecord>>,
    names: HashMap<i32, String>,
    pub(crate) started_at: i64,
    pub(crate) settling: bool,
    force_started: bool,
    rotation: Option<(i32, i32, bool)>,
}

impl PlayState {
    pub(crate) fn new(chart: Arc<ChartInfo>, active: BTreeSet<i32>, started_at: i64) -> Self {
        Self {
            chart,
            active,
            done: BTreeSet::new(),
            records: BTreeMap::new(),
            names: HashMap::new(),
            started_at,
            settling: false,
            force_started: false,
            rotation: None,
        }
    }
}

pub(crate) enum PlayedCheck {
    Ignore,
    Fetch { epoch: u64 },
}

pub(crate) type Recovery = (i32, Result<Option<GameRecord>, String>);

pub(crate) struct SettleJob {
    room: Arc<Room>,
    epoch: u64,
    chart_id: i32,
    started_at: i64,
    missing: Vec<i32>,
}

impl SettleJob {
    pub(crate) async fn run(self) {
        let svc = self.room.svc.clone();
        let _guard = svc.track_settlement();
        let recovered = recover_missing(&svc, &self.missing, self.chart_id, self.started_at).await;
        let epoch = self.epoch;
        self.room
            .run_spawn(|tx| tx.complete_settlement(epoch, recovered));
    }

    pub(crate) fn spawn(self) {
        let runtime = self.room.svc.runtime.clone();
        runtime.spawn(self.run());
    }
}

async fn recover_missing(
    svc: &Arc<Services>,
    missing: &[i32],
    chart_id: i32,
    started_at: i64,
) -> Vec<Recovery> {
    let limit = Arc::new(Semaphore::new(svc.options.recovery_concurrency.max(1)));
    let mut tasks = JoinSet::new();
    for &player_id in missing {
        let svc = svc.clone();
        let limit = limit.clone();
        tasks.spawn(async move {
            let _permit = limit.acquire_owned().await;
            let result = svc.upstream.recent_records(player_id).await;
            (player_id, result)
        });
    }
    let mut results: BTreeMap<i32, Result<Option<GameRecord>, String>> = BTreeMap::new();
    let deadline = tokio::time::sleep(svc.options.recovery_timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            next = tasks.join_next() => {
                let Some(joined) = next else { break };
                let Ok((player_id, result)) = joined else { continue };
                let picked = result
                    .map(|records| pick_recovered(svc, &records, chart_id, started_at))
                    .map_err(|e: UpstreamError| e.to_string());
                results.insert(player_id, picked);
            }
            () = &mut deadline => {
                tasks.abort_all();
                break;
            }
        }
    }
    missing
        .iter()
        .map(|&id| {
            let result = results
                .remove(&id)
                .unwrap_or_else(|| Err("Record recovery timed out".to_owned()));
            (id, result)
        })
        .collect()
}

fn pick_recovered(
    svc: &Services,
    records: &[GameRecord],
    chart_id: i32,
    started_at: i64,
) -> Option<GameRecord> {
    let accepted = lock(&svc.accepted);
    records
        .iter()
        .find(|candidate| {
            candidate.chart == chart_id
                && candidate
                    .time
                    .as_ref()
                    .and_then(|t| t.to_epoch_milli())
                    .is_some_and(|t| t >= started_at - RECORD_TIME_TOLERANCE_MILLIS)
                && !accepted.contains(candidate.id)
        })
        .cloned()
}

fn compare_records(a: &GameRecord, b: &GameRecord) -> Ordering {
    b.score
        .cmp(&a.score)
        .then_with(|| java_double_compare(f64::from(b.accuracy), f64::from(a.accuracy)))
        .then_with(|| java_double_compare(f64::from(a.std), f64::from(b.std)))
}

fn points_by_rank(rank: i32) -> i32 {
    match rank {
        1 => 100,
        2 => 75,
        3 => 50,
        4 => 25,
        _ => 10,
    }
}

impl Tx<'_> {
    fn play(&mut self) -> Option<&mut PlayState> {
        match &mut self.s.phase {
            Phase::Playing(p) => Some(p),
            _ => None,
        }
    }

    fn play_ref(&self) -> Option<&PlayState> {
        match &self.s.phase {
            Phase::Playing(p) => Some(p),
            _ => None,
        }
    }

    pub(crate) fn start_force_finish_countdown(&mut self) {
        let configured = self.s.setting.force_finish_seconds;
        let Some(play) = self.play() else {
            return;
        };
        if play.force_started {
            return;
        }
        play.force_started = true;
        let chart = play.chart.clone();
        let seconds = effective_force_finish_seconds(configured, chart.duration_seconds());
        tracing::info!(
            "Room {} round on '{}' ({}s chart) will force finish after {}s (configured {})",
            self.room.id,
            chart.name_or_null(),
            chart
                .duration_seconds()
                .map_or_else(|| "null".to_owned(), |d| d.to_string()),
            seconds,
            configured
        );
        if seconds > FORCE_FINISH_NOTICE_SECONDS {
            self.schedule(
                Duration::from_secs((seconds - FORCE_FINISH_NOTICE_SECONDS) as u64),
                TimerKind::ForceNotice,
                false,
            );
        }
        self.schedule(
            Duration::from_secs(seconds.max(1) as u64),
            TimerKind::ForceFinish,
            false,
        );
    }

    pub(crate) fn force_finish_notice(&mut self) {
        if self.play_ref().is_some_and(|p| !p.settling) {
            self.s
                .members
                .broadcast_chat(&text::force_finish_notice(FORCE_FINISH_NOTICE_SECONDS));
        }
    }

    pub(crate) fn playing_handle_leave(&mut self, player: &Arc<Player>) {
        if self
            .play_ref()
            .is_some_and(|p| p.active.contains(&player.id))
        {
            self.finish_player(player, false);
        }
    }

    pub(crate) fn playing_abort(&mut self, player: &Arc<Player>) {
        let Some(play) = self.play_ref() else {
            return;
        };
        if !play.active.contains(&player.id) || play.done.contains(&player.id) {
            return;
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::Abort {
                user: player.id,
            }));
        self.finish_player(player, true);
    }

    pub(crate) fn played_check(&self, player: &Arc<Player>) -> Result<PlayedCheck, GameError> {
        if !self.s.members.is_member(player.id) {
            return Ok(PlayedCheck::Ignore);
        }
        match &self.s.phase {
            // A late score after the round settled is accepted silently.
            Phase::Select(_) => Ok(PlayedCheck::Ignore),
            Phase::Ready(_) => Err(GameError::invalid_state()),
            Phase::Playing(play) => {
                if !play.active.contains(&player.id) || play.done.contains(&player.id) {
                    Ok(PlayedCheck::Ignore)
                } else {
                    Ok(PlayedCheck::Fetch {
                        epoch: self.s.timers.epoch(),
                    })
                }
            }
        }
    }

    pub(crate) fn played_apply(
        &mut self,
        player: &Arc<Player>,
        epoch: u64,
        fetched: Result<Arc<GameRecord>, UpstreamError>,
    ) -> Result<(), GameError> {
        if self.s.timers.epoch() != epoch {
            return Ok(());
        }
        let Some(play) = self.play_ref() else {
            return Ok(());
        };
        if !play.active.contains(&player.id) || play.done.contains(&player.id) {
            return Ok(());
        }
        let chart_id = play.chart.id;
        let started_at = play.started_at;
        let record = match fetched {
            Ok(record) => record,
            Err(_) => {
                player.send_chat(text::SUBMIT_NETWORK_FAILED_SELF);
                self.s
                    .members
                    .broadcast_chat(&text::submit_network_failed_room(&player.name));
                self.s
                    .members
                    .broadcast(&ClientBoundPacket::Message(Message::Abort {
                        user: player.id,
                    }));
                self.finish_player(player, true);
                return Err(GameError::record_submit_failed());
            }
        };
        if !self.record_is_valid(player.id, chart_id, started_at, &record) {
            tracing::info!(
                "Round in {}: {} submitted record {} that does not belong to this round",
                self.room.id,
                player.name,
                record.id
            );
            player.send_chat(text::SUBMIT_INVALID_SELF);
            self.s
                .members
                .broadcast_chat(&text::submit_invalid_room(&player.name));
            self.s
                .members
                .broadcast(&ClientBoundPacket::Message(Message::Abort {
                    user: player.id,
                }));
            self.finish_player(player, true);
            return Err(GameError::record_submit_failed());
        }
        lock(&self.svc().accepted).insert(record.id);
        if let Some(play) = self.play() {
            play.names
                .entry(player.id)
                .or_insert_with(|| player.name.clone());
            play.records.insert(player.id, record.clone());
        }
        tracing::info!(
            "Round in {}: {} submitted record {} (score {})",
            self.room.id,
            player.name,
            record.id,
            record.score
        );
        self.s
            .members
            .broadcast_chat(&text::played_record(player.id, &player.name, &record));
        self.start_force_finish_countdown();
        self.finish_player(player, true);
        Ok(())
    }

    fn record_is_valid(
        &self,
        player_id: i32,
        chart_id: i32,
        started_at: i64,
        record: &GameRecord,
    ) -> bool {
        record.player == player_id
            && record.chart == chart_id
            && record
                .time
                .as_ref()
                .and_then(|t| t.to_epoch_milli())
                .is_some_and(|t| t >= started_at - RECORD_TIME_TOLERANCE_MILLIS)
            && !lock(&self.svc().accepted).contains(record.id)
    }

    fn finish_player(&mut self, player: &Arc<Player>, update_client: bool) {
        let Some(play) = self.play() else {
            return;
        };
        play.done.insert(player.id);
        let chart_id = play.chart.id;
        if update_client && player.is_online() {
            // Back to the chart, and told it is host so the client keeps its voting controls.
            player.send(&ClientBoundPacket::ChangeHost { host: true });
            player.send(&ClientBoundPacket::ChangeState(GameState::SelectChart(
                Some(chart_id),
            )));
        }
        if self.all_online_active_done() {
            self.finish_round();
        }
    }

    pub(crate) fn all_online_active_done(&self) -> bool {
        let Some(play) = self.play_ref() else {
            return false;
        };
        play.active.iter().all(|id| {
            play.done.contains(id) || !self.s.members.player(*id).is_some_and(|p| p.is_online())
        })
    }

    pub(crate) fn finish_round(&mut self) {
        if self.s.frozen {
            return;
        }
        let interval = self.s.setting.refresh_interval_rounds;
        let epoch = self.s.timers.epoch();
        if self.play_ref().is_none_or(|p| p.settling) {
            return;
        }
        let previous = self.s.pool.current_pool_id();
        let quota = self.s.pool.resolve_rounds_per_stay(interval);
        let rotated = self.s.pool.finish_playing_round(interval);
        self.svc().mark_rooms_dirty();
        self.abort_phase_timers();
        let Some(play) = self.play() else {
            return;
        };
        play.settling = true;
        play.rotation = Some((previous, quota, rotated));
        let missing: Vec<i32> = play
            .active
            .iter()
            .copied()
            .filter(|id| !play.records.contains_key(id))
            .collect();
        let chart_id = play.chart.id;
        let started_at = play.started_at;
        if missing.is_empty() {
            self.complete_settlement(epoch, Vec::new());
            return;
        }
        let Some(room) = self.room.arc() else {
            self.complete_settlement(epoch, Vec::new());
            return;
        };
        self.start_settlement_job(SettleJob {
            room,
            epoch,
            chart_id,
            started_at,
            missing,
        });
    }

    pub(crate) fn complete_settlement(&mut self, epoch: u64, recovered: Vec<Recovery>) {
        // Runs even if the room was destroyed meanwhile, so nobody's points are lost.
        if self.s.timers.epoch() != epoch || self.play_ref().is_none_or(|p| !p.settling) {
            return;
        }
        for (player_id, result) in recovered {
            match result {
                Ok(Some(record)) => {
                    if self
                        .play_ref()
                        .is_some_and(|p| p.records.contains_key(&player_id))
                    {
                        continue;
                    }
                    let name = self
                        .s
                        .members
                        .player(player_id)
                        .map_or_else(|| format!("#{player_id}"), |p| p.name.clone());
                    lock(&self.svc().accepted).insert(record.id);
                    let (id, score) = (record.id, record.score);
                    if let Some(play) = self.play() {
                        play.records.insert(player_id, Arc::new(record));
                        play.names.entry(player_id).or_insert_with(|| name.clone());
                    }
                    tracing::info!(
                        "Recovered record {} for {} (score {}) from Phira",
                        id,
                        name,
                        score
                    );
                    self.s
                        .members
                        .broadcast_chat(&text::record_recovered(&name, score));
                }
                Ok(None) => {}
                Err(message) => tracing::warn!(
                    "Could not recover record for player {}: {}",
                    player_id,
                    message
                ),
            }
        }
        let Some(play) = self.play_ref() else {
            return;
        };
        tracing::info!(
            "Round in {} ended: {} of {} player(s) submitted a record",
            self.room.id,
            play.records.len(),
            play.active.len()
        );
        let (previous, quota, rotated) = play.rotation.unwrap_or((0, 0, false));
        self.broadcast_ranking();

        let next = super::SelectState::new(&self.s.pool, &self.svc().catalog);
        // Every client is told it is host again so it shows the voting controls.
        let host = ClientBoundPacket::ChangeHost { host: true };
        for player in self.s.members.online_players() {
            player.send(&host);
        }
        self.s
            .members
            .broadcast(&ClientBoundPacket::Message(Message::GameEnd));
        self.set_phase(Phase::Select(next));
        self.s
            .members
            .broadcast(&ClientBoundPacket::ChangeState(GameState::SelectChart(
                None,
            )));
        self.announce_pool_progress(previous, quota, rotated);
        if !self.s.destroyed && !self.s.frozen {
            self.update_countdown_state();
        }
    }

    fn broadcast_ranking(&mut self) {
        let Some(play) = self.play_ref() else {
            return;
        };
        let room_id = self.room.id.clone();
        let chart_id = play.chart.id;
        let chart_name = play.chart.name.clone();
        let started_at = play.started_at;
        if play.records.is_empty() {
            self.record_round(room_id, chart_id, chart_name, started_at, Vec::new());
            return;
        }
        let mut ranking: Vec<(i32, Arc<GameRecord>)> = play
            .records
            .iter()
            .map(|(id, r)| (*id, r.clone()))
            .collect();
        ranking.sort_by(|(ida, a), (idb, b)| compare_records(a, b).then(ida.cmp(idb)));

        let mut ranked = Vec::with_capacity(ranking.len());
        let mut rank = 0;
        let mut previous: Option<&Arc<GameRecord>> = None;
        for (i, (player_id, record)) in ranking.iter().enumerate() {
            if previous.is_none_or(|p| compare_records(record, p) != Ordering::Equal) {
                rank = i as i32 + 1;
            }
            let name = play
                .names
                .get(player_id)
                .cloned()
                .or_else(|| self.s.members.player(*player_id).map(|p| p.name.clone()))
                .unwrap_or_else(|| format!("#{player_id}"));
            ranked.push((*player_id, name, record.clone(), rank, points_by_rank(rank)));
            previous = Some(record);
        }

        let awards: Vec<PointAward> = ranked
            .iter()
            .map(|(id, name, _, _, gained)| PointAward {
                player_id: *id,
                name: Some(name.clone()),
                gained: *gained,
            })
            .collect();
        let totals: HashMap<i32, i32> = self
            .svc()
            .points
            .add_points_batch(&awards)
            .into_iter()
            .collect();

        let mut results = Vec::with_capacity(ranked.len());
        self.s.members.broadcast_chat(text::SEPARATOR);
        self.s.members.broadcast_chat(text::RANKING_HEADER);
        for (player_id, name, record, rank, gained) in &ranked {
            let total = totals.get(player_id).copied().unwrap_or(0);
            self.s
                .members
                .broadcast_chat(&text::ranking_line(*rank, name, record, *gained, total));
            results.push(PlayerResult {
                player_id: *player_id,
                player_name: name.clone(),
                rank: *rank,
                score: record.score,
                accuracy: f64::from(record.accuracy),
                std: f64::from(record.std),
                gained_points: *gained,
                total_points: total,
            });
        }
        self.s.members.broadcast_chat(text::SEPARATOR);
        self.record_round(room_id, chart_id, chart_name, started_at, results);
    }

    fn record_round(
        &self,
        room_id: String,
        chart_id: i32,
        chart_name: Option<String>,
        started_at: i64,
        results: Vec<PlayerResult>,
    ) {
        let svc = self.svc();
        let finished_at = svc.now_millis();
        svc.rounds.append(RoundRecord {
            id: svc.rounds.next_id(&room_id, finished_at),
            room_id,
            chart_id,
            chart_name,
            started_at,
            finished_at,
            results,
        });
    }

    fn announce_pool_progress(&mut self, previous: i32, quota: i32, rotated: bool) {
        let current = self.s.pool.current_pool_id();
        if !rotated {
            let finished = self.s.pool.finished_rounds_since_refresh();
            self.s
                .members
                .broadcast_chat(&text::pool_progress(current, finished, quota));
            self.broadcast_vote_board_hint();
            return;
        }
        // Checked on the list, not the ids: a manual switch to the current pool keeps the id.
        if self.s.pool.pool_ids().len() == 1 {
            self.s
                .members
                .broadcast_chat(&text::pool_single(current, quota));
        } else {
            self.s
                .members
                .broadcast_chat(&text::pool_rotated(previous, quota, current));
        }
        self.broadcast_vote_board();
    }

    pub(crate) fn force_finish(&mut self) -> bool {
        if self.s.phase.kind() != PhaseKind::Playing {
            return false;
        }
        self.finish_round();
        true
    }
}
