use std::fmt;

use crate::i18n::I18n;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameError {
    message_key: String,
}

impl GameError {
    pub fn new(message_key: impl Into<String>) -> Self {
        Self {
            message_key: message_key.into(),
        }
    }

    pub fn message_key(&self) -> &str {
        &self.message_key
    }

    pub fn localized(&self, i18n: &I18n, language: &str) -> String {
        i18n.message(language, &self.message_key)
    }

    pub fn invalid_state() -> Self {
        Self::new("error.invalid_state")
    }

    pub fn permission_denied() -> Self {
        Self::new("error.permission_denied")
    }

    pub fn room_full() -> Self {
        Self::new("error.room_full")
    }

    pub fn room_locked() -> Self {
        Self::new("error.room_locked")
    }

    pub fn room_not_found() -> Self {
        Self::new("error.room_not_found")
    }

    pub fn room_already_exists() -> Self {
        Self::new("error.room_already_exists")
    }

    pub fn room_limit_reached(max_rooms: i32) -> Self {
        Self::new(format!(
            "服务器房间数已达上限（{max_rooms}），无法继续创建。"
        ))
    }

    pub fn chart_not_selected() -> Self {
        Self::new("error.chart_not_selected")
    }

    pub fn chart_not_found() -> Self {
        Self::new("error.chart_not_found")
    }

    pub fn chat_not_enabled() -> Self {
        Self::new("error.chat_not_enabled")
    }

    pub fn record_not_found() -> Self {
        Self::new("error.record_not_found")
    }

    pub fn record_submit_failed() -> Self {
        Self::new("成绩提交失败，请稍后重试")
    }

    pub fn chart_pool_unavailable() -> Self {
        Self::new("当前不在投票阶段，无法查看谱池状态。")
    }

    pub fn already_in_room() -> Self {
        Self::new("error.already_in_room")
    }

    pub fn not_in_room() -> Self {
        Self::new("error.not_in_room")
    }

    pub fn not_host() -> Self {
        Self::new("error.not_host")
    }

    pub fn player_not_found() -> Self {
        Self::new("error.player_not_found")
    }

    pub fn session_expired() -> Self {
        Self::new("error.session_expired")
    }

    pub fn authentication_failed() -> Self {
        Self::new("error.authentication_failed")
    }
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message_key)
    }
}

impl std::error::Error for GameError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_literals() {
        assert_eq!(GameError::room_full().message_key(), "error.room_full");
        assert_eq!(GameError::not_host().to_string(), "error.not_host");
        assert_eq!(
            GameError::room_limit_reached(200).message_key(),
            "服务器房间数已达上限（200），无法继续创建。"
        );
        assert_eq!(
            GameError::record_submit_failed().message_key(),
            "成绩提交失败，请稍后重试"
        );
        assert_eq!(
            GameError::chart_pool_unavailable().message_key(),
            "当前不在投票阶段，无法查看谱池状态。"
        );
        let i18n = I18n::new("zh-CN");
        assert_eq!(
            GameError::room_full().localized(&i18n, "en-US"),
            "Room is full"
        );
        assert_eq!(GameError::room_full().localized(&i18n, "zh-CN"), "房间已满");
        assert_eq!(
            GameError::record_submit_failed().localized(&i18n, "en-US"),
            "成绩提交失败，请稍后重试"
        );
    }
}
