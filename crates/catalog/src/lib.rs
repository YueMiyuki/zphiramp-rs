//! Chart pools, screening rules, the chart index and player submissions.

mod cache;
mod category;
mod config;
mod error;
mod index;
mod persist;
mod pool;
mod screening;
mod submissions;
mod transfer;
mod upstream;

pub use category::PoolCategory;
pub use config::{AtomicFiles, CatalogConfig, Clock, FileSink, Sleeper, SystemClock, TokioSleeper};
pub use error::CatalogError;
pub use index::{
    ChartIndex, INCREMENTAL_PAGES, IndexLookup, MAX_PROBE_BUDGET, PROBE_CONCURRENCY,
    PROBE_FAILURE_MEMORY, PROBE_TIMEOUT, REQUEST_INTERVAL, RefreshOutcome, SWEEP_PAUSE,
};
pub use pool::{
    ChartPool, GenerateOutcome, MAX_BATCH_CHARTS, MAX_BATCH_POOLS, MAX_GENERATED_POOLS,
    MAX_POOL_ORDER, PoolSnapshot,
};
pub use screening::ScreeningRule;
pub use submissions::{
    ApproveOutcome, DEFAULT_MAX_PENDING_SUBMISSIONS, MAX_SUBMITTERS, REASON_ALREADY_IN_POOL,
    REASON_ALREADY_SUBMITTED, REASON_SUBMITTERS_FULL, SubmissionStatus, SubmissionView,
    Submissions, SubmitResult, Submitter, pending_limit_reason,
};
pub use transfer::{BackupFn, ImportMode, ImportResult, MAX_IMPORT_LENGTH};
pub use upstream::{BoxFuture, Upstream, UpstreamError};

pub mod files {
    pub use crate::persist::{
        CACHE_FILE, DURATION_RESET_MARKER, INDEX_FILE, POOL_FILE, SUBMISSIONS_FILE,
    };
}
