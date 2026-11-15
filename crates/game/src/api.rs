use std::sync::Arc;

use serde::Serialize;
use zphira_catalog::PoolSnapshot;
use zphira_common::error::GameError;
use zphira_phira::ChartInfo;
use zphira_protocol::{ClientBoundPacket, Message};

use crate::context::GameContext;
use crate::room::store::{self, SavedRoom, SavedSetting};
use crate::room::{Phase, Room, RoomChartPool, RoomSetting, Tx, effective_force_finish_seconds};
use crate::text;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Serializes to the HTTP API's room object.
pub struct RoomView {
    pub room_id: String,
    pub state: &'static str,
    pub live: bool,
    pub locked: bool,
    pub cycle: bool,
    pub host: Option<i32>,
    #[serde(serialize_with = "serialize_chart")]
    pub chart: Option<Arc<ChartInfo>>,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub config: RoomConfigView,
    pub effective_force_finish: i32,
    pub pool: PoolStatusView,
    pub players: Vec<MemberView>,
    pub monitors: Vec<MemberView>,
    #[serde(skip)]
    pub online_players: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomConfigView {
    pub min_player: i32,
    pub max_player: i32,
    pub select_countdown: i32,
    pub ready_countdown: i32,
    pub force_finish: i32,
    pub interval: i32,
    pub admin_ids: Vec<i32>,
    pub chat_enable: bool,
    pub cancel_ready_in_vote: bool,
    pub start_when_all_ready: bool,
    pub all_ready_delay: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolStatusView {
    pub current_pool: PoolView,
    pub pools: Vec<PoolView>,
    pub pending_pool_id: Option<i32>,
    pub favorite_id: Option<i32>,
    pub finished_rounds_since_refresh: i32,
    pub refresh_interval_rounds: i32,
    pub effective_rounds_per_stay: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolView {
    pub id: i32,
    pub favorite_id: Option<i32>,
    pub default: bool,
    pub chart_ids: Vec<i32>,
    pub category: Option<&'static str>,
    pub size_limit: Option<i32>,
    pub rounds_per_stay: Option<i32>,
    pub order: i32,
    pub submission_open: bool,
}

impl From<&PoolSnapshot> for PoolView {
    fn from(p: &PoolSnapshot) -> Self {
        Self {
            id: p.id,
            favorite_id: p.favorite_id,
            default: p.default_flag,
            chart_ids: p.chart_ids.clone(),
            category: Some(p.category.name()),
            size_limit: p.size_limit,
            rounds_per_stay: p.rounds_per_stay,
            order: p.order,
            submission_open: p.submission_open,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberView {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerView {
    pub id: i32,
    pub name: String,
    pub room: Option<String>,
}

#[derive(Debug, Clone, Copy)]
/// The HTTP API's chart object.
pub struct ChartApiView<'a>(pub &'a ChartInfo);

impl Serialize for ChartApiView<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let c = self.0;
        let mut map = serializer.serialize_map(Some(23))?;
        map.serialize_entry("id", &c.id)?;
        map.serialize_entry("name", &c.name)?;
        map.serialize_entry("level", &c.level)?;
        map.serialize_entry("difficulty", &c.difficulty)?;
        map.serialize_entry("charter", &c.charter)?;
        map.serialize_entry("composer", &c.composer)?;
        map.serialize_entry("illustrator", &c.illustrator)?;
        map.serialize_entry("description", &c.description)?;
        map.serialize_entry("ranked", &c.ranked)?;
        map.serialize_entry("reviewed", &c.reviewed)?;
        map.serialize_entry("stable", &c.stable)?;
        map.serialize_entry("stableRequest", &c.stable_request)?;
        map.serialize_entry("illustration", &c.illustration)?;
        map.serialize_entry("preview", &c.preview)?;
        map.serialize_entry("file", &c.file)?;
        map.serialize_entry("uploader", &c.uploader)?;
        let tags: &[String] = c.tags.as_deref().unwrap_or(&[]);
        map.serialize_entry("tags", tags)?;
        map.serialize_entry("rating", &c.rating)?;
        map.serialize_entry("ratingCount", &c.rating_count)?;
        map.serialize_entry("created", &c.created)?;
        map.serialize_entry("updated", &c.updated)?;
        map.serialize_entry("chartUpdated", &c.chart_updated)?;
        map.serialize_entry("durationSeconds", &c.duration_seconds())?;
        map.end()
    }
}

fn serialize_chart<S: serde::Serializer>(
    chart: &Option<Arc<ChartInfo>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match chart {
        Some(c) => ChartApiView(c).serialize(serializer),
        None => serializer.serialize_none(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// `Resolve` skips unknown and empty pools; `Exact` refuses unknown ones.
pub enum RoomPools {
    Default,
    Resolve(Vec<i32>),
    Exact(Vec<i32>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateRoomError {
    InvalidId,
    AlreadyExists,
    NoDefaultPools,
    NoUsablePools,
    PoolNotFound(i32),
    AllPoolsEmpty,
    LimitReached(GameError),
}

impl CreateRoomError {
    pub fn http_message(&self) -> String {
        match self {
            Self::InvalidId => text::ROOM_ID_RULE.to_owned(),
            Self::AlreadyExists => "房间已存在".to_owned(),
            Self::NoDefaultPools => "当前没有默认启用的谱池".to_owned(),
            Self::NoUsablePools | Self::PoolNotFound(_) | Self::AllPoolsEmpty => {
                "指定的谱池不存在或均为空池".to_owned()
            }
            Self::LimitReached(e) => e.message_key().to_owned(),
        }
    }

    pub fn console_message(&self, room_id: &str) -> String {
        match self {
            Self::InvalidId => {
                "Room id must be letters, digits, - or _ with length <= 20".to_owned()
            }
            Self::AlreadyExists => format!("Room already exists: {room_id}"),
            Self::NoDefaultPools => "No default pools available".to_owned(),
            Self::NoUsablePools | Self::AllPoolsEmpty => crate::room::EMPTY_POOL_LIST.to_owned(),
            Self::PoolNotFound(id) => format!("Pool not found: {id}"),
            Self::LimitReached(e) => e.message_key().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Validated as a whole before anything is applied.
pub struct RoomUpdate {
    pub live: Option<bool>,
    pub locked: Option<bool>,
    pub admin_ids: Option<Vec<i32>>,
    pub chat_enable: Option<bool>,
    pub min_player: Option<i32>,
    pub max_player: Option<i32>,
    pub select_countdown: Option<i32>,
    pub ready_countdown: Option<i32>,
    pub force_finish: Option<i32>,
    pub interval: Option<i32>,
    pub cancel_ready_in_vote: Option<bool>,
    pub start_when_all_ready: Option<bool>,
    pub all_ready_delay: Option<i32>,
    pub reject_min_above_max: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateRoomError {
    RoomNotFound,
    AdminIdNotPositive,
    MinPlayerNotPositive,
    MaxPlayerNotPositive,
    SelectCountdownTooShort,
    ReadyCountdownNotPositive,
    ForceFinishNotPositive,
    IntervalNotPositive,
    AllReadyDelayOutOfRange,
    MinAboveMax,
}

impl UpdateRoomError {
    pub fn http_message(&self) -> &'static str {
        match self {
            Self::RoomNotFound => "欲请求的内容不存在",
            Self::AdminIdNotPositive => "管理员 ID 必须是正整数",
            Self::MinPlayerNotPositive => "minPlayer 必须大于 0",
            Self::MaxPlayerNotPositive => "maxPlayer 必须大于 0",
            Self::SelectCountdownTooShort => "selectCountdown 必须大于等于 10",
            Self::ReadyCountdownNotPositive => "readyCountdown 必须大于 0",
            Self::ForceFinishNotPositive => "forceFinish 必须大于 0",
            Self::IntervalNotPositive => "interval 必须大于 0",
            Self::AllReadyDelayOutOfRange => "allReadyDelay 必须在 0 到 300 之间",
            Self::MinAboveMax => "minPlayer 不能大于 maxPlayer",
        }
    }

    pub fn console_message(&self, room_id: &str) -> String {
        match self {
            Self::RoomNotFound => format!("Room not found: {room_id}"),
            Self::AdminIdNotPositive => "adminIds must be positive".to_owned(),
            Self::MinPlayerNotPositive => "minPlayer must be positive".to_owned(),
            Self::MaxPlayerNotPositive => "maxPlayer must be positive".to_owned(),
            Self::SelectCountdownTooShort => "selectCountdown must be at least 10".to_owned(),
            Self::ReadyCountdownNotPositive => "readyCountdown must be positive".to_owned(),
            Self::ForceFinishNotPositive => "forceFinish must be positive".to_owned(),
            Self::IntervalNotPositive => "interval must be positive".to_owned(),
            Self::AllReadyDelayOutOfRange => "allReadyDelay must be between 0 and 300".to_owned(),
            Self::MinAboveMax => "minPlayer must not exceed maxPlayer".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomNotFound;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndRoundError {
    RoomNotFound,
    NotPlaying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveRoomError {
    RoomNotFound,
    NotEmpty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchPoolError {
    RoomNotFound,
    NotInRoom(String),
}

impl GameContext {
    fn room_handle(&self, room_id: &str) -> Option<Arc<Room>> {
        self.inner.rooms.find(room_id)
    }

    pub(crate) fn register_room(
        &self,
        room_id: &str,
        setting: RoomSetting,
        pool: RoomChartPool,
    ) -> Result<Arc<Room>, GameError> {
        let ids = pool.pool_ids();
        let svc = self.svc().clone();
        let registry = Arc::downgrade(&self.inner.rooms);
        let max_rooms = svc.args.max_rooms as usize;
        let room = self.inner.rooms.create(room_id, max_rooms, || {
            Room::new(room_id.to_owned(), svc, registry, setting, pool)
        })?;
        if ids.len() == 1 {
            tracing::warn!(
                "Room {} has a single chart pool ({}), so it will never rotate",
                room_id,
                ids[0]
            );
        }
        Ok(room)
    }

    pub fn is_valid_room_id(&self, room_id: &str) -> bool {
        crate::lobby::is_valid_room_id(room_id)
    }

    pub fn room_exists(&self, room_id: &str) -> bool {
        self.room_handle(room_id).is_some()
    }

    pub fn room_ids(&self) -> Vec<String> {
        self.inner
            .rooms
            .all()
            .iter()
            .map(|r| r.id.clone())
            .collect()
    }

    pub fn rooms(&self) -> Vec<RoomView> {
        self.inner
            .rooms
            .all()
            .iter()
            .map(|room| room.run(|tx| tx.view()).0)
            .collect()
    }

    pub fn room(&self, room_id: &str) -> Option<RoomView> {
        self.room_handle(room_id)
            .map(|room| room.run(|tx| tx.view()).0)
    }

    pub fn room_pool(&self, room_id: &str) -> Option<PoolStatusView> {
        self.room_handle(room_id)
            .map(|room| room.run(|tx| tx.pool_view()).0)
    }

    /// Returns the room's pool ids.
    pub fn create_room(
        &self,
        room_id: &str,
        pools: RoomPools,
    ) -> Result<Vec<i32>, CreateRoomError> {
        if !crate::lobby::is_valid_room_id(room_id) {
            return Err(CreateRoomError::InvalidId);
        }
        if self.room_exists(room_id) {
            return Err(CreateRoomError::AlreadyExists);
        }
        let catalog = self.svc().catalog.clone();
        let snapshots: Vec<Arc<PoolSnapshot>> = match pools {
            RoomPools::Default => {
                let pools = catalog.get_default_pools();
                if pools.is_empty() {
                    return Err(CreateRoomError::NoDefaultPools);
                }
                pools
            }
            RoomPools::Resolve(ids) => {
                let pools = catalog.resolve_pools(&ids);
                if pools.is_empty() {
                    return Err(CreateRoomError::NoUsablePools);
                }
                pools
            }
            RoomPools::Exact(ids) => {
                let mut out: Vec<Arc<PoolSnapshot>> = Vec::new();
                for id in ids {
                    let pool = catalog
                        .find_pool(id)
                        .ok_or(CreateRoomError::PoolNotFound(id))?;
                    if !out.iter().any(|p| p.id == pool.id) {
                        out.push(pool);
                    }
                }
                out
            }
        };
        let pool =
            RoomChartPool::new(catalog, &snapshots).map_err(|_| CreateRoomError::AllPoolsEmpty)?;
        let ids = pool.pool_ids();
        self.register_room(room_id, RoomSetting::operator_created(), pool)
            .map_err(|e| {
                if e.message_key() == GameError::room_already_exists().message_key() {
                    CreateRoomError::AlreadyExists
                } else {
                    CreateRoomError::LimitReached(e)
                }
            })?;
        self.svc().mark_rooms_dirty();
        Ok(ids)
    }

    pub fn update_room(&self, room_id: &str, update: &RoomUpdate) -> Result<(), UpdateRoomError> {
        let room = self
            .room_handle(room_id)
            .ok_or(UpdateRoomError::RoomNotFound)?;
        room.run(|tx| tx.apply_update(update)).0?;
        self.svc().mark_rooms_dirty();
        Ok(())
    }

    /// Returns once the round has settled.
    pub async fn end_round(&self, room_id: &str) -> Result<(), EndRoundError> {
        let room = self
            .room_handle(room_id)
            .ok_or(EndRoundError::RoomNotFound)?;
        let (playing, settle) = room.run(|tx| tx.force_finish());
        if let Some(job) = settle {
            job.run().await;
        }
        if playing {
            Ok(())
        } else {
            Err(EndRoundError::NotPlaying)
        }
    }

    /// Returns the number of players in the room, online or not, monitors not counted.
    pub fn say_to_room(&self, room_id: &str, message: &str) -> Option<usize> {
        let room = self.room_handle(room_id)?;
        Some(
            room.run(|tx| {
                tx.s.members.broadcast_chat(message);
                tx.s.members.players.len()
            })
            .0,
        )
    }

    /// Returns the number of online players.
    pub fn say_to_all(&self, message: &str) -> usize {
        let frame = crate::net::conn::chat_frame(text::SYSTEM_SENDER, message);
        for player in self.online_player_handles() {
            player.send_frame(&frame);
        }
        self.online_count()
    }

    pub fn switch_room_pool(&self, room_id: &str, pool_id: i32) -> Result<(), SwitchPoolError> {
        let room = self
            .room_handle(room_id)
            .ok_or(SwitchPoolError::RoomNotFound)?;
        room.run(|tx| tx.s.pool.switch_pool(pool_id))
            .0
            .map_err(SwitchPoolError::NotInRoom)?;
        self.svc().mark_rooms_dirty();
        Ok(())
    }

    /// Applies to the current stay only and is not saved.
    pub fn set_room_favorite(
        &self,
        room_id: &str,
        favorite: Option<i32>,
    ) -> Result<(), RoomNotFound> {
        let room = self.room_handle(room_id).ok_or(RoomNotFound)?;
        room.run(|tx| tx.s.pool.set_favorite(favorite));
        Ok(())
    }

    pub fn remove_room(&self, room_id: &str) -> Result<(), RemoveRoomError> {
        let room = self
            .room_handle(room_id)
            .ok_or(RemoveRoomError::RoomNotFound)?;
        let removed = room
            .run(|tx| {
                if !tx.s.members.is_empty() {
                    return false;
                }
                tx.destroy_now();
                true
            })
            .0;
        if !removed {
            return Err(RemoveRoomError::NotEmpty);
        }
        self.svc().mark_rooms_dirty();
        Ok(())
    }

    /// Removes and disconnects everyone in the room; returns how many.
    pub async fn clear_room(&self, room_id: &str) -> Result<usize, RoomNotFound> {
        let room = self.room_handle(room_id).ok_or(RoomNotFound)?;
        let (removed, settle) = room.run(|tx| tx.clear(None));
        self.drop_sessions(&removed);
        if let Some(job) = settle {
            job.run().await;
        }
        Ok(removed.len())
    }

    pub fn online_players(&self) -> Vec<PlayerView> {
        self.online_player_handles()
            .into_iter()
            .map(|p| PlayerView {
                id: p.id,
                name: p.name.clone(),
                room: p.room().map(|r| r.id.clone()),
            })
            .collect()
    }

    pub fn player_room(&self, user_id: i32) -> Option<String> {
        self.find_player(user_id)
            .and_then(|p| p.room())
            .map(|r| r.id.clone())
    }

    /// Membership only; an admin bypass is the caller's.
    pub fn is_in_room(&self, user_id: i32, room_id: &str) -> bool {
        self.player_room(user_id).as_deref() == Some(room_id)
    }

    pub fn can_control_room(&self, user_id: i32, room_id: &str) -> bool {
        self.room_handle(room_id)
            .is_some_and(|room| room.can_control(user_id))
    }

    pub fn kick_player(&self, user_id: i32) -> bool {
        let Some(player) = self.find_player(user_id) else {
            return false;
        };
        self.kick(&player);
        self.drop_sessions(std::slice::from_ref(&player));
        true
    }

    pub fn mark_rooms_dirty(&self) {
        self.svc().mark_rooms_dirty();
    }

    pub fn flush_rooms_blocking(&self) -> bool {
        let _guard = crate::util::lock(&self.inner.store_lock);
        if !self
            .svc()
            .rooms_dirty
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            return false;
        }
        let saved: Vec<SavedRoom> = self
            .inner
            .rooms
            .all()
            .iter()
            .map(|room| room.run(|tx| tx.saved()).0)
            .collect();
        match store::write(&self.inner.rooms_path, &saved) {
            Ok(()) => true,
            Err(e) => {
                self.svc().mark_rooms_dirty();
                tracing::error!(error = %e, "Failed to save rooms");
                false
            }
        }
    }

    /// Writes rooms.json if anything changed; returns whether it wrote.
    pub async fn flush_rooms(&self) -> bool {
        let ctx = self.clone();
        tokio::task::spawn_blocking(move || ctx.flush_rooms_blocking())
            .await
            .unwrap_or(false)
    }

    pub async fn restore_rooms(&self) -> usize {
        let path = self.inner.rooms_path.clone();
        let loaded = tokio::task::spawn_blocking(move || store::read(&path))
            .await
            .unwrap_or_else(|e| store::Loaded::Unreadable(e.to_string()));
        let rooms = match loaded {
            store::Loaded::Missing => return 0,
            store::Loaded::Unreadable(message) => {
                tracing::warn!(
                    "Room store is unreadable ({}), starting with no rooms",
                    message
                );
                return 0;
            }
            store::Loaded::Rooms(rooms) => rooms,
        };
        if rooms.is_empty() {
            return 0;
        }
        let mut restored = 0;
        for saved in rooms {
            if self.restore_room(&saved) {
                restored += 1;
            }
        }
        tracing::info!(
            "Restored {} room(s) from {}",
            restored,
            self.inner.rooms_path.display()
        );
        restored
    }

    fn restore_room(&self, saved: &SavedRoom) -> bool {
        let room_id = saved.room_id.as_deref().unwrap_or("null");
        let catalog = self.svc().catalog.clone();
        let ids = saved.pool_ids.clone().unwrap_or_default();
        let Ok(mut pool) = RoomChartPool::restore(catalog, &ids) else {
            tracing::warn!("Skipping room {}: its pools no longer exist", room_id);
            return false;
        };
        let Some(id) = saved.room_id.as_deref() else {
            tracing::warn!("Skipping room {}: {}", room_id, "the room id is missing");
            return false;
        };
        let Some(setting) = &saved.setting else {
            tracing::warn!("Skipping room {}: {}", room_id, "the setting is missing");
            return false;
        };
        pool.restore_rotation(Some(saved.rotation()));
        match self.register_room(id, setting.to_setting(), pool) {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("Skipping room {}: {}", room_id, e.message_key());
                false
            }
        }
    }
}

impl Tx<'_> {
    pub(crate) fn view(&self) -> RoomView {
        let s = &self.s;
        let member = |p: &Arc<crate::player::Player>| MemberView {
            id: p.id,
            name: p.name.clone(),
        };
        let effective = match &s.phase {
            Phase::Playing(play) => effective_force_finish_seconds(
                s.setting.force_finish_seconds,
                play.chart.duration_seconds(),
            ),
            _ => s.setting.force_finish_seconds,
        };
        RoomView {
            room_id: self.room.id.clone(),
            state: s.phase.name(),
            live: s.setting.live,
            locked: s.setting.locked,
            cycle: s.setting.cycle,
            host: s.host,
            chart: s.phase.chart().cloned(),
            kind: "local",
            config: RoomConfigView {
                min_player: s.setting.min_player,
                max_player: s.setting.max_player,
                select_countdown: s.setting.select_chart_countdown_seconds,
                ready_countdown: s.setting.ready_countdown_seconds,
                force_finish: s.setting.force_finish_seconds,
                interval: s.setting.refresh_interval_rounds,
                admin_ids: s.setting.admin_ids.clone(),
                chat_enable: s.setting.chat,
                cancel_ready_in_vote: s.setting.cancel_ready_in_vote,
                start_when_all_ready: s.setting.start_when_all_ready,
                all_ready_delay: s.setting.all_ready_delay_seconds,
            },
            effective_force_finish: effective,
            pool: self.pool_view(),
            players: s
                .members
                .players
                .values()
                .map(|m| member(&m.player))
                .collect(),
            monitors: s.members.monitors.values().map(member).collect(),
            online_players: s.members.online_player_count(),
        }
    }

    pub(crate) fn pool_view(&self) -> PoolStatusView {
        let pool = &self.s.pool;
        let interval = self.s.setting.refresh_interval_rounds;
        let current = pool.current_pool_snapshot();
        PoolStatusView {
            current_pool: PoolView::from(current.as_ref()),
            pools: pool
                .pools()
                .iter()
                .map(|p| PoolView::from(p.as_ref()))
                .collect(),
            pending_pool_id: pool.pending_pool_id(),
            favorite_id: current.favorite_id,
            finished_rounds_since_refresh: pool.finished_rounds_since_refresh(),
            refresh_interval_rounds: interval,
            effective_rounds_per_stay: pool.resolve_rounds_per_stay(interval),
        }
    }

    pub(crate) fn saved(&self) -> SavedRoom {
        let rotation = self.s.pool.rotation_state();
        SavedRoom {
            room_id: Some(self.room.id.clone()),
            pool_ids: Some(self.s.pool.pool_ids()),
            setting: Some(SavedSetting::of(&self.s.setting)),
            current_pool_id: rotation.current_pool_id,
            pending_pool_id: rotation.pending_pool_id,
            finished_rounds: Some(rotation.finished_rounds),
        }
    }

    fn apply_update(&mut self, u: &RoomUpdate) -> Result<(), UpdateRoomError> {
        let admins = match &u.admin_ids {
            Some(ids) => {
                let mut clean: Vec<i32> = Vec::new();
                for &id in ids {
                    if id <= 0 {
                        return Err(UpdateRoomError::AdminIdNotPositive);
                    }
                    if !clean.contains(&id) {
                        clean.push(id);
                    }
                }
                Some(clean)
            }
            None => None,
        };
        let check = |value: Option<i32>, ok: fn(i32) -> bool, error| match value {
            Some(v) if !ok(v) => Err(error),
            _ => Ok(()),
        };
        check(
            u.min_player,
            |v| v > 0,
            UpdateRoomError::MinPlayerNotPositive,
        )?;
        check(
            u.max_player,
            |v| v > 0,
            UpdateRoomError::MaxPlayerNotPositive,
        )?;
        check(
            u.select_countdown,
            |v| v >= 10,
            UpdateRoomError::SelectCountdownTooShort,
        )?;
        check(
            u.ready_countdown,
            |v| v > 0,
            UpdateRoomError::ReadyCountdownNotPositive,
        )?;
        check(
            u.force_finish,
            |v| v > 0,
            UpdateRoomError::ForceFinishNotPositive,
        )?;
        check(u.interval, |v| v > 0, UpdateRoomError::IntervalNotPositive)?;
        check(
            u.all_ready_delay,
            |v| (0..=crate::room::MAX_ALL_READY_DELAY_SECONDS).contains(&v),
            UpdateRoomError::AllReadyDelayOutOfRange,
        )?;
        let setting = &mut self.s.setting;
        if u.reject_min_above_max {
            let min = u.min_player.unwrap_or(setting.min_player);
            let max = u.max_player.unwrap_or(setting.max_player);
            if min > max {
                return Err(UpdateRoomError::MinAboveMax);
            }
        }
        if let Some(live) = u.live {
            setting.live = live;
        }
        if let Some(locked) = u.locked {
            setting.locked = locked;
        }
        if let Some(admins) = admins {
            setting.admin_ids = admins;
        }
        if let Some(chat) = u.chat_enable {
            setting.chat = chat;
        }
        if let Some(v) = u.min_player {
            setting.min_player = v;
        }
        if let Some(v) = u.max_player {
            setting.max_player = v;
        }
        if let Some(v) = u.select_countdown {
            setting.select_chart_countdown_seconds = v;
        }
        if let Some(v) = u.ready_countdown {
            setting.ready_countdown_seconds = v;
        }
        if let Some(v) = u.force_finish {
            setting.force_finish_seconds = v;
        }
        if let Some(v) = u.interval {
            setting.refresh_interval_rounds = v;
        }
        if let Some(v) = u.cancel_ready_in_vote {
            setting.cancel_ready_in_vote = v;
        }
        if let Some(v) = u.start_when_all_ready {
            setting.start_when_all_ready = v;
        }
        if let Some(v) = u.all_ready_delay {
            setting.all_ready_delay_seconds = v;
        }
        Ok(())
    }
}

pub fn system_chat(message: &str) -> ClientBoundPacket {
    ClientBoundPacket::Message(Message::Chat {
        user: text::SYSTEM_SENDER,
        content: message.to_owned(),
    })
}
