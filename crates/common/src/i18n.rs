use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::gson;

pub const FALLBACK_LANGUAGE: &str = "zh-CN";

const BUNDLED: [(&str, &str); 2] = [
    ("en-US", include_str!("../lang/en-US.json")),
    ("zh-CN", include_str!("../lang/zh-CN.json")),
];

/// Unknown languages fall back to zh-CN; a key without a translation is returned as is.
#[derive(Clone)]
pub struct I18n {
    inner: Arc<Inner>,
    default_language: Arc<str>,
}

struct Inner {
    tables: HashMap<&'static str, HashMap<String, String>>,
    warned: Mutex<HashSet<String>>,
}

impl I18n {
    pub fn new(default_language: &str) -> Self {
        let tables = BUNDLED
            .iter()
            .map(|(lang, json)| {
                let table: HashMap<String, String> = gson::from_str(json)
                    .unwrap_or_else(|e| panic!("bundled lang/{lang}.json: {e}"));
                (*lang, table)
            })
            .collect();
        Self {
            inner: Arc::new(Inner {
                tables,
                warned: Mutex::new(HashSet::new()),
            }),
            default_language: Arc::from(default_language),
        }
    }

    pub fn with_default_language(&self, default_language: &str) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            default_language: Arc::from(default_language),
        }
    }

    pub fn default_language(&self) -> &str {
        &self.default_language
    }

    pub fn bundled_languages(&self) -> impl Iterator<Item = &str> {
        BUNDLED.iter().map(|(lang, _)| *lang)
    }

    pub fn message(&self, language: &str, key: &str) -> String {
        let table = match self.inner.tables.get(language) {
            Some(t) => t,
            None => {
                self.warn_once(language);
                &self.inner.tables[FALLBACK_LANGUAGE]
            }
        };
        table.get(key).cloned().unwrap_or_else(|| key.to_owned())
    }

    pub fn message_default(&self, key: &str) -> String {
        self.message(&self.default_language, key)
    }

    pub fn message_for(&self, player_language: Option<&str>, key: &str) -> String {
        self.message(player_language.unwrap_or(&self.default_language), key)
    }

    fn warn_once(&self, language: &str) {
        let first = match self.inner.warned.lock() {
            Ok(mut warned) => warned.insert(language.to_owned()),
            Err(poisoned) => poisoned.into_inner().insert(language.to_owned()),
        };
        if first {
            tracing::warn!(
                "Language file not found: {}, falling back to default",
                language
            );
        }
    }
}

impl std::fmt::Debug for I18n {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("I18n")
            .field("default_language", &self.default_language)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups_and_fallback() {
        let i18n = I18n::new("zh-CN");
        assert_eq!(i18n.message("en-US", "error.room_full"), "Room is full");
        assert_eq!(i18n.message("zh-CN", "error.room_full"), "房间已满");
        assert_eq!(i18n.message("ja-JP", "error.room_full"), "房间已满");
        assert_eq!(i18n.message("ja-JP", "error.room_full"), "房间已满");
        assert_eq!(
            i18n.message("en-US", "成绩提交失败，请稍后重试"),
            "成绩提交失败，请稍后重试"
        );
        assert_eq!(
            i18n.message_default("system.live_recorder_name"),
            "录制状态设置器(请忽略该账号)"
        );
        let en = i18n.with_default_language("en-US");
        assert_eq!(
            en.message_for(None, "error.not_host"),
            "You are not the host"
        );
        assert_eq!(
            en.message_for(Some("zh-CN"), "error.not_host"),
            "你不是房主"
        );
        assert_eq!(i18n.inner.warned.lock().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_first_lookups_never_fail() {
        let i18n = I18n::new("fr-FR");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let i18n = i18n.clone();
                std::thread::spawn(move || i18n.message(&format!("x-{}", i % 3), "error.not_host"))
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), "你不是房主");
        }
        assert_eq!(i18n.message_default("error.not_host"), "你不是房主");
    }
}
