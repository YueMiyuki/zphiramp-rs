use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use zphira_catalog::{CatalogError, ImportMode as CatalogMode};
use zphira_common::fs;
use zphira_ledger::{ImportError, ImportMode as LedgerMode, ImportResult as LedgerResult};

use crate::state::AppState;

const KEEP_BACKUPS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataKind {
    Admins,
    Pools,
    Points,
    Records,
    Submissions,
}

impl DataKind {
    pub fn of(value: &str) -> Option<Self> {
        [
            Self::Admins,
            Self::Pools,
            Self::Points,
            Self::Records,
            Self::Submissions,
        ]
        .into_iter()
        .find(|k| k.key() == value)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Admins => "admins",
            Self::Pools => "pools",
            Self::Points => "points",
            Self::Records => "records",
            Self::Submissions => "submissions",
        }
    }

    fn file(self, state: &AppState) -> PathBuf {
        match self {
            Self::Admins => state.admins.path().to_path_buf(),
            Self::Pools => state.pools.file_path().to_path_buf(),
            Self::Points => state.points.path().to_path_buf(),
            Self::Records => state.records.path().to_path_buf(),
            Self::Submissions => state.submissions.file_path().to_path_buf(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Imported {
    pub(crate) imported: i64,
    pub(crate) total: i64,
    pub(crate) backup: String,
}

#[derive(Debug)]
pub(crate) enum TransferError {
    Invalid(String),
    Io(String),
}

pub(crate) struct DataTransfer {
    backup_dir: PathBuf,
    backup_lock: Mutex<()>,
}

impl DataTransfer {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        Self {
            backup_dir: data_dir.join("backups"),
            backup_lock: Mutex::new(()),
        }
    }

    fn backup(&self, kind: DataKind, content: &str, nothing_to_save: bool) -> io::Result<String> {
        if nothing_to_save {
            return Ok(String::new());
        }
        let _guard = self
            .backup_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        std::fs::create_dir_all(&self.backup_dir)?;
        // Never overwrite an earlier backup: wait for the next millisecond instead.
        let target = loop {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
            let target = self.backup_dir.join(format!("{}-{stamp}.json", kind.key()));
            if !target.exists() {
                break target;
            }
            std::thread::yield_now();
        };
        fs::write_atomic(&target, content.as_bytes())?;
        self.prune(kind);
        Ok(target.display().to_string())
    }

    fn prune(&self, kind: DataKind) {
        let names = match self.list(kind) {
            Ok(names) => names,
            Err(e) => {
                tracing::warn!(error = %e, "Could not list the backups of {}", kind.key());
                return;
            }
        };
        for name in names.into_iter().skip(KEEP_BACKUPS) {
            let path = self.backup_dir.join(&name);
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(error = %e, "Could not delete old backup {}", path.display());
            }
        }
    }

    fn list(&self, kind: DataKind) -> io::Result<Vec<String>> {
        let entries = match std::fs::read_dir(&self.backup_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let prefix = format!("{}-", kind.key());
        let mut names = Vec::new();
        for entry in entries {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) {
                names.push(name);
            }
        }
        names.sort_by(|a, b| b.cmp(a));
        Ok(names)
    }
}

async fn blocking<R: Send + 'static>(
    work: impl FnOnce() -> R + Send + 'static,
) -> Result<R, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("task failed: {e}"))
}

pub(crate) async fn export(state: &Arc<AppState>, kind: DataKind) -> Result<String, String> {
    let state = state.clone();
    blocking(move || match kind {
        DataKind::Admins => state.admins.export().map_err(|e| e.to_string()),
        DataKind::Pools => state.pools.export_json().map_err(|e| e.message()),
        DataKind::Points => state.points.export().map_err(|e| e.to_string()),
        DataKind::Records => state.records.export().map_err(|e| e.to_string()),
        DataKind::Submissions => state.submissions.export_json().map_err(|e| e.message()),
    })
    .await?
}

pub(crate) async fn list_backups(state: &Arc<AppState>, kind: DataKind) -> io::Result<Vec<String>> {
    let state = state.clone();
    blocking(move || state.transfer.list(kind))
        .await
        .map_err(io::Error::other)?
}

pub(crate) async fn import(
    state: &Arc<AppState>,
    kind: DataKind,
    document: String,
    merge: bool,
) -> Result<Imported, TransferError> {
    let state = state.clone();
    let runtime = tokio::runtime::Handle::current();
    let result = blocking(move || import_blocking(&state, &runtime, kind, &document, merge))
        .await
        .map_err(TransferError::Io)??;
    tracing::info!(
        "Imported {} ({} rows, merge={}), previous copy at {}",
        kind.key(),
        result.imported,
        merge,
        result.backup
    );
    Ok(result)
}

fn ledger_error(e: ImportError) -> TransferError {
    match e {
        ImportError::Invalid(message) => TransferError::Invalid(message),
        ImportError::Write(message) => TransferError::Io(message),
    }
}

fn catalog_error(e: CatalogError) -> TransferError {
    match e {
        CatalogError::Invalid(message) | CatalogError::NotFound(message) => {
            TransferError::Invalid(message)
        }
        other => TransferError::Io(other.message()),
    }
}

fn import_blocking(
    state: &Arc<AppState>,
    runtime: &tokio::runtime::Handle,
    kind: DataKind,
    document: &str,
    merge: bool,
) -> Result<Imported, TransferError> {
    let file_exists = kind.file(state).exists();
    let ledger_mode = LedgerMode::from_merge(merge);
    let catalog_mode = if merge {
        CatalogMode::Merge
    } else {
        CatalogMode::Replace
    };

    let ledger_backup = |current: &str| -> io::Result<String> {
        let empty = matches!(current.trim(), "[]" | "{}");
        state.transfer.backup(kind, current, !file_exists && empty)
    };
    let ledger = |result: Result<LedgerResult, ImportError>| -> Result<Imported, TransferError> {
        let result = result.map_err(ledger_error)?;
        Ok(Imported {
            imported: i64::try_from(result.imported).unwrap_or(i64::MAX),
            total: i64::try_from(result.total).unwrap_or(i64::MAX),
            backup: result.backup_file,
        })
    };

    match kind {
        DataKind::Admins => ledger(
            state
                .admins
                .import_json(document, ledger_mode, ledger_backup),
        ),
        DataKind::Points => ledger(
            state
                .points
                .import_json(document, ledger_mode, ledger_backup),
        ),
        DataKind::Records => ledger(state.records.import_json(
            document,
            ledger_mode,
            ledger_backup,
        )),
        DataKind::Pools => {
            let pools = state.pools.clone();
            let transfer_state = state.clone();
            let backup = move |_path: &Path| -> io::Result<String> {
                let current = pools.export_json().map_err(io::Error::other)?;
                transfer_state.transfer.backup(kind, &current, !file_exists)
            };
            let result = runtime
                .block_on(state.pools.import_json(document, catalog_mode, backup))
                .map_err(catalog_error)?;
            Ok(Imported {
                imported: i64::from(result.imported),
                total: i64::from(result.total),
                backup: result.backup_file,
            })
        }
        DataKind::Submissions => {
            let submissions = state.submissions.clone();
            let transfer_state = state.clone();
            let backup = move |path: &Path| -> io::Result<String> {
                let current = submissions.export_json().map_err(io::Error::other)?;
                transfer_state
                    .transfer
                    .backup(kind, &current, !path.exists())
            };
            let result = runtime
                .block_on(
                    state
                        .submissions
                        .import_json(document, catalog_mode, backup),
                )
                .map_err(catalog_error)?;
            Ok(Imported {
                imported: i64::from(result.imported),
                total: i64::from(result.total),
                backup: result.backup_file,
            })
        }
    }
}

pub(crate) async fn restore(
    state: &Arc<AppState>,
    kind: DataKind,
    name: &str,
) -> Result<Imported, TransferError> {
    let missing = || TransferError::Invalid("备份不存在".to_owned());
    // The kind is checked on exactly the file name that is opened.
    let plain = !name.is_empty() && !name.contains(['/', '\\', '\0']);
    if !plain || !name.starts_with(&format!("{}-", kind.key())) {
        return Err(missing());
    }
    let source = state.transfer.backup_dir.join(name);
    let content = blocking(move || {
        if !source.exists() {
            return Ok(None);
        }
        std::fs::read_to_string(&source).map(Some)
    })
    .await
    .map_err(TransferError::Io)?
    .map_err(|e| TransferError::Io(e.to_string()))?
    .ok_or_else(missing)?;
    if content.trim().is_empty() {
        return Err(TransferError::Invalid("内容为空".to_owned()));
    }
    import(state, kind, content, false).await
}
