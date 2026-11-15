//! Player points, round history and the admin list: three small stores under `data/`.

mod admins;
mod error;
mod import;
mod persist;
mod points;
mod records;
mod util;
mod writer;

pub use admins::{Admins, RemoveOutcome};
pub use error::LedgerError;
pub use import::{BackupFn, ImportError, ImportMode, ImportReport, ImportResult, MAX_IMPORT_BYTES};
pub use points::{PointAward, PointSummary, Points, RankEntry};
pub use records::{
    MAX_RECORDS, PlayerResult, PlayerRound, PlayerRoundView, RecordView, ResultView, RoundRecord,
    RoundRecords,
};
pub use util::{Clock, system_clock};
pub use writer::{SAVE_INTERVAL, WriterHandle};
