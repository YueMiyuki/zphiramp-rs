//! Client for the upstream Phira API: models, caches, and the chart duration probe.
//! Parsing of chart archives is pure and lives in `duration`.

mod cache;
mod client;
pub mod duration;
mod error;
pub mod model;

pub use client::{DEFAULT_BASE_URL, Options, PhiraClient};
pub use error::PhiraError;
pub use model::{ChartInfo, ChartListPage, GameRecord, LoginResult, UserInfo};
