use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use zphira_common::fs;
use zphira_common::json;
use zphira_phira::ChartInfo;

use crate::cache::ChartInfoCache;
use crate::category::PoolCategory;
use crate::config::{CatalogConfig, FileSink};
use crate::error::CatalogError;
use crate::index::ChartIndex;
use crate::persist::{
    POOL_FILE, distinct, duration_reset_pending, id_list, remove_stale, run_blocking,
    write_json_blocking,
};
use crate::screening::ScreeningRule;
use crate::transfer::{
    BackupFn, ImportMode, ImportResult, bad_row, check_enum, document_rows, has_value, row_object,
};
use crate::upstream::{Upstream, fetch_chart};

const DEFAULT_POOL_CHART_IDS: [[i32; 2]; 2] = [[30474, 42058], [33530, 52087]];
const ORDER_STEP: i32 = 10;
pub const MAX_POOL_ORDER: i32 = 1_000_000_000;
pub const MAX_GENERATED_POOLS: usize = 200;
pub const MAX_BATCH_CHARTS: usize = 500;
pub const MAX_BATCH_POOLS: i32 = 50;

const SAVE_POOLS_FAILED: &str = "Failed to save chart pools";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolSnapshot {
    pub id: i32,
    pub favorite_id: Option<i32>,
    pub default_flag: bool,
    pub chart_ids: Vec<i32>,
    pub category: PoolCategory,
    pub size_limit: Option<i32>,
    pub rounds_per_stay: Option<i32>,
    pub order: i32,
    pub submission_open: bool,
}

impl PoolSnapshot {
    fn new(id: i32, default_flag: bool, chart_ids: Vec<i32>) -> Self {
        Self {
            id,
            favorite_id: None,
            default_flag,
            chart_ids,
            category: PoolCategory::Manual,
            size_limit: None,
            rounds_per_stay: None,
            order: 0,
            submission_open: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GenerateOutcome {
    pub pool_ids: Vec<i32>,
    pub leftover: usize,
}

struct Published {
    pools: Vec<Arc<PoolSnapshot>>,
    sorted: Vec<Arc<PoolSnapshot>>,
    by_id: HashMap<i32, Arc<PoolSnapshot>>,
}

impl Published {
    fn new(pools: Vec<Arc<PoolSnapshot>>) -> Self {
        let mut sorted = pools.clone();
        sorted.sort_by_key(|p| (p.order, p.id));
        let mut by_id = HashMap::with_capacity(pools.len());
        for pool in &pools {
            by_id.entry(pool.id).or_insert_with(|| pool.clone());
        }
        Self {
            pools,
            sorted,
            by_id,
        }
    }

    fn position(&self, pool_id: i32) -> Option<usize> {
        self.pools.iter().position(|p| p.id == pool_id)
    }
}

#[derive(Clone)]
pub struct ChartPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    path: PathBuf,
    files: Arc<dyn FileSink>,
    upstream: Arc<dyn Upstream>,
    cache: ChartInfoCache,
    write_lock: Mutex<()>,
    published: ArcSwap<Published>,
}

impl ChartPool {
    /// Also fetches the info of uncached charts and rewrites both files.
    pub async fn load(
        config: &CatalogConfig,
        upstream: Arc<dyn Upstream>,
    ) -> Result<ChartPool, CatalogError> {
        let blocking_config = config.clone();
        let (cache, file, reset_pending) = run_blocking(move || {
            let config = blocking_config;
            std::fs::create_dir_all(&config.data_dir)?;
            remove_stale(&config, POOL_FILE);
            let cache = ChartInfoCache::load_blocking(&config);
            let file = fs::read_to_string_if_exists(config.path(POOL_FILE));
            Ok((cache, file, duration_reset_pending(&config)))
        })
        .await
        .map_err(CatalogError::storage)?;
        let (cache, unreadable) = cache?;
        if let Some(reason) = unreadable {
            tracing::warn!(
                "Chart info cache is unreadable ({}), starting empty; the charts of the pools will be fetched again",
                reason
            );
        }
        if reset_pending {
            cache.clear_durations();
        }

        let file = file.map_err(CatalogError::storage)?;
        let pools = match file {
            Some(text) => {
                match json::from_str_opt::<PoolConfigFile>(&text).map_err(CatalogError::json)? {
                    Some(PoolConfigFile { pools: Some(rows) }) => normalize(rows),
                    _ => default_pools(),
                }
            }
            None => default_pools(),
        };
        validate(&pools)?;

        let mut fetched: Vec<(i32, Arc<ChartInfo>)> = Vec::new();
        let mut seen = HashSet::new();
        for pool in &pools {
            for &chart_id in &pool.chart_ids {
                if seen.insert(chart_id) && !cache.contains(chart_id) {
                    let info = fetch_chart(&*upstream, chart_id).await?;
                    fetched.push((chart_id, info));
                }
            }
        }

        let pool = ChartPool {
            inner: Arc::new(PoolInner {
                path: config.path(POOL_FILE),
                files: config.files.clone(),
                upstream,
                cache,
                write_lock: Mutex::new(()),
                published: ArcSwap::from_pointee(Published::new(Vec::new())),
            }),
        };
        pool.inner
            .cache
            .commit(fetched)
            .await
            .map_err(CatalogError::storage)?;
        let pools: Vec<Arc<PoolSnapshot>> = pools.into_iter().map(Arc::new).collect();
        pool.write_pools(&pools)
            .await
            .map_err(CatalogError::storage)?;
        pool.inner.published.store(Arc::new(Published::new(pools)));
        Ok(pool)
    }

    pub(crate) fn cache(&self) -> &ChartInfoCache {
        &self.inner.cache
    }

    pub fn list_pools(&self) -> Vec<Arc<PoolSnapshot>> {
        self.inner.published.load().sorted.clone()
    }

    pub fn get_default_pools(&self) -> Vec<Arc<PoolSnapshot>> {
        self.inner
            .published
            .load()
            .sorted
            .iter()
            .filter(|p| p.default_flag && !p.chart_ids.is_empty())
            .cloned()
            .collect()
    }

    pub fn find_pool(&self, pool_id: i32) -> Option<Arc<PoolSnapshot>> {
        self.inner.published.load().by_id.get(&pool_id).cloned()
    }

    pub fn resolve_pools(&self, pool_ids: &[i32]) -> Vec<Arc<PoolSnapshot>> {
        let published = self.inner.published.load();
        distinct(pool_ids)
            .into_iter()
            .filter_map(|id| published.by_id.get(&id))
            .filter(|p| !p.chart_ids.is_empty())
            .cloned()
            .collect()
    }

    pub fn pool_ids_by_category(&self, category: PoolCategory) -> Vec<i32> {
        self.inner
            .published
            .load()
            .sorted
            .iter()
            .filter(|p| p.category == category)
            .map(|p| p.id)
            .collect()
    }

    pub fn is_submission_open(&self, pool_id: i32) -> Result<bool, CatalogError> {
        self.find_pool(pool_id)
            .map(|p| p.submission_open)
            .ok_or_else(|| pool_not_found(pool_id))
    }

    pub fn list_open_for_submission(&self) -> Vec<Arc<PoolSnapshot>> {
        self.inner
            .published
            .load()
            .sorted
            .iter()
            .filter(|p| p.submission_open)
            .cloned()
            .collect()
    }

    pub fn chart_info(&self, chart_id: i32) -> Option<Arc<ChartInfo>> {
        self.inner.cache.get(chart_id)
    }

    pub fn is_chart_info_loaded(&self, chart_id: i32) -> bool {
        self.inner.cache.contains(chart_id)
    }

    pub fn contains(chart_id: i32, pool: &[Arc<ChartInfo>]) -> bool {
        pool.iter().any(|chart| chart.id == chart_id)
    }

    pub async fn add_pool(&self, pool_id: i32, chart_ids: &[i32]) -> Result<(), CatalogError> {
        self.add_pool_with(pool_id, chart_ids, None, None, None, None)
            .await
    }

    pub async fn add_pool_with(
        &self,
        pool_id: i32,
        chart_ids: &[i32],
        category: Option<PoolCategory>,
        size_limit: Option<i32>,
        rounds_per_stay: Option<i32>,
        favorite_id: Option<i32>,
    ) -> Result<(), CatalogError> {
        if self.find_pool(pool_id).is_some() {
            return Err(pool_exists(pool_id));
        }
        let chart_ids = distinct(chart_ids);
        let (_, fetched) = self.resolve(&chart_ids).await?;

        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.published.load_full();
        if current.position(pool_id).is_some() {
            return Err(pool_exists(pool_id));
        }
        let mut pool = PoolSnapshot::new(pool_id, false, chart_ids);
        pool.favorite_id = favorite_id;
        pool.category = category.unwrap_or(PoolCategory::Manual);
        pool.size_limit = size_limit;
        pool.rounds_per_stay = rounds_per_stay;
        pool.order = next_order(&current.pools);
        let mut pools = current.pools.clone();
        pools.push(Arc::new(pool));
        self.inner
            .cache
            .commit(fetched)
            .await
            .map_err(CatalogError::storage)?;
        self.publish(pools).await.map_err(CatalogError::storage)
    }

    pub async fn remove_pool(&self, pool_id: i32) -> Result<(), CatalogError> {
        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.published.load_full();
        let index = current
            .position(pool_id)
            .ok_or_else(|| pool_not_found(pool_id))?;
        if current.pools.len() == 1 {
            return Err(CatalogError::invalid("Cannot remove the last pool"));
        }
        let default_count = current.pools.iter().filter(|p| p.default_flag).count();
        if current.pools[index].default_flag && default_count == 1 {
            return Err(CatalogError::invalid("Cannot remove the last default pool"));
        }
        let mut pools = current.pools.clone();
        pools.remove(index);
        warn_empty_pools(&pools);
        self.publish(pools).await.map_err(save_pools_failed)
    }

    pub async fn add_chart(&self, pool_id: i32, chart_id: i32) -> Result<(), CatalogError> {
        self.add_charts(pool_id, &[chart_id]).await.map(drop)
    }

    pub async fn add_charts(&self, pool_id: i32, chart_ids: &[i32]) -> Result<i32, CatalogError> {
        if self.find_pool(pool_id).is_none() {
            return Err(pool_not_found(pool_id));
        }
        let chart_ids = distinct(chart_ids);
        if chart_ids.len() > MAX_BATCH_CHARTS {
            return Err(CatalogError::invalid(format!(
                "Too many charts in one batch: {} (max {MAX_BATCH_CHARTS})",
                chart_ids.len()
            )));
        }
        let (infos, fetched) = self.resolve(&chart_ids).await?;

        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.published.load_full();
        let index = current
            .position(pool_id)
            .ok_or_else(|| pool_not_found(pool_id))?;
        let mut pool = (*current.pools[index]).clone();
        let mut added = 0;
        for info in &infos {
            if pool.chart_ids.contains(&info.id) {
                continue;
            }
            pool.chart_ids.push(info.id);
            added += 1;
        }
        if added > 0 {
            let mut pools = current.pools.clone();
            pools[index] = Arc::new(pool);
            // Chart info is cached before a pool that references it is published.
            self.inner
                .cache
                .commit(fetched)
                .await
                .map_err(CatalogError::storage)?;
            self.publish(pools).await.map_err(CatalogError::storage)?;
        }
        Ok(added)
    }

    pub async fn remove_chart(&self, pool_id: i32, chart_id: i32) -> Result<(), CatalogError> {
        self.update(pool_id, |pool| {
            match pool.chart_ids.iter().position(|&id| id == chart_id) {
                Some(position) => {
                    pool.chart_ids.remove(position);
                    true
                }
                None => false,
            }
        })
        .await
    }

    pub async fn set_favorite_id(
        &self,
        pool_id: i32,
        favorite_id: Option<i32>,
    ) -> Result<(), CatalogError> {
        self.update(pool_id, |pool| {
            pool.favorite_id = favorite_id;
            true
        })
        .await
    }

    pub async fn set_default_flag(
        &self,
        pool_id: i32,
        default_flag: bool,
    ) -> Result<(), CatalogError> {
        self.update(pool_id, |pool| {
            if pool.default_flag == default_flag {
                return false;
            }
            pool.default_flag = default_flag;
            true
        })
        .await
    }

    /// A negative size limit or rounds per stay resets that field; `order` is clamped.
    pub async fn update_pool(
        &self,
        pool_id: i32,
        category: Option<PoolCategory>,
        size_limit: Option<i32>,
        rounds_per_stay: Option<i32>,
        order: Option<i32>,
        submission_open: Option<bool>,
    ) -> Result<(), CatalogError> {
        self.update(pool_id, |pool| {
            if let Some(category) = category {
                pool.category = category;
            }
            if let Some(value) = size_limit {
                pool.size_limit = reset_if_negative(value);
            }
            if let Some(value) = rounds_per_stay {
                pool.rounds_per_stay = reset_if_negative(value);
            }
            if let Some(order) = order {
                pool.order = order.clamp(0, MAX_POOL_ORDER);
            }
            if let Some(open) = submission_open {
                pool.submission_open = open;
            }
            true
        })
        .await
    }

    pub async fn generate_pools(
        &self,
        index: &ChartIndex,
        rule: &ScreeningRule,
        size_limit: i32,
        rounds_per_stay: Option<i32>,
    ) -> Result<GenerateOutcome, CatalogError> {
        if size_limit < 1 {
            return Err(CatalogError::invalid("Pool size limit must be positive"));
        }
        let matched = index.search(rule, usize::MAX);
        if matched.is_empty() {
            return Ok(GenerateOutcome::default());
        }

        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.published.load_full();
        let placed: HashSet<i32> = current
            .pools
            .iter()
            .filter(|p| p.category == rule.category)
            .flat_map(|p| p.chart_ids.iter().copied())
            .collect();
        let candidates: Vec<Arc<ChartInfo>> = matched
            .into_iter()
            .filter(|chart| !placed.contains(&chart.id))
            .collect();
        if candidates.is_empty() {
            return Ok(GenerateOutcome::default());
        }

        let size = usize::try_from(size_limit).unwrap_or(usize::MAX);
        let mut pools = current.pools.clone();
        let mut created = Vec::new();
        let mut leftover = 0;
        let mut next_id = next_pool_id(&current.pools);
        let mut order = next_order(&current.pools);
        let mut fetched = Vec::new();
        let mut start = 0;
        while start < candidates.len() {
            if created.len() >= MAX_GENERATED_POOLS {
                leftover = candidates.len() - start;
                tracing::warn!(
                    "Stopped at {} generated pools; {} charts were left over. Raise sizeLimit or generate in several passes.",
                    MAX_GENERATED_POOLS,
                    leftover
                );
                break;
            }
            let end = start.saturating_add(size).min(candidates.len());
            let slice = &candidates[start..end];
            let mut pool = PoolSnapshot::new(next_id, false, slice.iter().map(|c| c.id).collect());
            pool.category = rule.category;
            pool.size_limit = Some(size_limit);
            pool.rounds_per_stay = rounds_per_stay;
            pool.order = order;
            order = order.saturating_add(ORDER_STEP);
            pools.push(Arc::new(pool));
            created.push(next_id);
            next_id = next_id.wrapping_add(1);
            fetched.extend(slice.iter().map(|chart| (chart.id, chart.clone())));
            start = end;
        }

        self.inner
            .cache
            .commit(fetched)
            .await
            .map_err(CatalogError::storage)?;
        self.publish(pools).await.map_err(CatalogError::storage)?;
        tracing::info!(
            "Generated {} {} pools from {} charts (sizeLimit={})",
            created.len(),
            rule.category,
            candidates.len(),
            size_limit
        );
        Ok(GenerateOutcome {
            pool_ids: created,
            leftover,
        })
    }

    pub async fn create_empty_pools(
        &self,
        count: i32,
        category: Option<PoolCategory>,
        size_limit: Option<i32>,
        rounds_per_stay: Option<i32>,
    ) -> Result<Vec<i32>, CatalogError> {
        if !(1..=MAX_BATCH_POOLS).contains(&count) {
            return Err(CatalogError::invalid(format!(
                "Pool count must be between 1 and {MAX_BATCH_POOLS}"
            )));
        }
        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.published.load_full();
        let mut pools = current.pools.clone();
        let mut created = Vec::new();
        let mut next_id = next_pool_id(&current.pools);
        let mut order = next_order(&current.pools);
        for _ in 0..count {
            let mut pool = PoolSnapshot::new(next_id, false, Vec::new());
            pool.category = category.unwrap_or(PoolCategory::Manual);
            pool.size_limit = size_limit;
            pool.rounds_per_stay = rounds_per_stay;
            pool.order = order;
            order = order.saturating_add(ORDER_STEP);
            pools.push(Arc::new(pool));
            created.push(next_id);
            next_id = next_id.wrapping_add(1);
        }
        self.publish(pools).await.map_err(save_pools_failed)?;
        tracing::info!(
            "Created {} empty {} pools: {}",
            created.len(),
            category.map_or("null", PoolCategory::name),
            id_list(&created)
        );
        Ok(created)
    }

    pub fn export_json(&self) -> Result<String, CatalogError> {
        let published = self.inner.published.load_full();
        serde_json::to_string_pretty(&PoolFileOut(&published.pools)).map_err(CatalogError::json)
    }

    pub async fn import_json(
        &self,
        document: &str,
        mode: ImportMode,
        backup: impl BackupFn,
    ) -> Result<ImportResult, CatalogError> {
        let incoming = parse_import_rows(document)?;
        if mode == ImportMode::Replace && incoming.is_empty() {
            return Err(CatalogError::invalid("pools 至少需要一个谱池"));
        }

        let mut fetched: Vec<(i32, Arc<ChartInfo>)> = Vec::new();
        let mut known: HashSet<i32> = HashSet::new();
        for (row, pool) in incoming.iter().enumerate() {
            for &chart_id in &pool.chart_ids {
                if known.contains(&chart_id) || self.inner.cache.contains(chart_id) {
                    continue;
                }
                let info = fetch_chart(&*self.inner.upstream, chart_id)
                    .await
                    .map_err(|e| {
                        bad_row(
                            "pools",
                            row,
                            format!("谱面 {chart_id} 无法获取：{}", e.message()),
                        )
                    })?;
                known.insert(chart_id);
                fetched.push((chart_id, info));
            }
        }

        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.published.load_full();
        let mut pools: Vec<Arc<PoolSnapshot>> = incoming.into_iter().map(Arc::new).collect();
        if mode == ImportMode::Merge {
            let ids: HashSet<i32> = pools.iter().map(|p| p.id).collect();
            pools.extend(
                current
                    .pools
                    .iter()
                    .filter(|p| !ids.contains(&p.id))
                    .cloned(),
            );
        }
        if pools.is_empty() {
            return Err(CatalogError::invalid("pools 至少需要一个谱池"));
        }
        let path = self.inner.path.clone();
        let backup_file = run_blocking(move || backup(&path))
            .await
            .map_err(CatalogError::storage)?;
        warn_empty_pools(&pools);
        self.inner
            .cache
            .commit(fetched)
            .await
            .map_err(CatalogError::storage)?;
        let count = i32::try_from(pools.len()).unwrap_or(i32::MAX);
        self.publish(pools).await.map_err(CatalogError::storage)?;
        Ok(ImportResult {
            imported: count,
            total: count,
            backup_file,
        })
    }

    async fn resolve(
        &self,
        chart_ids: &[i32],
    ) -> Result<(Vec<Arc<ChartInfo>>, Vec<(i32, Arc<ChartInfo>)>), CatalogError> {
        let mut infos = Vec::with_capacity(chart_ids.len());
        let mut fetched = Vec::new();
        for &chart_id in chart_ids {
            if let Some(info) = self.inner.cache.get(chart_id) {
                infos.push(info);
                continue;
            }
            let info = fetch_chart(&*self.inner.upstream, chart_id).await?;
            fetched.push((chart_id, info.clone()));
            infos.push(info);
        }
        Ok((infos, fetched))
    }

    async fn update(
        &self,
        pool_id: i32,
        change: impl FnOnce(&mut PoolSnapshot) -> bool,
    ) -> Result<(), CatalogError> {
        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.published.load_full();
        let index = current
            .position(pool_id)
            .ok_or_else(|| pool_not_found(pool_id))?;
        let mut pool = (*current.pools[index]).clone();
        if !change(&mut pool) {
            return Ok(());
        }
        let mut pools = current.pools.clone();
        pools[index] = Arc::new(pool);
        self.publish(pools).await.map_err(save_pools_failed)
    }

    async fn publish(&self, pools: Vec<Arc<PoolSnapshot>>) -> std::io::Result<()> {
        // Written before it is published, so memory and file never disagree.
        self.write_pools(&pools).await?;
        self.inner.published.store(Arc::new(Published::new(pools)));
        Ok(())
    }

    async fn write_pools(&self, pools: &[Arc<PoolSnapshot>]) -> std::io::Result<()> {
        let files = self.inner.files.clone();
        let path = self.inner.path.clone();
        let pools = pools.to_vec();
        run_blocking(move || write_json_blocking(&*files, &path, &PoolFileOut(&pools))).await
    }

    pub fn file_path(&self) -> &Path {
        &self.inner.path
    }
}

fn pool_not_found(pool_id: i32) -> CatalogError {
    CatalogError::not_found(format!("Pool not found: {pool_id}"))
}

fn pool_exists(pool_id: i32) -> CatalogError {
    CatalogError::invalid(format!("Pool already exists: {pool_id}"))
}

fn save_pools_failed(e: std::io::Error) -> CatalogError {
    CatalogError::storage_with(SAVE_POOLS_FAILED, e)
}

fn reset_if_negative(value: i32) -> Option<i32> {
    (value >= 0).then_some(value)
}

fn next_order(pools: &[Arc<PoolSnapshot>]) -> i32 {
    pools
        .iter()
        .map(|p| p.order)
        .max()
        .unwrap_or(0)
        .saturating_add(ORDER_STEP)
}

fn next_pool_id(pools: &[Arc<PoolSnapshot>]) -> i32 {
    pools
        .iter()
        .map(|p| p.id)
        .max()
        .unwrap_or(0)
        .wrapping_add(1)
}

fn warn_empty_pools(pools: &[Arc<PoolSnapshot>]) {
    for pool in pools {
        if pool.chart_ids.is_empty() {
            tracing::warn!("Pool {} is empty, rooms will skip it", pool.id);
        }
    }
}

fn validate(pools: &[PoolSnapshot]) -> Result<(), CatalogError> {
    if pools.is_empty() {
        return Err(CatalogError::bad_data("No chart pools configured"));
    }
    for pool in pools {
        if pool.chart_ids.is_empty() {
            tracing::warn!("Pool {} is empty, rooms will skip it", pool.id);
        }
    }
    Ok(())
}

fn default_pools() -> Vec<PoolSnapshot> {
    DEFAULT_POOL_CHART_IDS
        .iter()
        .enumerate()
        .map(|(i, ids)| PoolSnapshot::new(i as i32, i == 0, ids.to_vec()))
        .collect()
}

fn normalize(rows: Vec<Option<PoolDefinitionFile>>) -> Vec<PoolSnapshot> {
    rows.into_iter()
        .flatten()
        .map(PoolDefinitionFile::normalize)
        .collect()
}

#[derive(Deserialize)]
#[serde(default)]
struct PoolConfigFile {
    pools: Option<Vec<Option<PoolDefinitionFile>>>,
}

impl Default for PoolConfigFile {
    fn default() -> Self {
        // A missing "pools" key is an empty list; an explicit null creates the built-in pools.
        Self {
            pools: Some(Vec::new()),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PoolDefinitionFile {
    id: i32,
    favorite_id: Option<i32>,
    default_flag: bool,
    chart_ids: Option<Vec<i32>>,
    #[serde(deserialize_with = "json::enum_or_none")]
    category: Option<PoolCategory>,
    size_limit: Option<i32>,
    rounds_per_stay: Option<i32>,
    order: i32,
    submission_open: bool,
}

impl PoolDefinitionFile {
    fn normalize(self) -> PoolSnapshot {
        PoolSnapshot {
            id: self.id,
            favorite_id: self.favorite_id,
            default_flag: self.default_flag,
            chart_ids: distinct(&self.chart_ids.unwrap_or_default()),
            category: self.category.unwrap_or(PoolCategory::Manual),
            size_limit: self.size_limit,
            rounds_per_stay: self.rounds_per_stay,
            order: self.order,
            submission_open: self.submission_open,
        }
    }
}

struct PoolFileOut<'a>(&'a [Arc<PoolSnapshot>]);

impl Serialize for PoolFileOut<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let rows: Vec<PoolRow<'_>> = self.0.iter().map(|p| PoolRow::from(p.as_ref())).collect();
        let mut s = serializer.serialize_struct("PoolConfig", 1)?;
        s.serialize_field("pools", &rows)?;
        s.end()
    }
}

#[derive(Serialize)]
struct PoolRow<'a> {
    id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    favorite_id: Option<i32>,
    default_flag: bool,
    chart_ids: &'a [i32],
    category: PoolCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rounds_per_stay: Option<i32>,
    order: i32,
    submission_open: bool,
}

impl<'a> From<&'a PoolSnapshot> for PoolRow<'a> {
    fn from(p: &'a PoolSnapshot) -> Self {
        Self {
            id: p.id,
            favorite_id: p.favorite_id,
            default_flag: p.default_flag,
            chart_ids: &p.chart_ids,
            category: p.category,
            size_limit: p.size_limit,
            rounds_per_stay: p.rounds_per_stay,
            order: p.order,
            submission_open: p.submission_open,
        }
    }
}

fn parse_import_rows(document: &str) -> Result<Vec<PoolSnapshot>, CatalogError> {
    const CATEGORIES: [&str; 4] = ["REGULAR", "CONFIGURED", "TB", "MANUAL"];
    let rows = document_rows(document, "pools")?;
    let mut pools = Vec::with_capacity(rows.len());
    let mut ids = HashSet::new();
    for (i, row) in rows.iter().enumerate() {
        let object = row_object("pools", i, row)?;
        if !has_value(object, "id") {
            return Err(bad_row("pools", i, "缺少 id"));
        }
        check_enum(object, "category", &CATEGORIES, true, |raw| {
            format!("未知类别 {raw}")
        })
        .map_err(|reason| bad_row("pools", i, reason))?;
        let parsed = PoolDefinitionFile::deserialize(row).map_err(|e| bad_row("pools", i, e))?;
        if !ids.insert(parsed.id) {
            return Err(bad_row("pools", i, format!("谱池 id {} 重复", parsed.id)));
        }
        pools.push(parsed.normalize());
    }
    Ok(pools)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_rows_are_validated() {
        let err = |doc: &str| parse_import_rows(doc).unwrap_err().message();
        assert_eq!(
            err(r#"{"pools":[{"id":1},null]}"#),
            "pools[1] 无效：不是对象"
        );
        assert_eq!(
            err(r#"{"pools":[{"chart_ids":[1]}]}"#),
            "pools[0] 无效：缺少 id"
        );
        assert_eq!(err(r#"{"pools":[{"id":null}]}"#), "pools[0] 无效：缺少 id");
        assert_eq!(
            err(r#"{"pools":[{"id":1,"category":"regular"}]}"#),
            "pools[0] 无效：未知类别 \"regular\""
        );
        assert_eq!(
            err(r#"{"pools":[{"id":1,"category":true}]}"#),
            "pools[0] 无效：未知类别 true"
        );
        assert_eq!(
            err(r#"{"pools":[{"id":1},{"id":1}]}"#),
            "pools[1] 无效：谱池 id 1 重复"
        );
        assert!(err(r#"{"pools":[{"id":1,"chart_ids":["x"]}]}"#).starts_with("pools[0] 无效："));
        let ok = parse_import_rows(
            r#"{"pools":[{"id":3,"chart_ids":[5,5,6],"category":null},{"id":4,"category":"TB"}]}"#,
        )
        .unwrap();
        assert_eq!(ok[0].chart_ids, vec![5, 6]);
        assert_eq!(ok[0].category, PoolCategory::Manual);
        assert_eq!(ok[1].category, PoolCategory::Tb);
    }

    #[test]
    fn next_order_saturates_and_next_id_wraps() {
        let pool = |id, order| {
            let mut p = PoolSnapshot::new(id, false, vec![]);
            p.order = order;
            Arc::new(p)
        };
        assert_eq!(next_order(&[]), 10);
        assert_eq!(next_pool_id(&[]), 1);
        assert_eq!(next_order(&[pool(3, 7), pool(1, 25)]), 35);
        assert_eq!(next_pool_id(&[pool(3, 7), pool(1, 25)]), 4);
        assert_eq!(next_order(&[pool(1, i32::MAX - 3)]), i32::MAX);
        assert_eq!(next_pool_id(&[pool(i32::MAX, 0)]), i32::MIN);
    }
}
