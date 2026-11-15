use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use zphira_common::{fs, json};
use zphira_phira::ChartInfo;

use crate::config::{CatalogConfig, FileSink};
use crate::error::CatalogError;
use crate::persist::{
    CACHE_FILE, ChartMap, ChartMapRef, SerialWriter, remove_stale, run_blocking,
    write_json_blocking,
};

#[derive(Clone)]
pub(crate) struct ChartInfoCache {
    inner: Arc<CacheInner>,
}

struct CacheInner {
    path: PathBuf,
    files: Arc<dyn FileSink>,
    map: ArcSwap<ChartMap>,
    writer: SerialWriter,
}

impl ChartInfoCache {
    pub(crate) fn load_blocking(
        config: &CatalogConfig,
    ) -> Result<(Self, Option<String>), CatalogError> {
        remove_stale(config, CACHE_FILE);
        let path = config.path(CACHE_FILE);
        let (map, unreadable) =
            match fs::read_to_string_if_exists(&path).map_err(CatalogError::storage)? {
                None => (ChartMap::new(), None),
                Some(text) => match json::from_str_opt::<ChartMap>(&text) {
                    Ok(map) => (map.unwrap_or_default(), None),
                    Err(e) => (ChartMap::new(), Some(e.to_string())),
                },
            };
        let cache = Self {
            inner: Arc::new(CacheInner {
                path,
                files: config.files.clone(),
                map: ArcSwap::from_pointee(map),
                writer: SerialWriter::default(),
            }),
        };
        Ok((cache, unreadable))
    }

    pub(crate) fn get(&self, chart_id: i32) -> Option<Arc<ChartInfo>> {
        self.inner.map.load().get(&chart_id).cloned()
    }

    pub(crate) fn contains(&self, chart_id: i32) -> bool {
        self.inner.map.load().contains_key(&chart_id)
    }

    pub(crate) fn clear_durations(&self) -> usize {
        let map = self.inner.map.load();
        let mut cleared = 0;
        for chart in map.values() {
            if chart.duration_seconds().is_some() {
                chart.set_duration_seconds(None);
                cleared += 1;
            }
        }
        cleared
    }

    pub(crate) fn set_duration(&self, chart_id: i32, seconds: i32) -> bool {
        match self.inner.map.load().get(&chart_id) {
            Some(chart) => {
                chart.set_duration_seconds(Some(seconds));
                true
            }
            None => false,
        }
    }

    pub(crate) async fn commit(&self, entries: Vec<(i32, Arc<ChartInfo>)>) -> std::io::Result<()> {
        let inner = self.inner.clone();
        run_blocking(move || {
            inner.writer.write_now(|| {
                let current = inner.map.load_full();
                let next = if entries.iter().all(|(id, _)| current.contains_key(id)) {
                    current
                } else {
                    let mut map = (*current).clone();
                    for (id, chart) in entries {
                        map.entry(id).or_insert(chart);
                    }
                    Arc::new(map)
                };
                write_json_blocking(&*inner.files, &inner.path, &ChartMapRef(&next))?;
                // Memory changes only after the file write succeeded.
                inner.map.store(next);
                Ok(())
            })
        })
        .await
    }

    pub(crate) async fn save(&self) -> std::io::Result<()> {
        let inner = self.inner.clone();
        run_blocking(move || {
            inner.writer.save(|| {
                let current = inner.map.load_full();
                write_json_blocking(&*inner.files, &inner.path, &ChartMapRef(&current))
            })
        })
        .await
    }
}
