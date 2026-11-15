use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PoolCategory {
    Regular,
    Configured,
    Tb,
    Manual,
}

impl PoolCategory {
    pub const ALL: [PoolCategory; 4] = [Self::Regular, Self::Configured, Self::Tb, Self::Manual];

    pub fn name(self) -> &'static str {
        match self {
            Self::Regular => "REGULAR",
            Self::Configured => "CONFIGURED",
            Self::Tb => "TB",
            Self::Manual => "MANUAL",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    pub fn parse_ignore_case(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        Self::ALL
            .into_iter()
            .find(|c| c.name().eq_ignore_ascii_case(raw))
    }
}

impl fmt::Display for PoolCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
