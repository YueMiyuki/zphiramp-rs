use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zphira_common::time::Timestamp;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserInfo {
    pub id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    pub exp: i32,
    pub rks: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joined: Option<Timestamp>,
    #[serde(rename = "lastLogin", skip_serializing_if = "Option::is_none")]
    pub last_login: Option<Timestamp>,
    pub roles: i32,
    pub banned: bool,
    #[serde(rename = "loginBanned")]
    pub login_banned: bool,
    #[serde(rename = "followerCount")]
    pub follower_count: i32,
    #[serde(rename = "followingCount")]
    pub following_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    pub difficulty: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub illustrator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub ranked: bool,
    pub reviewed: bool,
    pub stable: bool,
    #[serde(rename = "stableRequest")]
    pub stable_request: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub illustration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub uploader: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub rating: f32,
    #[serde(rename = "ratingCount")]
    pub rating_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<Timestamp>,
    #[serde(rename = "chartUpdated", skip_serializing_if = "Option::is_none")]
    pub chart_updated: Option<Timestamp>,
    #[serde(skip_serializing_if = "DurationCell::is_unknown")]
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

    fn is_unknown(&self) -> bool {
        self.get().is_none()
    }

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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
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
    #[serde(rename = "bestStd", alias = "best_std")]
    pub best_std: bool,
    pub mods: i32,
    #[serde(rename = "fullCombo", alias = "full_combo")]
    pub full_combo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<Timestamp>,
    pub std: f32,
    #[serde(rename = "stdScore", alias = "std_score")]
    pub std_score: f32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginResult {
    pub id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartListPage {
    pub count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<Arc<ChartInfo>>>,
}
