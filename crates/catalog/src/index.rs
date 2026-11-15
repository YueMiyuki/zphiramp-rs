use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use zphira_common::{fs, json};
use zphira_phira::ChartInfo;

use crate::cache::ChartInfoCache;
use crate::config::{CatalogConfig, Clock, FileSink, Sleeper};
use crate::error::CatalogError;
use crate::persist::{
    ChartMap, ChartMapRef, INDEX_FILE, SerialWriter, duration_reset_pending, remove_stale,
    run_blocking, write_duration_reset_marker, write_json_blocking,
};
use crate::pool::ChartPool;
use crate::screening::ScreeningRule;
use crate::upstream::{Upstream, fetch_chart};

pub const REQUEST_INTERVAL: Duration = Duration::from_millis(60);
pub const SWEEP_PAUSE: Duration = Duration::from_millis(4000);
pub const INCREMENTAL_PAGES: i32 = 40;
pub const MAX_PROBE_BUDGET: usize = 500;
pub const PROBE_CONCURRENCY: usize = 4;
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(300);
pub const PROBE_FAILURE_MEMORY: Duration = Duration::from_secs(6 * 3600);

const TAG_PLAIN: &str = "plain";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    Refreshed { indexed: usize },
    AlreadyRunning { indexed: usize },
}

impl RefreshOutcome {
    pub fn indexed(self) -> usize {
        match self {
            Self::Refreshed { indexed } | Self::AlreadyRunning { indexed } => indexed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IndexLookup {
    pub chart_id: i32,
    pub result: Result<Arc<ChartInfo>, CatalogError>,
}

#[derive(Clone)]
pub struct ChartIndex {
    inner: Arc<IndexInner>,
}

struct IndexInner {
    path: PathBuf,
    files: Arc<dyn FileSink>,
    upstream: Arc<dyn Upstream>,
    sleeper: Arc<dyn Sleeper>,
    clock: Arc<dyn Clock>,
    runtime: Handle,
    charts: RwLock<ChartMap>,
    refreshing: AtomicBool,
    total_on_remote: AtomicI32,
    writer: SerialWriter,
    failures: Mutex<HashMap<i32, Instant>>,
    in_flight: Mutex<HashSet<i32>>,
    cache: ChartInfoCache,
}

impl ChartIndex {
    /// Call after `ChartPool::load`, inside the Tokio runtime (it finishes the duration reset).
    pub async fn load(
        config: &CatalogConfig,
        upstream: Arc<dyn Upstream>,
        pool: &ChartPool,
    ) -> ChartIndex {
        let blocking_config = config.clone();
        let (loaded, reset_pending) = run_blocking(move || {
            remove_stale(&blocking_config, INDEX_FILE);
            Ok((
                read_index(&blocking_config),
                duration_reset_pending(&blocking_config),
            ))
        })
        .await
        .unwrap_or_else(|e| (Err(e.to_string()), false));
        let charts = match loaded {
            Ok(charts) => charts,
            Err(message) => {
                tracing::warn!(
                    "Chart index cache is unreadable ({}), starting empty; run a refresh to rebuild it",
                    message
                );
                ChartMap::new()
            }
        };

        let index = ChartIndex {
            inner: Arc::new(IndexInner {
                path: config.path(INDEX_FILE),
                files: config.files.clone(),
                upstream,
                sleeper: config.sleeper.clone(),
                clock: config.clock.clone(),
                runtime: Handle::current(),
                charts: RwLock::new(charts),
                refreshing: AtomicBool::new(false),
                total_on_remote: AtomicI32::new(-1),
                writer: SerialWriter::default(),
                failures: Mutex::new(HashMap::new()),
                in_flight: Mutex::new(HashSet::new()),
                cache: pool.cache().clone(),
            }),
        };
        if reset_pending {
            index.reset_durations(config).await;
        }
        index
    }

    async fn reset_durations(&self, config: &CatalogConfig) {
        let cleared = {
            let charts = self.read();
            let mut cleared = 0;
            for chart in charts.values() {
                if chart.duration_seconds().is_some() {
                    chart.set_duration_seconds(None);
                    cleared += 1;
                }
            }
            cleared
        };
        if cleared > 0
            && let Err(e) = self.save().await
        {
            tracing::warn!(
                "Could not save the chart index after clearing old durations ({}); will retry at the next start",
                e
            );
            return;
        }
        let config = config.clone();
        match run_blocking(move || write_duration_reset_marker(&config)).await {
            Ok(()) => {
                if cleared > 0 {
                    tracing::info!(
                        "Cleared {} chart durations probed by an older server; they will be probed again",
                        cleared
                    );
                }
            }
            Err(e) => tracing::warn!("Could not write the duration reset marker: {}", e),
        }
    }

    pub fn is_refreshing(&self) -> bool {
        self.inner.refreshing.load(Ordering::SeqCst)
    }

    pub fn indexed_count(&self) -> usize {
        self.read().len()
    }

    /// -1 before any sweep.
    pub fn remote_count(&self) -> i32 {
        self.inner.total_on_remote.load(Ordering::SeqCst)
    }

    pub fn all(&self) -> Vec<Arc<ChartInfo>> {
        self.read().values().cloned().collect()
    }

    pub fn get(&self, chart_id: i32) -> Option<Arc<ChartInfo>> {
        self.read().get(&chart_id).cloned()
    }

    pub fn search(&self, rule: &ScreeningRule, limit: usize) -> Vec<Arc<ChartInfo>> {
        let mut matched: Vec<Arc<ChartInfo>> = self
            .read()
            .values()
            .filter(|chart| rule.matches(chart))
            .cloned()
            .collect();
        matched.sort_by(|a, b| b.rating.total_cmp(&a.rating).then(a.id.cmp(&b.id)));
        matched.truncate(limit);
        matched
    }

    pub fn count_matches(&self, rule: &ScreeningRule) -> usize {
        self.read()
            .values()
            .filter(|chart| rule.matches(chart))
            .count()
    }

    pub async fn refresh(&self, division: Option<&str>) -> Result<RefreshOutcome, CatalogError> {
        self.refresh_with(Request::Listing(division.map(str::to_owned)))
            .await
    }

    pub async fn refresh_full(&self) -> Result<RefreshOutcome, CatalogError> {
        self.refresh_with(Request::Full).await
    }

    async fn refresh_with(&self, request: Request) -> Result<RefreshOutcome, CatalogError> {
        let Some(guard) = RefreshGuard::begin(&self.inner) else {
            return Ok(RefreshOutcome::AlreadyRunning {
                indexed: self.indexed_count(),
            });
        };
        let result = self.run_refresh(&request).await;
        drop(guard);
        result.map(|indexed| RefreshOutcome::Refreshed { indexed })
    }

    /// Returns false, doing nothing, while another refresh runs.
    pub fn refresh_async(&self, division: Option<String>) -> bool {
        self.spawn_refresh(Request::Listing(division))
    }

    pub fn refresh_full_async(&self) -> bool {
        self.spawn_refresh(Request::Full)
    }

    fn spawn_refresh(&self, request: Request) -> bool {
        let Some(guard) = RefreshGuard::begin(&self.inner) else {
            return false;
        };
        let index = self.clone();
        self.inner.runtime.spawn(async move {
            let _guard = guard;
            if let Err(e) = index.run_refresh(&request).await {
                tracing::error!(
                    error = &e as &dyn std::error::Error,
                    "Chart index refresh failed"
                );
            }
        });
        true
    }

    async fn run_refresh(&self, request: &Request) -> Result<usize, CatalogError> {
        let mut merged = 0;
        if let Err(e) = self.run_sweeps(request, &mut merged).await {
            if merged > 0 {
                self.save_unchecked().await;
            }
            return Err(e);
        }
        self.save().await.map_err(CatalogError::storage)?;
        let indexed = self.indexed_count();
        tracing::info!(
            "Chart index refreshed: {} charts (division={})",
            indexed,
            match request {
                Request::Listing(Some(division)) => division.as_str(),
                Request::Listing(None) | Request::Full => "all",
            }
        );
        Ok(indexed)
    }

    async fn run_sweeps(&self, request: &Request, merged: &mut usize) -> Result<(), CatalogError> {
        let division = match request {
            Request::Full => {
                tracing::info!("Running a full chart index sweep on request");
                self.sweep(Some(TAG_PLAIN), i32::MAX, true, merged).await?;
                self.inner.sleeper.sleep(SWEEP_PAUSE).await;
                return self.sweep(None, i32::MAX, true, merged).await;
            }
            Request::Listing(division) => division.as_deref(),
        };
        let Some(division) = division else {
            // Decided before the plain pass fills the index.
            let empty = self.read().is_empty();
            if self.count_tagged(TAG_PLAIN) == 0 {
                self.sweep(Some(TAG_PLAIN), i32::MAX, false, merged).await?;
                self.inner.sleeper.sleep(SWEEP_PAUSE).await;
            }
            if empty {
                tracing::info!("Chart index is empty, running a full sweep (slow on first run)");
            }
            let max_pages = if empty { i32::MAX } else { INCREMENTAL_PAGES };
            return self.sweep(None, max_pages, false, merged).await;
        };
        self.sweep(Some(division), i32::MAX, false, merged).await
    }

    fn count_tagged(&self, tag: &str) -> usize {
        self.read()
            .values()
            .filter(|chart| chart.has_tag(tag))
            .count()
    }

    async fn sweep(
        &self,
        division: Option<&str>,
        max_pages: i32,
        to_the_end: bool,
        merged: &mut usize,
    ) -> Result<(), CatalogError> {
        let mut seen: i64 = 0;
        for page in 1..=max_pages {
            let result = self.inner.upstream.chart_page(page, division).await?;
            self.inner
                .total_on_remote
                .store(result.count, Ordering::SeqCst);
            let charts = match result.results {
                Some(charts) if !charts.is_empty() => charts,
                _ => return Ok(()),
            };
            let fresh = self.merge_page(&charts);
            *merged += charts.len();
            // The listing is ordered by `updated` desc: nothing new on a page ends the pass.
            if fresh == 0 && !to_the_end {
                return Ok(());
            }
            seen += charts.len() as i64;
            if seen >= i64::from(result.count) {
                return Ok(());
            }
            self.inner.sleeper.sleep(REQUEST_INTERVAL).await;
        }
        Ok(())
    }

    fn merge_page(&self, page: &[Arc<ChartInfo>]) -> usize {
        let mut charts = self.write();
        let mut fresh_count = 0;
        for fresh in page {
            match charts.get(&fresh.id) {
                None => {
                    charts.insert(fresh.id, fresh.clone());
                    fresh_count += 1;
                }
                Some(cached) => {
                    if let Some(seconds) = cached.duration_seconds() {
                        fresh.set_duration_seconds(Some(seconds));
                    }
                    charts.insert(fresh.id, fresh.clone());
                }
            }
        }
        fresh_count
    }

    pub async fn ensure_indexed(&self, chart_ids: &[i32]) -> Vec<IndexLookup> {
        let mut results = Vec::with_capacity(chart_ids.len());
        let mut added = false;
        for &chart_id in chart_ids {
            let result = match self.get(chart_id) {
                Some(chart) => Ok(chart),
                None => match fetch_chart(&*self.inner.upstream, chart_id).await {
                    Ok(chart) => {
                        let chart = self.write().entry(chart_id).or_insert(chart).clone();
                        added = true;
                        Ok(chart)
                    }
                    Err(e) => Err(CatalogError::Upstream(e)),
                },
            };
            results.push(IndexLookup { chart_id, result });
        }
        if added {
            self.save_unchecked().await;
        }
        results
    }

    pub async fn probe_durations(&self, charts: &[Arc<ChartInfo>], limit: usize) -> usize {
        let budget = limit.min(MAX_PROBE_BUDGET);
        let now = self.inner.clock.now();
        let mut skipped = 0;
        let pending: Vec<Arc<ChartInfo>> = {
            let mut failures = self.failures();
            failures.retain(|_, at| now.saturating_duration_since(*at) < PROBE_FAILURE_MEMORY);
            charts
                .iter()
                .filter(|chart| chart.duration_seconds().is_none())
                .filter(|chart| {
                    let failed = failures.contains_key(&chart.id);
                    skipped += usize::from(failed);
                    !failed
                })
                .take(budget)
                .cloned()
                .collect()
        };
        if skipped > 0 {
            tracing::info!(
                "Skipped {} chart(s) whose duration probe failed in the last 6 hours",
                skipped
            );
        }
        if pending.is_empty() {
            return 0;
        }

        let permits = Arc::new(Semaphore::new(PROBE_CONCURRENCY));
        let mut tasks = JoinSet::new();
        for chart in &pending {
            let chart = chart.clone();
            let permits = permits.clone();
            let upstream = self.inner.upstream.clone();
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let result = upstream.probe_duration(chart.id).await;
                (chart, result)
            });
        }
        let mut resolved = 0;
        let mut failed = 0;
        let mut cache_touched = false;
        let collect = async {
            while let Some(joined) = tasks.join_next().await {
                let Ok((chart, result)) = joined else {
                    failed += 1;
                    continue;
                };
                match result {
                    Ok(seconds) => {
                        chart.set_duration_seconds(Some(seconds));
                        cache_touched |= self.record_duration(chart.id, seconds);
                        resolved += 1;
                    }
                    Err(e) => {
                        self.record_failure(chart.id);
                        failed += 1;
                        tracing::warn!("Duration probe failed for chart {}: {}", chart.id, e);
                    }
                }
            }
        };
        if tokio::time::timeout(PROBE_TIMEOUT, collect).await.is_err() {
            tasks.abort_all();
        }

        if resolved > 0 {
            self.save_unchecked().await;
            if cache_touched {
                self.save_cache_unchecked().await;
            }
        }
        tracing::info!(
            "Duration probe finished: {}/{} resolved, {} failed",
            resolved,
            pending.len(),
            failed
        );
        resolved
    }

    /// Spawns the probe and returns at once; safe under a room lock.
    pub fn probe_duration_async(&self, chart: &Arc<ChartInfo>) {
        if chart.duration_seconds().is_some() {
            return;
        }
        let chart_id = chart.id;
        if self.recently_failed(chart_id) {
            return;
        }
        if !self
            .inner
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(chart_id)
        {
            return;
        }
        let index = self.clone();
        let chart = chart.clone();
        self.inner.runtime.spawn(async move {
            match index.inner.upstream.probe_duration(chart_id).await {
                Ok(seconds) => {
                    chart.set_duration_seconds(Some(seconds));
                    let cached = index.record_duration(chart_id, seconds);
                    index.save_unchecked().await;
                    if cached {
                        index.save_cache_unchecked().await;
                    }
                    tracing::info!("Probed duration of chart {}: {}s", chart_id, seconds);
                }
                Err(e) => {
                    index.record_failure(chart_id);
                    tracing::warn!("Duration probe failed for chart {}: {}", chart_id, e);
                }
            }
            index
                .inner
                .in_flight
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&chart_id);
        });
    }

    pub fn recently_failed(&self, chart_id: i32) -> bool {
        let now = self.inner.clock.now();
        self.failures()
            .get(&chart_id)
            .is_some_and(|at| now.saturating_duration_since(*at) < PROBE_FAILURE_MEMORY)
    }

    fn record_duration(&self, chart_id: i32, seconds: i32) -> bool {
        if let Some(entry) = self.read().get(&chart_id) {
            entry.set_duration_seconds(Some(seconds));
        }
        self.failures().remove(&chart_id);
        self.inner.cache.set_duration(chart_id, seconds)
    }

    fn record_failure(&self, chart_id: i32) {
        let now = self.inner.clock.now();
        self.failures().insert(chart_id, now);
    }

    async fn save(&self) -> std::io::Result<()> {
        let inner = self.inner.clone();
        run_blocking(move || {
            inner.writer.save(|| {
                let snapshot: ChartMap = inner
                    .charts
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                write_json_blocking(&*inner.files, &inner.path, &ChartMapRef(&snapshot))
            })
        })
        .await
    }

    async fn save_unchecked(&self) {
        if let Err(e) = self.save().await {
            tracing::error!(
                error = &e as &dyn std::error::Error,
                "Failed to save chart index"
            );
        }
    }

    async fn save_cache_unchecked(&self) {
        if let Err(e) = self.inner.cache.save().await {
            tracing::error!(
                error = &e as &dyn std::error::Error,
                "Failed to save chart info cache"
            );
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, ChartMap> {
        self.inner
            .charts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, ChartMap> {
        self.inner
            .charts
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn failures(&self) -> std::sync::MutexGuard<'_, HashMap<i32, Instant>> {
        self.inner
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

enum Request {
    Listing(Option<String>),
    Full,
}

struct RefreshGuard {
    inner: Arc<IndexInner>,
}

impl RefreshGuard {
    fn begin(inner: &Arc<IndexInner>) -> Option<Self> {
        inner
            .refreshing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self {
                inner: inner.clone(),
            })
    }
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.inner.refreshing.store(false, Ordering::SeqCst);
    }
}

fn read_index(config: &CatalogConfig) -> Result<ChartMap, String> {
    let text = fs::read_to_string_if_exists(config.path(INDEX_FILE)).map_err(|e| e.to_string())?;
    match text {
        None => Ok(ChartMap::new()),
        Some(text) => json::from_str_opt::<BTreeMap<i32, ChartInfo>>(&text)
            .map(|map| {
                map.unwrap_or_default()
                    .into_iter()
                    .map(|(id, chart)| (id, Arc::new(chart)))
                    .collect()
            })
            .map_err(|e| e.to_string()),
    }
}
