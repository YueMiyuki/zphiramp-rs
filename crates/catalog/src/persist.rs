use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde::ser::SerializeMap;
use zphira_phira::ChartInfo;

use crate::config::{CatalogConfig, FileSink};

pub const POOL_FILE: &str = "chart-pools.json";
pub const CACHE_FILE: &str = "chart-info-cache.json";
pub const INDEX_FILE: &str = "chart-index.json";
pub const SUBMISSIONS_FILE: &str = "submissions.json";
pub const DURATION_RESET_MARKER: &str = ".duration-probe-v2";

pub(crate) type ChartMap = BTreeMap<i32, Arc<ChartInfo>>;

pub(crate) struct ChartMapRef<'a>(pub(crate) &'a ChartMap);

impl Serialize for ChartMapRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (id, chart) in self.0 {
            map.serialize_entry(id, chart.as_ref())?;
        }
        map.end()
    }
}

pub(crate) fn write_json_blocking<T: Serialize + ?Sized>(
    files: &dyn FileSink,
    path: &Path,
    value: &T,
) -> io::Result<()> {
    files.write_atomic(path, &mut |w| {
        serde_json::to_writer_pretty(w, value).map_err(io::Error::from)
    })
}

pub(crate) async fn run_blocking<R, F>(work: F) -> io::Result<R>
where
    F: FnOnce() -> io::Result<R> + Send + 'static,
    R: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(e) => Err(io::Error::other(format!("file task failed: {e}"))),
    }
}

#[derive(Default)]
pub(crate) struct SerialWriter {
    lock: Mutex<()>,
    requested: AtomicU64,
    written: AtomicU64,
}

impl SerialWriter {
    pub(crate) fn save(&self, write: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        let ticket = self.requested.fetch_add(1, Ordering::SeqCst) + 1;
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        if self.written.load(Ordering::SeqCst) >= ticket {
            return Ok(());
        }
        self.write_locked(write)
    }

    pub(crate) fn write_now<R>(&self, write: impl FnOnce() -> io::Result<R>) -> io::Result<R> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.write_locked(write)
    }

    fn write_locked<R>(&self, write: impl FnOnce() -> io::Result<R>) -> io::Result<R> {
        // Requests made before this point changed memory first, so the snapshot taken by
        // `write` covers them.
        let covered = self.requested.load(Ordering::SeqCst);
        let result = write()?;
        self.written.fetch_max(covered, Ordering::SeqCst);
        Ok(result)
    }
}

pub(crate) fn duration_reset_pending(config: &CatalogConfig) -> bool {
    !config.path(DURATION_RESET_MARKER).exists()
}

pub(crate) fn write_duration_reset_marker(config: &CatalogConfig) -> io::Result<()> {
    config
        .files
        .write_atomic(&config.path(DURATION_RESET_MARKER), &mut |w| {
            w.write_all(
                b"Chart durations probed before zphiramp were cleared once; delete this file to clear them again.\n",
            )
        })
}

pub(crate) fn remove_stale(config: &CatalogConfig, name: &str) {
    if let Err(e) = zphira_common::fs::remove_stale_temp_files(config.path(name)) {
        tracing::debug!("could not remove stale temporary files of {name}: {e}");
    }
}

pub(crate) fn id_list(ids: &[i32]) -> String {
    let parts: Vec<String> = ids.iter().map(i32::to_string).collect();
    format!("[{}]", parts.join(", "))
}

pub(crate) fn distinct(ids: &[i32]) -> Vec<i32> {
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}
