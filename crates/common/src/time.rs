use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn now_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// An instant with the offset it was written with; ordered and compared by instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(chrono::DateTime<chrono::FixedOffset>);

impl Timestamp {
    pub fn now() -> Self {
        Self(chrono::Local::now().fixed_offset())
    }

    /// RFC 3339, or the same without seconds (`2026-10-04T12:30Z`).
    pub fn parse(text: &str) -> Result<Self, chrono::ParseError> {
        match chrono::DateTime::parse_from_rfc3339(text) {
            Ok(t) => Ok(Self(t)),
            Err(e) => {
                let minutes = text.get(..16).filter(|_| {
                    text.as_bytes().get(13) == Some(&b':')
                        && matches!(text.as_bytes().get(16), Some(b'Z' | b'z' | b'+' | b'-'))
                });
                match minutes {
                    Some(head) => {
                        chrono::DateTime::parse_from_rfc3339(&format!("{head}:00{}", &text[16..]))
                            .map(Self)
                    }
                    None => Err(e),
                }
            }
        }
    }

    pub fn from_epoch_millis(millis: i64) -> Option<Self> {
        chrono::DateTime::from_timestamp_millis(millis).map(|t| Self(t.fixed_offset()))
    }

    pub fn epoch_millis(&self) -> i64 {
        self.0.timestamp_millis()
    }

    pub fn datetime(&self) -> chrono::DateTime<chrono::FixedOffset> {
        self.0
    }
}

impl From<chrono::DateTime<chrono::FixedOffset>> for Timestamp {
    fn from(t: chrono::DateTime<chrono::FixedOffset>) -> Self {
        Self(t)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
    }
}

impl std::str::FromStr for Timestamp {
    type Err = chrono::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Timestamp;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an RFC 3339 timestamp")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Timestamp::parse(v).map_err(|e| E::custom(format!("{e}: {v}")))
            }
        }
        deserializer.deserialize_str(V)
    }
}
