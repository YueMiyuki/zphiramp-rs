use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zphira_common::{fs, gson};

use super::setting::{DEFAULT_ALL_READY_DELAY_SECONDS, MAX_ALL_READY_DELAY_SECONDS};
use super::{RoomSetting, Rotation};

pub const ROOMS_FILE: &str = "rooms.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedRoom {
    pub room_id: Option<String>,
    pub pool_ids: Option<Vec<i32>>,
    pub setting: Option<SavedSetting>,
    pub current_pool_id: Option<i32>,
    pub pending_pool_id: Option<i32>,
    pub finished_rounds: Option<i32>,
}

impl SavedRoom {
    pub fn rotation(&self) -> Rotation {
        Rotation {
            current_pool_id: self.current_pool_id,
            pending_pool_id: self.pending_pool_id,
            finished_rounds: self.finished_rounds.unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedSetting {
    pub auto_destroy: bool,
    pub host: bool,
    pub max_player: i32,
    pub locked: bool,
    pub cycle: bool,
    pub live: bool,
    pub chat: bool,
    pub min_player: i32,
    pub select_chart_countdown_seconds: i32,
    pub ready_countdown_seconds: i32,
    pub force_finish_seconds: i32,
    pub refresh_interval_rounds: i32,
    pub admin_ids: Option<Vec<i32>>,
    pub cancel_ready_in_vote: Option<bool>,
    pub start_when_all_ready: Option<bool>,
    pub all_ready_delay_seconds: Option<i32>,
}

impl SavedSetting {
    pub fn of(setting: &RoomSetting) -> Self {
        Self {
            auto_destroy: setting.auto_destroy,
            host: setting.host,
            max_player: setting.max_player,
            locked: setting.locked,
            cycle: setting.cycle,
            live: setting.live,
            chat: setting.chat,
            min_player: setting.min_player,
            select_chart_countdown_seconds: setting.select_chart_countdown_seconds,
            ready_countdown_seconds: setting.ready_countdown_seconds,
            force_finish_seconds: setting.force_finish_seconds,
            refresh_interval_rounds: setting.refresh_interval_rounds,
            admin_ids: Some(setting.admin_ids.clone()),
            cancel_ready_in_vote: Some(setting.cancel_ready_in_vote),
            start_when_all_ready: Some(setting.start_when_all_ready),
            all_ready_delay_seconds: Some(setting.all_ready_delay_seconds),
        }
    }

    pub fn to_setting(&self) -> RoomSetting {
        RoomSetting {
            auto_destroy: self.auto_destroy,
            host: self.host,
            max_player: self.max_player,
            locked: self.locked,
            cycle: self.cycle,
            live: self.live,
            chat: self.chat,
            min_player: self.min_player,
            select_chart_countdown_seconds: self.select_chart_countdown_seconds,
            ready_countdown_seconds: self.ready_countdown_seconds,
            force_finish_seconds: self.force_finish_seconds,
            refresh_interval_rounds: self.refresh_interval_rounds,
            admin_ids: RoomSetting::normalize_builder_admins(
                self.admin_ids.as_deref().unwrap_or(&[]),
            ),
            cancel_ready_in_vote: self.cancel_ready_in_vote.unwrap_or(true),
            start_when_all_ready: self.start_when_all_ready.unwrap_or(true),
            all_ready_delay_seconds: self
                .all_ready_delay_seconds
                .unwrap_or(DEFAULT_ALL_READY_DELAY_SECONDS)
                .clamp(0, MAX_ALL_READY_DELAY_SECONDS),
        }
    }
}

#[derive(Debug)]
pub enum Loaded {
    Missing,
    Unreadable(String),
    Rooms(Vec<SavedRoom>),
}

pub fn read(path: &Path) -> Loaded {
    let _ = fs::remove_stale_temp_files(path);
    let text = match fs::read_to_string_if_exists(path) {
        Ok(Some(text)) => text,
        Ok(None) => return Loaded::Missing,
        Err(e) => return Loaded::Unreadable(e.to_string()),
    };
    match gson::from_str_opt::<Vec<Option<SavedRoom>>>(&text) {
        Ok(Some(rooms)) => Loaded::Rooms(rooms.into_iter().flatten().collect()),
        Ok(None) => Loaded::Rooms(Vec::new()),
        Err(e) => Loaded::Unreadable(e.to_string()),
    }
}

pub fn write(path: &Path, rooms: &[SavedRoom]) -> Result<(), gson::GsonError> {
    fs::write_atomic_with(path, &fs::AtomicWriteOptions::new(), |w| {
        gson::to_writer_pretty(w, rooms)
    })
}

pub fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(ROOMS_FILE)
}
