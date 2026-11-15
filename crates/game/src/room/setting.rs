#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomSetting {
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
    pub admin_ids: Vec<i32>,
    pub cancel_ready_in_vote: bool,
    pub start_when_all_ready: bool,
    pub all_ready_delay_seconds: i32,
}

impl Default for RoomSetting {
    fn default() -> Self {
        Self {
            auto_destroy: true,
            host: true,
            max_player: 1000,
            locked: false,
            cycle: false,
            live: false,
            chat: true,
            min_player: 2,
            select_chart_countdown_seconds: 150,
            ready_countdown_seconds: 60,
            force_finish_seconds: 3600,
            refresh_interval_rounds: 5,
            admin_ids: Vec::new(),
            cancel_ready_in_vote: true,
            start_when_all_ready: true,
            all_ready_delay_seconds: DEFAULT_ALL_READY_DELAY_SECONDS,
        }
    }
}

impl RoomSetting {
    pub fn player_created() -> Self {
        Self {
            host: false,
            cycle: false,
            chat: true,
            auto_destroy: true,
            ..Self::default()
        }
    }

    pub fn operator_created() -> Self {
        Self {
            auto_destroy: false,
            ..Self::player_created()
        }
    }

    pub(crate) fn normalize_admins(ids: &[i32]) -> Vec<i32> {
        let mut out = ids.to_vec();
        out.sort_unstable();
        out.dedup();
        out
    }
}

pub const DEFAULT_ALL_READY_DELAY_SECONDS: i32 = 5;
pub const MAX_ALL_READY_DELAY_SECONDS: i32 = 300;

pub const MIN_FORCE_FINISH_SECONDS: i32 = 3600;
pub const FORCE_FINISH_GRACE_SECONDS: i32 = 120;

pub fn effective_force_finish_seconds(configured: i32, chart_duration: Option<i32>) -> i32 {
    let floor = if configured <= 0 {
        MIN_FORCE_FINISH_SECONDS
    } else {
        configured.max(MIN_FORCE_FINISH_SECONDS)
    };
    match chart_duration {
        Some(duration) if duration > 0 => {
            floor.max(duration.saturating_add(FORCE_FINISH_GRACE_SECONDS))
        }
        _ => floor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_finish_floor() {
        assert_eq!(effective_force_finish_seconds(120, None), 3600);
        assert_eq!(effective_force_finish_seconds(0, Some(100)), 3600);
        assert_eq!(effective_force_finish_seconds(7200, Some(100)), 7200);
        assert_eq!(effective_force_finish_seconds(3600, Some(3550)), 3670);
    }
}
