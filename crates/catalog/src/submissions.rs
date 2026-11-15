use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use zphira_common::time::JavaOffsetDateTime;
use zphira_common::{fs, gson};

use crate::config::{CatalogConfig, FileSink};
use crate::error::CatalogError;
use crate::persist::{SUBMISSIONS_FILE, remove_stale, run_blocking, write_json_blocking};
use crate::pool::ChartPool;
use crate::transfer::{
    BackupFn, ImportMode, ImportResult, bad_row, check_enum, document_rows, has_value, row_object,
};

pub const MAX_SUBMITTERS: usize = 200;
pub const DEFAULT_MAX_PENDING_SUBMISSIONS: u32 = 50;

pub const REASON_ALREADY_IN_POOL: &str = "已经在该谱池里了";
pub const REASON_ALREADY_SUBMITTED: &str = "你已经投过这张谱面";
pub const REASON_SUBMITTERS_FULL: &str = "这张谱面的投稿人数已达上限";
pub fn pending_limit_reason(limit: u32) -> String {
    format!("待审投稿已达上限（{limit}），请等待审核后再投")
}

const SAVE_FAILED: &str = "Failed to save submissions";
const EMPTY_DOCUMENT: &str = "{\"submissions\":[]}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubmissionStatus {
    Pending,
    Approved,
    Rejected,
}

impl SubmissionStatus {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Approved => "APPROVED",
            Self::Rejected => "REJECTED",
        }
    }

    pub fn parse_ignore_case(raw: &str) -> Option<Self> {
        let upper = raw.trim_matches(|c: char| c <= ' ').to_uppercase();
        [Self::Pending, Self::Approved, Self::Rejected]
            .into_iter()
            .find(|s| s.name() == upper)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Submitter {
    pub user_id: i32,
    pub name: Option<String>,
    pub at: Option<JavaOffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionView {
    pub pool_id: i32,
    pub chart_id: i32,
    pub status: SubmissionStatus,
    pub submitters: Vec<Submitter>,
    pub created_at: Option<JavaOffsetDateTime>,
    pub reviewed_at: Option<JavaOffsetDateTime>,
    pub reviewer_id: Option<i32>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitResult {
    pub chart_id: i32,
    pub accepted: bool,
    pub reason: Option<String>,
}

impl SubmitResult {
    fn accepted(chart_id: i32) -> Self {
        Self {
            chart_id,
            accepted: true,
            reason: None,
        }
    }

    fn refused(chart_id: i32, reason: &str) -> Self {
        Self {
            chart_id,
            accepted: false,
            reason: Some(reason.to_owned()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ApproveOutcome {
    NotFound,
    Approved,
    AddFailed(CatalogError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Submission {
    pool_id: i32,
    chart_id: i32,
    #[serde(deserialize_with = "gson::enum_or_none")]
    status: Option<SubmissionStatus>,
    submitters: Vec<Submitter>,
    created_at: Option<JavaOffsetDateTime>,
    reviewed_at: Option<JavaOffsetDateTime>,
    reviewer_id: Option<i32>,
    reason: Option<String>,
}

impl Default for Submission {
    fn default() -> Self {
        Self {
            pool_id: 0,
            chart_id: 0,
            status: Some(SubmissionStatus::Pending),
            submitters: Vec::new(),
            created_at: None,
            reviewed_at: None,
            reviewer_id: None,
            reason: None,
        }
    }
}

impl Submission {
    fn has_submitted(&self, user_id: i32) -> bool {
        self.submitters.iter().any(|s| s.user_id == user_id)
    }

    fn is_pending_for(&self, user_id: i32) -> bool {
        self.status == Some(SubmissionStatus::Pending) && self.has_submitted(user_id)
    }

    fn view(&self) -> SubmissionView {
        SubmissionView {
            pool_id: self.pool_id,
            chart_id: self.chart_id,
            status: self.status.unwrap_or(SubmissionStatus::Pending),
            submitters: self.submitters.clone(),
            created_at: self.created_at,
            reviewed_at: self.reviewed_at,
            reviewer_id: self.reviewer_id,
            reason: self.reason.clone(),
        }
    }
}

type Key = (i32, i32);
type Entries = BTreeMap<Key, Arc<Submission>>;

#[derive(Deserialize)]
#[serde(default)]
struct StoreFile {
    submissions: Option<Vec<Option<Submission>>>,
}

impl Default for StoreFile {
    fn default() -> Self {
        Self {
            submissions: Some(Vec::new()),
        }
    }
}

struct StoreOut<'a>(&'a Entries);

impl Serialize for StoreOut<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let rows: Vec<&Submission> = self.0.values().map(Arc::as_ref).collect();
        let mut s = serializer.serialize_struct("Store", 1)?;
        s.serialize_field("submissions", &rows)?;
        s.end()
    }
}

struct State {
    entries: Entries,
    file_exists: bool,
}

#[derive(Clone)]
pub struct Submissions {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    files: Arc<dyn FileSink>,
    max_pending: Option<u32>,
    write_lock: Mutex<()>,
    state: ArcSwap<State>,
}

impl Submissions {
    pub async fn load(config: &CatalogConfig) -> Submissions {
        let blocking_config = config.clone();
        let read = run_blocking(move || {
            remove_stale(&blocking_config, SUBMISSIONS_FILE);
            Ok(fs::read_to_string_if_exists(
                blocking_config.path(SUBMISSIONS_FILE),
            ))
        })
        .await
        .and_then(|r| r);
        let (entries, file_exists) = match read {
            Ok(None) => (Entries::new(), false),
            Ok(Some(text)) => match parse_store(&text) {
                Ok((entries, normalized)) => {
                    for (pool_id, chart_id) in normalized {
                        tracing::warn!(
                            "Submission {}/{} has an unknown status, treating it as PENDING",
                            pool_id,
                            chart_id
                        );
                    }
                    (entries, true)
                }
                Err(message) => {
                    tracing::warn!(
                        "Submission store is unreadable ({}), starting empty",
                        message
                    );
                    (Entries::new(), true)
                }
            },
            Err(e) => {
                tracing::warn!("Submission store is unreadable ({}), starting empty", e);
                (Entries::new(), true)
            }
        };
        Submissions {
            inner: Arc::new(Inner {
                path: config.path(SUBMISSIONS_FILE),
                files: config.files.clone(),
                max_pending: config.max_pending_submissions,
                write_lock: Mutex::new(()),
                state: ArcSwap::from_pointee(State {
                    entries,
                    file_exists,
                }),
            }),
        }
    }

    pub fn list_by_pool(
        &self,
        pool_id: i32,
        status: Option<SubmissionStatus>,
    ) -> Vec<SubmissionView> {
        let state = self.inner.state.load();
        let mut views: Vec<SubmissionView> = state
            .entries
            .values()
            .filter(|e| e.pool_id == pool_id)
            .filter(|e| status.is_none() || e.status == status)
            .map(|e| e.view())
            .collect();
        views.sort_by_key(|v| v.chart_id);
        views
    }

    pub fn list_pending(&self) -> Vec<SubmissionView> {
        self.list_newest_first(|e| e.status == Some(SubmissionStatus::Pending))
    }

    pub fn list_by_submitter(&self, user_id: i32) -> Vec<SubmissionView> {
        self.list_newest_first(|e| e.has_submitted(user_id))
    }

    pub fn pending_count(&self, user_id: i32) -> usize {
        self.inner
            .state
            .load()
            .entries
            .values()
            .filter(|e| e.is_pending_for(user_id))
            .count()
    }

    pub fn contains(&self, pool_id: i32, chart_id: i32) -> bool {
        self.inner
            .state
            .load()
            .entries
            .contains_key(&(pool_id, chart_id))
    }

    fn list_newest_first(&self, keep: impl Fn(&Submission) -> bool) -> Vec<SubmissionView> {
        let state = self.inner.state.load();
        let mut entries: Vec<&Arc<Submission>> =
            state.entries.values().filter(|e| keep(e)).collect();
        // Newest first; entries without a creation time come first.
        entries.sort_by(|a, b| match (&a.created_at, &b.created_at) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => y.cmp(x),
        });
        entries.into_iter().map(|e| e.view()).collect()
    }

    pub async fn submit(
        &self,
        pool_id: i32,
        chart_ids: &[i32],
        user_id: i32,
        name: Option<&str>,
    ) -> Result<Vec<SubmitResult>, CatalogError> {
        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.state.load_full();
        let mut entries = current.entries.clone();
        let mut pending = entries
            .values()
            .filter(|e| e.is_pending_for(user_id))
            .count();
        let mut results = Vec::with_capacity(chart_ids.len());
        let mut changed = false;
        for &chart_id in chart_ids {
            let key = (pool_id, chart_id);
            let existing = entries.get(&key);
            if let Some(entry) = existing {
                if entry.status == Some(SubmissionStatus::Approved) {
                    results.push(SubmitResult::refused(chart_id, REASON_ALREADY_IN_POOL));
                    continue;
                }
                if entry.has_submitted(user_id) {
                    results.push(SubmitResult::refused(chart_id, REASON_ALREADY_SUBMITTED));
                    continue;
                }
                if entry.submitters.len() >= MAX_SUBMITTERS {
                    results.push(SubmitResult::refused(chart_id, REASON_SUBMITTERS_FULL));
                    continue;
                }
            }
            if let Some(limit) = self.inner.max_pending
                && pending >= limit as usize
            {
                results.push(SubmitResult::refused(
                    chart_id,
                    &pending_limit_reason(limit),
                ));
                continue;
            }
            let mut entry = match existing {
                Some(entry) => (**entry).clone(),
                None => Submission {
                    pool_id,
                    chart_id,
                    created_at: Some(JavaOffsetDateTime::now()),
                    ..Submission::default()
                },
            };
            entry.submitters.push(Submitter {
                user_id,
                name: name.map(str::to_owned),
                at: Some(JavaOffsetDateTime::now()),
            });
            if entry.status != Some(SubmissionStatus::Pending) {
                entry.reviewed_at = None;
                entry.reviewer_id = None;
                entry.reason = None;
            }
            entry.status = Some(SubmissionStatus::Pending);
            entries.insert(key, Arc::new(entry));
            pending += 1;
            results.push(SubmitResult::accepted(chart_id));
            changed = true;
        }
        if changed {
            self.commit(entries)
                .await
                .map_err(|e| CatalogError::save_failed(SAVE_FAILED, e))?;
        }
        Ok(results)
    }

    pub async fn withdraw(
        &self,
        pool_id: i32,
        chart_id: i32,
        user_id: i32,
    ) -> Result<bool, CatalogError> {
        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.state.load_full();
        let key = (pool_id, chart_id);
        let Some(entry) = current.entries.get(&key) else {
            return Ok(false);
        };
        if entry.status == Some(SubmissionStatus::Approved) || !entry.has_submitted(user_id) {
            return Ok(false);
        }
        let mut entries = current.entries.clone();
        let mut entry = (**entry).clone();
        entry.submitters.retain(|s| s.user_id != user_id);
        if entry.submitters.is_empty() {
            entries.remove(&key);
        } else {
            entries.insert(key, Arc::new(entry));
        }
        self.commit(entries)
            .await
            .map_err(|e| CatalogError::save_failed(SAVE_FAILED, e))?;
        Ok(true)
    }

    /// Does not add the chart to the pool; approve through `approve`.
    pub async fn review(
        &self,
        pool_id: i32,
        chart_id: i32,
        approve: bool,
        reviewer_id: i32,
        reason: Option<&str>,
    ) -> Result<bool, CatalogError> {
        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.state.load_full();
        let key = (pool_id, chart_id);
        let Some(entry) = current.entries.get(&key) else {
            return Ok(false);
        };
        let mut entry = (**entry).clone();
        entry.status = Some(if approve {
            SubmissionStatus::Approved
        } else {
            SubmissionStatus::Rejected
        });
        entry.reviewed_at = Some(JavaOffsetDateTime::now());
        entry.reviewer_id = Some(reviewer_id);
        entry.reason = reason.map(str::to_owned);
        let mut entries = current.entries.clone();
        entries.insert(key, Arc::new(entry));
        self.commit(entries)
            .await
            .map_err(|e| CatalogError::save_failed(SAVE_FAILED, e))?;
        Ok(true)
    }

    /// Adds the chart to the pool first and marks the submission approved only if that worked.
    pub async fn approve(
        &self,
        pool: &ChartPool,
        pool_id: i32,
        chart_id: i32,
        reviewer_id: i32,
    ) -> Result<ApproveOutcome, CatalogError> {
        if !self.contains(pool_id, chart_id) {
            return Ok(ApproveOutcome::NotFound);
        }
        if let Err(e) = pool.add_chart(pool_id, chart_id).await {
            return Ok(ApproveOutcome::AddFailed(e));
        }
        if self
            .review(pool_id, chart_id, true, reviewer_id, None)
            .await?
        {
            Ok(ApproveOutcome::Approved)
        } else {
            Ok(ApproveOutcome::NotFound)
        }
    }

    pub fn export_json(&self) -> Result<String, CatalogError> {
        let state = self.inner.state.load_full();
        if !state.file_exists {
            return Ok(EMPTY_DOCUMENT.to_owned());
        }
        gson::to_string_pretty(&StoreOut(&state.entries)).map_err(CatalogError::Json)
    }

    pub async fn import_json(
        &self,
        document: &str,
        mode: ImportMode,
        backup: impl BackupFn,
    ) -> Result<ImportResult, CatalogError> {
        let incoming = parse_import_rows(document)?;
        let _guard = self.inner.write_lock.lock().await;
        let current = self.inner.state.load_full();
        let mut entries = match mode {
            ImportMode::Replace => Entries::new(),
            ImportMode::Merge => current.entries.clone(),
        };
        for row in incoming {
            entries.insert((row.pool_id, row.chart_id), Arc::new(row));
        }
        let path = self.inner.path.clone();
        let backup_file = run_blocking(move || backup(&path))
            .await
            .map_err(CatalogError::io)?;
        let count = i32::try_from(entries.len()).unwrap_or(i32::MAX);
        self.commit(entries).await.map_err(CatalogError::io)?;
        Ok(ImportResult {
            imported: count,
            total: count,
            backup_file,
        })
    }

    pub fn file_path(&self) -> &Path {
        &self.inner.path
    }

    async fn commit(&self, entries: Entries) -> std::io::Result<()> {
        let files = self.inner.files.clone();
        let path = self.inner.path.clone();
        let entries = Arc::new(entries);
        let to_write = entries.clone();
        run_blocking(move || write_json_blocking(&*files, &path, &StoreOut(&to_write))).await?;
        let entries = Arc::try_unwrap(entries).unwrap_or_else(|shared| (*shared).clone());
        self.inner.state.store(Arc::new(State {
            entries,
            file_exists: true,
        }));
        Ok(())
    }
}

fn parse_store(text: &str) -> Result<(Entries, Vec<Key>), String> {
    let store = gson::from_str_opt::<StoreFile>(text).map_err(|e| e.message().to_owned())?;
    let mut entries = Entries::new();
    let mut normalized = Vec::new();
    if let Some(StoreFile {
        submissions: Some(rows),
    }) = store
    {
        for row in rows {
            let Some(mut row) = row else {
                // Fixed log text for a null row.
                return Err("Cannot read field \"poolId\" because \"entry\" is null".to_owned());
            };
            if row.status.is_none() {
                row.status = Some(SubmissionStatus::Pending);
                normalized.push((row.pool_id, row.chart_id));
            }
            entries.insert((row.pool_id, row.chart_id), Arc::new(row));
        }
    }
    Ok((entries, normalized))
}

fn parse_import_rows(document: &str) -> Result<Vec<Submission>, CatalogError> {
    const STATUSES: [&str; 3] = ["PENDING", "APPROVED", "REJECTED"];
    const FIELD: &str = "submissions";
    let rows = document_rows(document, FIELD)?;
    let mut parsed = Vec::with_capacity(rows.len());
    let mut keys = HashSet::new();
    for (i, row) in rows.iter().enumerate() {
        let object = row_object(FIELD, i, row)?;
        for key in ["pool_id", "chart_id"] {
            if !has_value(object, key) {
                return Err(bad_row(FIELD, i, format!("缺少 {key}")));
            }
        }
        check_enum(object, "status", &STATUSES, false, |raw| {
            format!("未知状态 {raw}")
        })
        .map_err(|reason| bad_row(FIELD, i, reason))?;
        if let Some(submitters) = object.get("submitters").and_then(|v| v.as_array()) {
            for (j, submitter) in submitters.iter().enumerate() {
                match submitter.as_object() {
                    None => return Err(bad_row(FIELD, i, format!("submitters[{j}] 不是对象"))),
                    Some(s) if !has_value(s, "user_id") => {
                        return Err(bad_row(FIELD, i, format!("submitters[{j}] 缺少 user_id")));
                    }
                    Some(_) => {}
                }
            }
        }
        let submission: Submission =
            gson::from_element(row).map_err(|e| bad_row(FIELD, i, e.message()))?;
        if !keys.insert((submission.pool_id, submission.chart_id)) {
            return Err(bad_row(
                FIELD,
                i,
                format!(
                    "池 {} 谱面 {} 重复",
                    submission.pool_id, submission.chart_id
                ),
            ));
        }
        parsed.push(submission);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_rows_are_validated() {
        let err = |doc: &str| parse_import_rows(doc).unwrap_err().message();
        assert_eq!(
            err(r#"{"submissions":[1]}"#),
            "submissions[0] 无效：不是对象"
        );
        assert_eq!(
            err(r#"{"submissions":[{"chart_id":1}]}"#),
            "submissions[0] 无效：缺少 pool_id"
        );
        assert_eq!(
            err(r#"{"submissions":[{"pool_id":1}]}"#),
            "submissions[0] 无效：缺少 chart_id"
        );
        assert_eq!(
            err(r#"{"submissions":[{"pool_id":1,"chart_id":2,"status":"pending"}]}"#),
            "submissions[0] 无效：未知状态 \"pending\""
        );
        assert_eq!(
            err(r#"{"submissions":[{"pool_id":1,"chart_id":2,"status":null}]}"#),
            "submissions[0] 无效：未知状态 null"
        );
        assert_eq!(
            err(r#"{"submissions":[{"pool_id":1,"chart_id":2,"submitters":[null]}]}"#),
            "submissions[0] 无效：submitters[0] 不是对象"
        );
        assert_eq!(
            err(r#"{"submissions":[{"pool_id":1,"chart_id":2,"submitters":[{"name":"x"}]}]}"#),
            "submissions[0] 无效：submitters[0] 缺少 user_id"
        );
        assert_eq!(
            err(r#"{"submissions":[{"pool_id":1,"chart_id":2},{"pool_id":"1","chart_id":2}]}"#),
            "submissions[1] 无效：池 1 谱面 2 重复"
        );
        assert!(
            err(r#"{"submissions":[{"pool_id":1,"chart_id":2,"created_at":"yesterday"}]}"#)
                .starts_with("submissions[0] 无效：")
        );
        let ok = parse_import_rows(
            r#"{"submissions":[{"pool_id":1,"chart_id":2,"submitters":null},{"pool_id":1,"chart_id":3,"status":"APPROVED"}]}"#,
        )
        .unwrap();
        assert_eq!(ok[0].status, Some(SubmissionStatus::Pending));
        assert!(ok[0].submitters.is_empty());
        assert_eq!(ok[1].status, Some(SubmissionStatus::Approved));
    }

    #[test]
    fn status_parsing() {
        assert_eq!(
            SubmissionStatus::parse_ignore_case(" pending "),
            Some(SubmissionStatus::Pending)
        );
        assert_eq!(
            SubmissionStatus::parse_ignore_case("Rejected"),
            Some(SubmissionStatus::Rejected)
        );
        assert_eq!(SubmissionStatus::parse_ignore_case("nope"), None);
    }
}
