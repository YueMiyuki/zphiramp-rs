use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zphira_common::gson::{self, GsonError, JsonElement};

use crate::error::LedgerError;
use crate::import::{
    BackupFn, ImportError, ImportMode, ImportReport, ImportResult, Kind, gson_reason,
    parse_document,
};
use crate::persist::{Document, Loaded, Persisted, read_file};
use crate::writer::{self, SAVE_INTERVAL, WriterHandle};

const FILE_NAME: &str = "round-records.json";
const WHAT: &str = "round records";

pub const MAX_RECORDS: usize = 1000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoundRecord {
    pub id: String,
    pub room_id: String,
    pub chart_id: i32,
    pub chart_name: Option<String>,
    pub started_at: i64,
    pub finished_at: i64,
    pub results: Vec<PlayerResult>,
}

/// `accuracy` and `std` are doubles widened from the game's `f32` values: pass `f64::from`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerResult {
    pub player_id: i32,
    pub player_name: String,
    pub rank: i32,
    pub score: i32,
    pub accuracy: f64,
    pub std: f64,
    pub gained_points: i32,
    pub total_points: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerRound {
    pub record_id: String,
    pub room_id: String,
    pub chart_id: i32,
    pub chart_name: Option<String>,
    pub finished_at: i64,
    pub result: PlayerResult,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordView<'a> {
    pub id: &'a str,
    pub room_id: &'a str,
    pub chart_id: i32,
    pub chart_name: Option<&'a str>,
    pub started_at: i64,
    pub finished_at: i64,
    pub results: Vec<ResultView<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultView<'a> {
    pub player_id: i32,
    pub player_name: &'a str,
    pub rank: i32,
    pub score: i32,
    pub accuracy: f64,
    pub std: f64,
    pub gained_points: i32,
    pub total_points: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRoundView<'a> {
    pub record_id: &'a str,
    pub room_id: &'a str,
    pub chart_id: i32,
    pub chart_name: Option<&'a str>,
    pub finished_at: i64,
    pub result: ResultView<'a>,
}

impl PlayerResult {
    pub fn api_view(&self) -> ResultView<'_> {
        ResultView {
            player_id: self.player_id,
            player_name: &self.player_name,
            rank: self.rank,
            score: self.score,
            accuracy: self.accuracy,
            std: self.std,
            gained_points: self.gained_points,
            total_points: self.total_points,
        }
    }
}

impl RoundRecord {
    pub fn api_view(&self) -> RecordView<'_> {
        RecordView {
            id: &self.id,
            room_id: &self.room_id,
            chart_id: self.chart_id,
            chart_name: self.chart_name.as_deref(),
            started_at: self.started_at,
            finished_at: self.finished_at,
            results: self.results.iter().map(PlayerResult::api_view).collect(),
        }
    }

    // Gson cannot write NaN or infinity, which would fail every later write of the history.
    fn make_finite(&mut self) -> bool {
        let mut changed = false;
        for result in &mut self.results {
            for value in [&mut result.accuracy, &mut result.std] {
                if !value.is_finite() {
                    *value = 0.0;
                    changed = true;
                }
            }
        }
        changed
    }
}

impl PlayerRound {
    pub fn api_view(&self) -> PlayerRoundView<'_> {
        PlayerRoundView {
            record_id: &self.record_id,
            room_id: &self.room_id,
            chart_id: self.chart_id,
            chart_name: self.chart_name.as_deref(),
            finished_at: self.finished_at,
            result: self.result.api_view(),
        }
    }
}

struct RecordsData {
    records: VecDeque<Arc<RoundRecord>>,
}

impl Document for RecordsData {
    type Content = Vec<Arc<RoundRecord>>;

    fn content(&self) -> Self::Content {
        self.records.iter().rev().cloned().collect()
    }

    fn install(&mut self, content: Self::Content) {
        self.records = content.into_iter().rev().collect();
    }

    fn serialize<W: std::io::Write>(content: &Self::Content, out: W) -> Result<(), GsonError> {
        let rows: Vec<&RoundRecord> = content.iter().map(|r| &**r).collect();
        gson::to_writer_pretty(out, &rows)
    }

    fn rows(content: &Self::Content) -> usize {
        content.len()
    }
}

struct Inner {
    store: Persisted<RecordsData>,
    sequence: AtomicI32,
}

#[derive(Clone)]
pub struct RoundRecords(Arc<Inner>);

impl std::fmt::Debug for RoundRecords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoundRecords")
            .field("path", &self.0.store.path())
            .finish()
    }
}

impl RoundRecords {
    pub fn load(data_dir: impl AsRef<Path>) -> Result<Self, LedgerError> {
        let path = data_dir.as_ref().join(FILE_NAME);
        let mut rows = match read_file::<Vec<RoundRecord>>(WHAT, &path)? {
            Loaded::Value(rows) => rows,
            Loaded::Missing => Vec::new(),
            Loaded::Empty => {
                tracing::warn!(
                    "{} is empty, starting with no round records",
                    path.display()
                );
                Vec::new()
            }
        };
        if rows.len() > MAX_RECORDS {
            tracing::warn!(
                "{} holds {} round records, keeping the newest {MAX_RECORDS}",
                path.display(),
                rows.len()
            );
            rows.drain(..rows.len() - MAX_RECORDS);
        }
        for row in &mut rows {
            if row.make_finite() {
                tracing::warn!(
                    "Round record {} held a non-finite accuracy or deviation, stored as 0",
                    row.id
                );
            }
        }
        let records = rows.into_iter().rev().map(Arc::new).collect();
        Ok(Self(Arc::new(Inner {
            store: Persisted::new(
                path,
                WHAT,
                "Failed to save round records",
                RecordsData { records },
            ),
            sequence: AtomicI32::new(0),
        })))
    }

    pub fn path(&self) -> &Path {
        self.0.store.path()
    }

    pub fn next_id(&self, room_id: &str, finished_at: i64) -> String {
        let n = self
            .0
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        format!("{room_id}-{finished_at}-{n}")
    }

    pub fn append(&self, mut record: RoundRecord) {
        if record.make_finite() {
            tracing::warn!(
                "Round record {} had a non-finite accuracy or deviation, stored as 0",
                record.id
            );
        }
        let record = Arc::new(record);
        self.0.store.mutate(|data| {
            data.records.push_front(record);
            data.records.truncate(MAX_RECORDS);
        });
    }

    /// A `limit` of 0 still returns one row.
    pub fn query(
        &self,
        limit: usize,
        mut offset: usize,
        room_id: Option<&str>,
        player_id: Option<i32>,
    ) -> Vec<Arc<RoundRecord>> {
        let mut matched = Vec::new();
        for record in self.snapshot() {
            if room_id.is_some_and(|room| room != record.room_id) {
                continue;
            }
            if player_id.is_some_and(|id| record.results.iter().all(|r| r.player_id != id)) {
                continue;
            }
            if offset > 0 {
                offset -= 1;
                continue;
            }
            matched.push(record);
            if matched.len() >= limit {
                break;
            }
        }
        matched
    }

    pub fn find(&self, id: &str) -> Option<Arc<RoundRecord>> {
        self.0
            .store
            .with(|data| data.records.iter().find(|record| record.id == id).cloned())
    }

    pub fn by_player(&self, player_id: i32, limit: usize) -> Vec<PlayerRound> {
        let mut rounds = Vec::new();
        for record in self.snapshot() {
            if let Some(result) = record.results.iter().find(|r| r.player_id == player_id) {
                rounds.push(PlayerRound {
                    record_id: record.id.clone(),
                    room_id: record.room_id.clone(),
                    chart_id: record.chart_id,
                    chart_name: record.chart_name.clone(),
                    finished_at: record.finished_at,
                    result: result.clone(),
                });
            }
            if rounds.len() >= limit {
                break;
            }
        }
        rounds
    }

    pub fn len(&self) -> usize {
        self.0.store.with(|data| data.records.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn snapshot(&self) -> Vec<Arc<RoundRecord>> {
        self.0
            .store
            .with(|data| data.records.iter().cloned().collect())
    }

    pub fn flush(&self) -> Result<(), LedgerError> {
        self.0.store.flush()
    }

    pub fn spawn_writer(&self) -> WriterHandle {
        self.spawn_writer_every(SAVE_INTERVAL)
    }

    pub fn spawn_writer_every(&self, interval: Duration) -> WriterHandle {
        let this = self.clone();
        writer::spawn("round records", interval, Arc::new(move || this.flush()))
    }

    pub fn export(&self) -> Result<String, LedgerError> {
        self.0.store.export()
    }

    pub fn import(&self, document: &str, mode: ImportMode) -> Result<ImportReport, ImportError> {
        let root = parse_document(Kind::Records, document)?;
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
        let root = parse_document(Kind::Records, document)?;
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
    current: Option<&Vec<Arc<RoundRecord>>>,
    incoming: &[Arc<RoundRecord>],
) -> Vec<Arc<RoundRecord>> {
    let mut history = incoming.to_vec();
    if let (ImportMode::Merge, Some(current)) = (mode, current) {
        let mut seen: HashSet<&str> = incoming.iter().map(|r| r.id.as_str()).collect();
        history.extend(
            current
                .iter()
                .filter(|record| seen.insert(record.id.as_str()))
                .cloned(),
        );
        // Stable: equal times keep document-then-held order.
        history.sort_by_key(|record| record.finished_at);
    }
    if history.len() > MAX_RECORDS {
        history.drain(..history.len() - MAX_RECORDS);
    }
    history
}

fn parse_rows(root: &JsonElement) -> Result<Vec<Arc<RoundRecord>>, ImportError> {
    let rows = root
        .as_array()
        .expect("parse_document checked the container");
    let mut seen = HashSet::new();
    let mut history = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let bad = |reason: String| {
            let id = row
                .as_object()
                .and_then(|o| o.get("id"))
                .and_then(JsonElement::get_as_string);
            let which = match id {
                Some(id) => format!("比赛记录第 {} 项（id \"{id}\"）", index + 1),
                None => format!("比赛记录第 {} 项", index + 1),
            };
            ImportError::invalid(format!("{which}无效：{reason}"))
        };
        let record = parse_record(row).map_err(bad)?;
        if !seen.insert(record.id.clone()) {
            return Err(bad("id 重复".to_owned()));
        }
        history.push(Arc::new(record));
    }
    Ok(history)
}

const RECORD_NUMBERS: [&str; 3] = ["chart_id", "started_at", "finished_at"];
const RESULT_NUMBERS: [&str; 6] = [
    "rank",
    "score",
    "accuracy",
    "std",
    "gained_points",
    "total_points",
];

fn present<'a>(object: &'a gson::JsonObject, key: &str) -> Option<&'a JsonElement> {
    object.get(key).filter(|value| !value.is_null())
}

fn parse_record(row: &JsonElement) -> Result<RoundRecord, String> {
    let Some(object) = row.as_object() else {
        return Err("必须是对象".to_owned());
    };
    for key in ["id", "room_id"] {
        if present(object, key).is_none() {
            return Err(format!("缺少 {key}"));
        }
    }
    for key in RECORD_NUMBERS {
        if object.get(key).is_some_and(JsonElement::is_null) {
            return Err(format!("{key} 不能为 null"));
        }
    }
    let Some(results) = present(object, "results") else {
        return Err("缺少 results".to_owned());
    };
    let Some(results) = results.as_array() else {
        return Err("results 必须是数组".to_owned());
    };
    for (index, result) in results.iter().enumerate() {
        check_result(result).map_err(|reason| format!("成绩第 {} 项无效：{reason}", index + 1))?;
    }

    let record: RoundRecord = gson::from_element(row).map_err(|e| gson_reason(&e))?;
    if record.id.is_empty() {
        return Err("id 不能为空".to_owned());
    }
    for (index, result) in record.results.iter().enumerate() {
        for (key, value) in [("accuracy", result.accuracy), ("std", result.std)] {
            if !value.is_finite() {
                return Err(format!("成绩第 {} 项无效：{key} 必须是有限数", index + 1));
            }
        }
    }
    Ok(record)
}

fn check_result(result: &JsonElement) -> Result<(), String> {
    let Some(object) = result.as_object() else {
        return Err("必须是对象".to_owned());
    };
    if present(object, "player_id").is_none() {
        return Err("缺少 player_id".to_owned());
    }
    for key in RESULT_NUMBERS {
        if object.get(key).is_some_and(JsonElement::is_null) {
            return Err(format!("{key} 不能为 null"));
        }
    }
    Ok(())
}
