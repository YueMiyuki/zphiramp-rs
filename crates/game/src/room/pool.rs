use std::sync::Arc;

use zphira_catalog::{ChartPool, PoolSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rotation {
    pub current_pool_id: Option<i32>,
    pub pending_pool_id: Option<i32>,
    pub finished_rounds: i32,
}

#[derive(Debug, Clone)]
struct Entry {
    id: i32,
    snapshot: Option<Arc<PoolSnapshot>>,
}

#[derive(Clone)]
pub struct RoomChartPool {
    catalog: ChartPool,
    entries: Vec<Entry>,
    current: i32,
    pending: Option<i32>,
    finished: i32,
    favorite_override: Option<Option<i32>>,
}

impl std::fmt::Debug for RoomChartPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoomChartPool")
            .field("pool_ids", &self.pool_ids())
            .field("current", &self.current)
            .field("pending", &self.pending)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

pub const EMPTY_POOL_LIST: &str = "Room chart pool list cannot be empty";

impl RoomChartPool {
    pub fn new(catalog: ChartPool, pools: &[Arc<PoolSnapshot>]) -> Result<Self, &'static str> {
        let entries: Vec<Entry> = pools
            .iter()
            .filter(|p| !p.chart_ids.is_empty())
            .map(|p| Entry {
                id: p.id,
                snapshot: Some(p.clone()),
            })
            .collect();
        let Some(first) = entries.first() else {
            return Err(EMPTY_POOL_LIST);
        };
        let current = first.id;
        Ok(Self {
            catalog,
            entries,
            current,
            pending: None,
            finished: 0,
            favorite_override: None,
        })
    }

    /// Keeps every pool id, also pools that are empty now; at least one must be usable.
    pub fn restore(catalog: ChartPool, pool_ids: &[i32]) -> Result<Self, &'static str> {
        let mut entries: Vec<Entry> = Vec::new();
        for &id in pool_ids {
            if entries.iter().any(|e| e.id == id) {
                continue;
            }
            let snapshot = usable_live(&catalog, id);
            entries.push(Entry { id, snapshot });
        }
        let Some(current) = entries.iter().find(|e| e.snapshot.is_some()).map(|e| e.id) else {
            return Err(EMPTY_POOL_LIST);
        };
        Ok(Self {
            catalog,
            entries,
            current,
            pending: None,
            finished: 0,
            favorite_override: None,
        })
    }

    pub fn pools(&self) -> Vec<Arc<PoolSnapshot>> {
        self.entries
            .iter()
            .filter_map(|e| {
                self.resolve(e)
                    .or_else(|| self.catalog.find_published_pool(e.id))
            })
            .collect()
    }

    pub fn pool_ids(&self) -> Vec<i32> {
        self.entries.iter().map(|e| e.id).collect()
    }

    pub fn current_pool_snapshot(&self) -> Arc<PoolSnapshot> {
        let current = self
            .entry(self.current)
            .and_then(|e| self.resolve(e))
            .unwrap_or_else(|| {
                Arc::new(PoolSnapshot {
                    id: self.current,
                    favorite_id: None,
                    default_flag: false,
                    chart_ids: Vec::new(),
                    category: zphira_catalog::PoolCategory::Manual,
                    size_limit: None,
                    rounds_per_stay: None,
                    order: 0,
                    submission_open: false,
                })
            });
        match self.favorite_override {
            None => current,
            Some(favorite) => {
                let mut copy = (*current).clone();
                copy.favorite_id = favorite;
                Arc::new(copy)
            }
        }
    }

    pub fn current_pool_id(&self) -> i32 {
        self.current
    }

    pub fn pending_pool_id(&self) -> Option<i32> {
        self.pending
    }

    pub fn finished_rounds_since_refresh(&self) -> i32 {
        self.finished
    }

    pub fn switch_pool(&mut self, pool_id: i32) -> Result<(), String> {
        if self.entry(pool_id).is_none() {
            return Err(format!("Pool not in room chart pool list: {pool_id}"));
        }
        self.pending = Some(pool_id);
        Ok(())
    }

    pub fn set_favorite(&mut self, favorite_id: Option<i32>) {
        self.favorite_override = Some(favorite_id);
    }

    pub fn finish_playing_round(&mut self, room_interval_rounds: i32) -> bool {
        self.finished = self.finished.saturating_add(1);
        if self.finished >= self.resolve_rounds_per_stay(room_interval_rounds) {
            self.refresh_current_pool();
            return true;
        }
        false
    }

    pub fn resolve_rounds_per_stay(&self, room_interval_rounds: i32) -> i32 {
        match self.current_pool_snapshot().rounds_per_stay {
            Some(quota) if quota >= 1 => quota,
            _ => room_interval_rounds,
        }
    }

    pub fn rotation_state(&self) -> Rotation {
        Rotation {
            current_pool_id: Some(self.current),
            pending_pool_id: self.pending,
            finished_rounds: self.finished,
        }
    }

    pub fn restore_rotation(&mut self, rotation: Option<Rotation>) {
        let Some(rotation) = rotation else {
            return;
        };
        let Some(current) = rotation.current_pool_id else {
            return;
        };
        let usable = self
            .entry(current)
            .is_some_and(|e| e.snapshot.is_some() || usable_live(&self.catalog, e.id).is_some());
        if !usable {
            return;
        }
        self.ensure_snapshot(current);
        self.current = current;
        self.finished = rotation.finished_rounds.max(0);
        self.pending = rotation
            .pending_pool_id
            .filter(|&p| self.entry(p).is_some());
    }

    fn refresh_current_pool(&mut self) {
        let previous = self.current;
        let finished = self.finished;
        let pending = self.pending.take();
        let target = match pending {
            Some(p) if self.is_usable(p) => Some(p),
            _ => self.next_usable_after(self.current),
        };
        if let Some(target) = target {
            self.ensure_snapshot(target);
            self.current = target;
        }
        self.finished = 0;
        self.favorite_override = None;
        tracing::info!(
            "Room chart pool rotated from {} to {} after {} round(s)",
            previous,
            self.current,
            finished
        );
    }

    fn next_usable_after(&self, id: i32) -> Option<i32> {
        let start = self.entries.iter().position(|e| e.id == id).unwrap_or(0);
        let n = self.entries.len();
        (1..=n)
            .map(|step| &self.entries[(start + step) % n])
            .find(|e| self.is_usable(e.id))
            .map(|e| e.id)
    }

    fn is_usable(&self, id: i32) -> bool {
        self.entry(id)
            .is_some_and(|e| e.snapshot.is_some() || usable_live(&self.catalog, id).is_some())
    }

    fn ensure_snapshot(&mut self, id: i32) {
        let live = usable_live(&self.catalog, id);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id)
            && entry.snapshot.is_none()
        {
            entry.snapshot = live;
        }
    }

    fn entry(&self, id: i32) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn resolve(&self, entry: &Entry) -> Option<Arc<PoolSnapshot>> {
        usable_live(&self.catalog, entry.id).or_else(|| entry.snapshot.clone())
    }
}

fn usable_live(catalog: &ChartPool, id: i32) -> Option<Arc<PoolSnapshot>> {
    let live = catalog.find_published_pool(id)?;
    if live.chart_ids.is_empty()
        || !live
            .chart_ids
            .iter()
            .all(|&c| catalog.is_chart_info_loaded(c))
    {
        return None;
    }
    Some(live)
}
