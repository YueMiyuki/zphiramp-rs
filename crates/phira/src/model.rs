use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zphira_common::time::JavaOffsetDateTime;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserInfo {
    pub id: i32,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub language: Option<String>,
    pub bio: Option<String>,
    pub exp: i32,
    pub rks: f64,
    pub joined: Option<JavaOffsetDateTime>,
    #[serde(rename = "lastLogin")]
    pub last_login: Option<JavaOffsetDateTime>,
    pub roles: i32,
    pub banned: bool,
    #[serde(rename = "loginBanned")]
    pub login_banned: bool,
    #[serde(rename = "followerCount")]
    pub follower_count: i32,
    #[serde(rename = "followingCount")]
    pub following_count: i32,
    pub email: Option<String>,
}

impl UserInfo {
    pub fn name_or_null(&self) -> &str {
        self.name.as_deref().unwrap_or("null")
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartInfo {
    pub id: i32,
    pub name: Option<String>,
    pub level: Option<String>,
    pub difficulty: f32,
    pub charter: Option<String>,
    pub composer: Option<String>,
    pub illustrator: Option<String>,
    pub description: Option<String>,
    pub ranked: bool,
    pub reviewed: bool,
    pub stable: bool,
    #[serde(rename = "stableRequest")]
    pub stable_request: bool,
    pub illustration: Option<String>,
    pub preview: Option<String>,
    pub file: Option<String>,
    pub uploader: i32,
    pub tags: Option<Vec<String>>,
    pub rating: f32,
    #[serde(rename = "ratingCount")]
    pub rating_count: i32,
    pub created: Option<JavaOffsetDateTime>,
    pub updated: Option<JavaOffsetDateTime>,
    #[serde(rename = "chartUpdated")]
    pub chart_updated: Option<JavaOffsetDateTime>,
    duration_seconds: DurationCell,
}

impl ChartInfo {
    pub fn name_or_null(&self) -> &str {
        self.name.as_deref().unwrap_or("null")
    }

    pub fn level_or_null(&self) -> &str {
        self.level.as_deref().unwrap_or("null")
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|t| t == tag))
    }

    pub fn duration_seconds(&self) -> Option<i32> {
        self.duration_seconds.get()
    }

    /// Takes `&self`: holders of the same `Arc<ChartInfo>` see the new duration.
    pub fn set_duration_seconds(&self, value: Option<i32>) {
        self.duration_seconds.set(value);
    }
}

#[derive(Debug)]
pub(crate) struct DurationCell(AtomicI64);

impl DurationCell {
    // i32 values are stored widened, so i64::MIN can never be a real duration.
    const NONE: i64 = i64::MIN;

    fn get(&self) -> Option<i32> {
        let raw = self.0.load(Ordering::Acquire);
        if raw == Self::NONE {
            None
        } else {
            i32::try_from(raw).ok()
        }
    }

    fn set(&self, value: Option<i32>) {
        let raw = value.map_or(Self::NONE, i64::from);
        self.0.store(raw, Ordering::Release);
    }
}

impl Default for DurationCell {
    fn default() -> Self {
        Self(AtomicI64::new(Self::NONE))
    }
}

impl Clone for DurationCell {
    fn clone(&self) -> Self {
        Self(AtomicI64::new(self.0.load(Ordering::Acquire)))
    }
}

impl PartialEq for DurationCell {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl Serialize for DurationCell {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.get().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DurationCell {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let cell = Self::default();
        cell.set(Option::<i32>::deserialize(deserializer)?);
        Ok(cell)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct GameRecord {
    pub id: i32,
    pub player: i32,
    pub chart: i32,
    pub score: i32,
    pub accuracy: f32,
    pub perfect: i32,
    pub good: i32,
    pub bad: i32,
    pub miss: i32,
    pub speed: f32,
    pub max_combo: i32,
    pub best: bool,
    #[serde(rename = "bestStd")]
    pub best_std: bool,
    pub mods: i32,
    #[serde(rename = "fullCombo")]
    pub full_combo: bool,
    pub time: Option<JavaOffsetDateTime>,
    pub std: f32,
    #[serde(rename = "stdScore")]
    pub std_score: f32,
}

// The Gson reader identifies the struct by this slice's address: keep it a single static.
const GAME_RECORD_KEYS: &[&str] = &[
    "id",
    "player",
    "chart",
    "score",
    "accuracy",
    "perfect",
    "good",
    "bad",
    "miss",
    "speed",
    "max_combo",
    "best",
    "bestStd",
    "best_std",
    "mods",
    "fullCombo",
    "full_combo",
    "time",
    "std",
    "stdScore",
    "std_score",
];

impl<'de> Deserialize<'de> for GameRecord {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecordVisitor;

        impl<'de> Visitor<'de> for RecordVisitor {
            type Value = GameRecord;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a GameRecord object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<GameRecord, A::Error> {
                let mut r = GameRecord::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "id" => r.id = map.next_value()?,
                        "player" => r.player = map.next_value()?,
                        "chart" => r.chart = map.next_value()?,
                        "score" => r.score = map.next_value()?,
                        "accuracy" => r.accuracy = map.next_value()?,
                        "perfect" => r.perfect = map.next_value()?,
                        "good" => r.good = map.next_value()?,
                        "bad" => r.bad = map.next_value()?,
                        "miss" => r.miss = map.next_value()?,
                        "speed" => r.speed = map.next_value()?,
                        "max_combo" => r.max_combo = map.next_value()?,
                        "best" => r.best = map.next_value()?,
                        "bestStd" | "best_std" => r.best_std = map.next_value()?,
                        "mods" => r.mods = map.next_value()?,
                        "fullCombo" | "full_combo" => r.full_combo = map.next_value()?,
                        "time" => r.time = map.next_value()?,
                        "std" => r.std = map.next_value()?,
                        "stdScore" | "std_score" => r.std_score = map.next_value()?,
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(r)
            }
        }

        deserializer.deserialize_struct("GameRecord", GAME_RECORD_KEYS, RecordVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginResult {
    pub id: i32,
    pub token: Option<String>,
    pub refresh_token: Option<String>,
    pub expire_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartListPage {
    pub count: i32,
    pub results: Option<Vec<Arc<ChartInfo>>>,
}
