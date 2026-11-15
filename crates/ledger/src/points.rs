use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zphira_common::gson::{self, GsonError, JsonElement, JsonObject};

use crate::error::LedgerError;
use crate::import::{
    BackupFn, ImportError, ImportMode, ImportReport, ImportResult, Kind, gson_reason,
    parse_document,
};
use crate::persist::{Document, Loaded, Persisted, read_file};
use crate::util::{Clock, system_clock};
use crate::writer::{self, SAVE_INTERVAL, WriterHandle};

const FILE_NAME: &str = "player-points.json";
const WHAT: &str = "player points";
const TOP_LINES_CACHE_MILLIS: i64 = 60_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct PointData {
    name: Option<String>,
    points: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointSummary {
    pub points: i32,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankEntry {
    pub player_id: i32,
    pub name: Option<String>,
    pub points: i32,
    pub rank: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointAward {
    pub player_id: i32,
    pub name: Option<String>,
    pub gained: i32,
}

struct Ranking {
    version: u64,
    order: Vec<i32>,
    position: HashMap<i32, usize>,
}

// Gson refuses two keys that read as the same integer; serde's map impls would overwrite one.
struct Board(BTreeMap<i32, PointData>);

impl<'de> Deserialize<'de> for Board {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Rows;

        impl<'de> serde::de::Visitor<'de> for Rows {
            type Value = Board;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a map of player ids to point rows")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Board, A::Error> {
                let mut board = BTreeMap::new();
                while let Some((id, row)) = map.next_entry::<i32, PointData>()? {
                    if board.insert(id, row).is_some() {
                        return Err(serde::de::Error::custom(format!("duplicate key: {id}")));
                    }
                }
                Ok(Board(board))
            }
        }

        deserializer.deserialize_map(Rows)
    }
}

struct PointsData {
    points: BTreeMap<i32, PointData>,
    version: u64,
    ranking: Option<Ranking>,
    top_lines: Vec<String>,
    top_lines_time: i64,
}

impl PointsData {
    fn new(points: BTreeMap<i32, PointData>) -> Self {
        Self {
            points,
            version: 0,
            ranking: None,
            top_lines: Vec::new(),
            top_lines_time: 0,
        }
    }

    fn ranking(&mut self) -> &Ranking {
        let stale = self
            .ranking
            .as_ref()
            .is_none_or(|ranking| ranking.version != self.version);
        if stale {
            let mut order: Vec<i32> = self.points.keys().copied().collect();
            // Keys come out ascending, so the stable sort leaves points ties in id order.
            order.sort_by_key(|id| std::cmp::Reverse(self.points[id].points));
            let position = order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
            self.ranking = Some(Ranking {
                version: self.version,
                order,
                position,
            });
        }
        self.ranking.as_ref().expect("the ranking was just built")
    }

    fn rank(&mut self, player_id: i32) -> usize {
        let ranking = self.ranking();
        match ranking.position.get(&player_id) {
            Some(index) => index + 1,
            None => ranking.order.len() + 1,
        }
    }

    fn bump(&mut self) {
        self.version += 1;
    }

    fn touch(&mut self, player_id: i32, name: &str) {
        let changed = match self.points.get_mut(&player_id) {
            Some(row) if row.name.as_deref() == Some(name) => false,
            Some(row) => {
                row.name = Some(name.to_owned());
                true
            }
            None => {
                self.points.insert(
                    player_id,
                    PointData {
                        name: Some(name.to_owned()),
                        points: 0,
                    },
                );
                true
            }
        };
        if changed {
            self.bump();
        }
    }
}

impl Document for PointsData {
    type Content = BTreeMap<i32, PointData>;

    fn content(&self) -> Self::Content {
        self.points.clone()
    }

    fn install(&mut self, content: Self::Content) {
        self.points = content;
        self.version += 1;
    }

    fn serialize<W: std::io::Write>(content: &Self::Content, out: W) -> Result<(), GsonError> {
        gson::to_writer_pretty(out, content)
    }

    fn rows(content: &Self::Content) -> usize {
        content.len()
    }
}

struct Inner {
    store: Persisted<PointsData>,
    clock: Clock,
}

#[derive(Clone)]
pub struct Points(Arc<Inner>);

impl std::fmt::Debug for Points {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Points")
            .field("path", &self.0.store.path())
            .finish()
    }
}

impl Points {
    pub fn load(data_dir: impl AsRef<Path>) -> Result<Self, LedgerError> {
        Self::load_with_clock(data_dir, system_clock())
    }

    pub fn load_with_clock(data_dir: impl AsRef<Path>, clock: Clock) -> Result<Self, LedgerError> {
        let path = data_dir.as_ref().join(FILE_NAME);
        let points = match read_file::<Board>(WHAT, &path)? {
            Loaded::Value(Board(points)) => points,
            Loaded::Missing => BTreeMap::new(),
            Loaded::Empty => {
                tracing::warn!("{} is empty, starting with no points", path.display());
                BTreeMap::new()
            }
        };
        Ok(Self(Arc::new(Inner {
            store: Persisted::new(
                path,
                WHAT,
                "Failed to save player points",
                PointsData::new(points),
            ),
            clock,
        })))
    }

    pub fn path(&self) -> &Path {
        self.0.store.path()
    }

    pub fn summary(&self, player_id: i32, name: &str) -> PointSummary {
        self.0.store.with(|data| {
            data.touch(player_id, name);
            let points = data.points[&player_id].points;
            PointSummary {
                points,
                rank: data.rank(player_id),
            }
        })
    }

    pub fn add_points(&self, player_id: i32, name: &str, points: i32) -> i32 {
        self.0.store.mutate(|data| {
            data.touch(player_id, name);
            let row = data.points.entry(player_id).or_default();
            row.points = row.points.wrapping_add(points).max(0);
            let total = row.points;
            data.bump();
            total
        })
    }

    pub fn add_points_batch(&self, awards: &[PointAward]) -> Vec<(i32, i32)> {
        self.0.store.mutate(|data| {
            let totals = awards
                .iter()
                .map(|award| {
                    let row = data.points.entry(award.player_id).or_default();
                    if let Some(name) = &award.name {
                        row.name = Some(name.clone());
                    }
                    row.points = row.points.wrapping_add(award.gained).max(0);
                    (award.player_id, row.points)
                })
                .collect();
            data.bump();
            totals
        })
    }

    /// Kept for 60 seconds whatever limit the next caller passes.
    pub fn top_ranking_lines(&self, limit: usize) -> Vec<String> {
        let now = (self.0.clock)();
        self.0.store.with(|data| {
            if now - data.top_lines_time <= TOP_LINES_CACHE_MILLIS {
                return data.top_lines.clone();
            }
            let ids: Vec<i32> = data.ranking().order.iter().copied().take(limit).collect();
            let lines: Vec<String> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let row = &data.points[id];
                    format!(
                        "#{} {}：{} 分",
                        i + 1,
                        row.name.as_deref().unwrap_or("null"),
                        row.points
                    )
                })
                .collect();
            data.top_lines.clone_from(&lines);
            data.top_lines_time = now;
            lines
        })
    }

    pub fn ranking_snapshot(&self, limit: usize) -> Vec<RankEntry> {
        self.0.store.with(|data| {
            let ids: Vec<i32> = data.ranking().order.iter().copied().take(limit).collect();
            ids.iter()
                .enumerate()
                .map(|(i, id)| {
                    let row = &data.points[id];
                    RankEntry {
                        player_id: *id,
                        name: row.name.clone(),
                        points: row.points,
                        rank: i + 1,
                    }
                })
                .collect()
        })
    }

    /// `size + 1` for a player without a row.
    pub fn rank(&self, player_id: i32) -> usize {
        self.0.store.with(|data| data.rank(player_id))
    }

    pub fn len(&self) -> usize {
        self.0.store.with(|data| data.points.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn flush(&self) -> Result<(), LedgerError> {
        self.0.store.flush()
    }

    pub fn spawn_writer(&self) -> WriterHandle {
        self.spawn_writer_every(SAVE_INTERVAL)
    }

    pub fn spawn_writer_every(&self, interval: Duration) -> WriterHandle {
        let this = self.clone();
        writer::spawn("player points", interval, Arc::new(move || this.flush()))
    }

    pub fn export(&self) -> Result<String, LedgerError> {
        self.0.store.export()
    }

    pub fn import(&self, document: &str, mode: ImportMode) -> Result<ImportReport, ImportError> {
        let root = parse_document(Kind::Points, document)?;
        let incoming = parse_rows(&root)?;
        let rows = self.0.store.import(mode == ImportMode::Merge, |current| {
            assemble(mode, current, &incoming)
        })?;
        Ok(ImportReport::rows(rows))
    }

    /// Like `import`, but the store stays locked from the backup to the swap.
    pub fn import_json(
        &self,
        document: &str,
        mode: ImportMode,
        backup: impl BackupFn,
    ) -> Result<ImportResult, ImportError> {
        let root = parse_document(Kind::Points, document)?;
        let incoming = parse_rows(&root)?;
        let (rows, backup_file) = self
            .0
            .store
            .import_with_backup(|current| assemble(mode, current, &incoming), backup)?;
        Ok(ImportResult {
            imported: rows,
            total: rows,
            backup_file,
        })
    }
}

fn assemble(
    mode: ImportMode,
    current: Option<&BTreeMap<i32, PointData>>,
    incoming: &BTreeMap<i32, PointData>,
) -> BTreeMap<i32, PointData> {
    let mut board = match (mode, current) {
        (ImportMode::Merge, Some(current)) => current.clone(),
        _ => BTreeMap::new(),
    };
    board.extend(incoming.iter().map(|(id, row)| (*id, row.clone())));
    board
}

fn parse_rows(root: &JsonElement) -> Result<BTreeMap<i32, PointData>, ImportError> {
    let object: &JsonObject = root
        .as_object()
        .expect("parse_document checked the container");
    let mut board = BTreeMap::new();
    for (index, (key, value)) in object.iter().enumerate() {
        let bad = |reason: String| {
            ImportError::invalid(format!(
                "积分数据第 {} 项（玩家 ID \"{key}\"）无效：{reason}",
                index + 1
            ))
        };
        let player_id = parse_key(key).map_err(bad)?;
        let row = parse_row(value).map_err(bad)?;
        if board.insert(player_id, row).is_some() {
            return Err(bad(format!("玩家 ID {player_id} 重复")));
        }
    }
    Ok(board)
}

fn parse_key(key: &str) -> Result<i32, String> {
    let mut one = JsonObject::new();
    one.insert(key, JsonElement::Bool(true));
    gson::from_element::<BTreeMap<i32, serde::de::IgnoredAny>>(&JsonElement::Object(one))
        .ok()
        .and_then(|map| map.into_keys().next())
        .ok_or_else(|| "玩家 ID 必须是整数".to_owned())
}

fn parse_row(value: &JsonElement) -> Result<PointData, String> {
    let Some(object) = value.as_object() else {
        return Err("积分数据必须是对象".to_owned());
    };
    match object.get("points") {
        None | Some(JsonElement::Null) => return Err("缺少 points".to_owned()),
        Some(_) => {}
    }
    let row: PointData = gson::from_element(value).map_err(|e| gson_reason(&e))?;
    if row.points < 0 {
        return Err("points 不能为负数".to_owned());
    }
    Ok(row)
}
