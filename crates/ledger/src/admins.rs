use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use zphira_common::fs::{self, AtomicWriteOptions};
use zphira_common::gson;

use crate::error::LedgerError;
use crate::import::{
    BackupFn, ImportError, ImportMode, ImportReport, ImportResult, Kind, parse_document,
};
use crate::util::{Clock, lock, system_clock};

const FILE_NAME: &str = "admins.json";
const WHAT: &str = "admin list";
const CHECK_INTERVAL_MILLIS: i64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signature {
    modified: Option<SystemTime>,
    len: u64,
    #[cfg(unix)]
    inode: u64,
}

impl Signature {
    fn of(path: &Path) -> io::Result<Option<Self>> {
        match std::fs::metadata(path) {
            // The inode also changes when an editor saves through a rename that keeps time and
            // size.
            Ok(meta) => Ok(Some(Self {
                modified: meta.modified().ok(),
                len: meta.len(),
                #[cfg(unix)]
                inode: std::os::unix::fs::MetadataExt::ino(&meta),
            })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

struct State {
    ids: BTreeSet<i32>,
    signature: Option<Signature>,
    checked_at: i64,
}

struct Inner {
    path: PathBuf,
    clock: Clock,
    state: Mutex<State>,
}

#[derive(Clone)]
pub struct Admins(Arc<Inner>);

impl std::fmt::Debug for Admins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admins")
            .field("path", &self.0.path)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOutcome {
    Removed,
    NotAnAdmin,
    LastAdmin,
}

impl Admins {
    pub fn load(data_dir: impl AsRef<Path>) -> Result<Self, LedgerError> {
        Self::load_with_clock(data_dir, system_clock())
    }

    pub fn load_with_clock(data_dir: impl AsRef<Path>, clock: Clock) -> Result<Self, LedgerError> {
        let path = data_dir.as_ref().join(FILE_NAME);
        if let Err(e) = fs::remove_stale_temp_files(&path) {
            tracing::warn!(error = %e, "Could not remove stale temporary files of {}", path.display());
        }
        let mut state = State {
            ids: BTreeSet::new(),
            signature: None,
            checked_at: clock(),
        };
        let signature = Signature::of(&path).map_err(|e| LedgerError::load(WHAT, &path, e))?;
        match signature {
            None => save(&path, &mut state).map_err(|e| LedgerError::save(WHAT, &path, e))?,
            Some(_) => {
                state.signature = signature;
                match read_ids(&path) {
                    Ok(ids) => state.ids = ids,
                    Err(ReadError::Unreadable(e)) => {
                        tracing::error!(error = %e, "admins.json is unreadable, keeping the current admin list");
                    }
                    Err(ReadError::Io(e)) => return Err(LedgerError::load(WHAT, &path, e)),
                }
            }
        }
        Ok(Self(Arc::new(Inner {
            path,
            clock,
            state: Mutex::new(state),
        })))
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }

    pub fn is_admin(&self, user_id: i32) -> bool {
        let mut state = lock(&self.0.state);
        self.refresh(&mut state, false);
        state.ids.contains(&user_id)
    }

    pub fn list(&self) -> Vec<i32> {
        let mut state = lock(&self.0.state);
        self.refresh(&mut state, false);
        state.ids.iter().copied().collect()
    }

    pub fn add(&self, user_id: i32) -> Result<bool, LedgerError> {
        let mut state = lock(&self.0.state);
        self.refresh(&mut state, true);
        if state.ids.contains(&user_id) {
            return Ok(false);
        }
        let mut ids = state.ids.clone();
        ids.insert(user_id);
        self.commit(&mut state, ids)?;
        Ok(true)
    }

    /// No keep-one rule (the console); the HTTP route uses `remove_keeping_one`.
    pub fn remove(&self, user_id: i32) -> Result<bool, LedgerError> {
        let mut state = lock(&self.0.state);
        self.refresh(&mut state, true);
        if !state.ids.contains(&user_id) {
            return Ok(false);
        }
        let mut ids = state.ids.clone();
        ids.remove(&user_id);
        self.commit(&mut state, ids)?;
        Ok(true)
    }

    pub fn remove_keeping_one(&self, user_id: i32) -> Result<RemoveOutcome, LedgerError> {
        let mut state = lock(&self.0.state);
        self.refresh(&mut state, true);
        if !state.ids.contains(&user_id) {
            return Ok(RemoveOutcome::NotAnAdmin);
        }
        if state.ids.len() <= 1 {
            return Ok(RemoveOutcome::LastAdmin);
        }
        let mut ids = state.ids.clone();
        ids.remove(&user_id);
        self.commit(&mut state, ids)?;
        Ok(RemoveOutcome::Removed)
    }

    pub fn export(&self) -> Result<String, LedgerError> {
        let mut state = lock(&self.0.state);
        self.refresh(&mut state, false);
        let ids: Vec<i32> = state.ids.iter().copied().collect();
        gson::to_string_pretty(&ids).map_err(|e| LedgerError::export(WHAT, &self.0.path, e))
    }

    pub fn import(&self, document: &str, mode: ImportMode) -> Result<ImportReport, ImportError> {
        let result = self.import_json(document, mode, |_| Ok(String::new()))?;
        Ok(ImportReport::rows(result.total))
    }

    pub fn import_json(
        &self,
        document: &str,
        mode: ImportMode,
        backup: impl BackupFn,
    ) -> Result<ImportResult, ImportError> {
        let root = parse_document(Kind::Admins, document)?;
        let incoming = parse_ids(&root)?;

        let mut state = lock(&self.0.state);
        self.refresh(&mut state, true);
        let mut ids = match mode {
            ImportMode::Replace => BTreeSet::new(),
            ImportMode::Merge => state.ids.clone(),
        };
        ids.extend(incoming);
        if ids.is_empty() {
            return Err(ImportError::invalid("至少保留一名管理员"));
        }
        let current: Vec<i32> = state.ids.iter().copied().collect();
        let current =
            gson::to_string_pretty(&current).map_err(|e| ImportError::Write(e.to_string()))?;
        let backup_file = backup(&current).map_err(|e| ImportError::Write(e.to_string()))?;
        write_ids(&self.0.path, &ids).map_err(|e| ImportError::Write(e.to_string()))?;
        self.install(&mut state, ids);
        let count = state.ids.len();
        Ok(ImportResult {
            imported: count,
            total: count,
            backup_file,
        })
    }

    fn refresh(&self, state: &mut State, force: bool) {
        let now = (self.0.clock)();
        // A clock that steps back (negative elapsed) must not stall the check.
        let elapsed = now - state.checked_at;
        if !force && (0..CHECK_INTERVAL_MILLIS).contains(&elapsed) {
            return;
        }
        state.checked_at = now;

        let signature = match Signature::of(&self.0.path) {
            Ok(signature) => signature,
            Err(e) => {
                tracing::error!(error = %e, "admins.json is unreadable, keeping the current admin list");
                return;
            }
        };
        if signature == state.signature {
            return;
        }
        // Taken before reading: a change during the read differs from it and is seen by the next
        // check.
        state.signature = signature;
        match signature {
            None => {
                if let Err(e) = self.save(state) {
                    match std::error::Error::source(&e) {
                        Some(cause) => tracing::error!(error = %cause, "{e}"),
                        None => tracing::error!("{e}"),
                    }
                }
            }
            Some(_) => match read_ids(&self.0.path) {
                Ok(ids) => {
                    if ids != state.ids {
                        tracing::info!(
                            "admins.json changed on disk, reloaded {} admin(s)",
                            ids.len()
                        );
                    }
                    state.ids = ids;
                }
                Err(ReadError::Unreadable(e)) => {
                    tracing::error!(error = %e, "admins.json is unreadable, keeping the current admin list");
                }
                Err(ReadError::Io(e)) => {
                    tracing::error!(error = %e, "admins.json is unreadable, keeping the current admin list");
                }
            },
        }
    }

    fn save(&self, state: &mut State) -> Result<(), LedgerError> {
        save(&self.0.path, state).map_err(|e| LedgerError::save(WHAT, &self.0.path, e))?;
        state.checked_at = (self.0.clock)();
        Ok(())
    }

    fn commit(&self, state: &mut State, ids: BTreeSet<i32>) -> Result<(), LedgerError> {
        write_ids(&self.0.path, &ids).map_err(|e| LedgerError::save(WHAT, &self.0.path, e))?;
        self.install(state, ids);
        Ok(())
    }

    fn install(&self, state: &mut State, ids: BTreeSet<i32>) {
        state.ids = ids;
        state.signature = Signature::of(&self.0.path).ok().flatten();
        state.checked_at = (self.0.clock)();
    }
}

fn save(path: &Path, state: &mut State) -> Result<(), gson::GsonError> {
    write_ids(path, &state.ids)?;
    state.signature = Signature::of(path).ok().flatten();
    Ok(())
}

fn write_ids(path: &Path, ids: &BTreeSet<i32>) -> Result<(), gson::GsonError> {
    let sorted: Vec<i32> = ids.iter().copied().collect();
    fs::write_atomic_with(path, &AtomicWriteOptions::new(), |w| {
        gson::to_writer_pretty(w, &sorted)
    })
}

enum ReadError {
    Unreadable(Box<dyn std::error::Error + Send + Sync>),
    Io(io::Error),
}

fn read_ids(path: &Path) -> Result<BTreeSet<i32>, ReadError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            return Err(ReadError::Unreadable(Box::new(e)));
        }
        Err(e) => return Err(ReadError::Io(e)),
    };
    // An empty or `null` document is an error: an editor that truncates before writing must not
    // empty the list.
    let ids: Vec<i32> = gson::from_str(&text).map_err(|e| ReadError::Unreadable(Box::new(e)))?;
    Ok(ids.into_iter().collect())
}

fn parse_ids(root: &gson::JsonElement) -> Result<Vec<i32>, ImportError> {
    let rows = root
        .as_array()
        .expect("parse_document checked the container");
    let mut seen = BTreeSet::new();
    let mut ids = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let bad = |reason: String| {
            ImportError::invalid(format!("管理员名单第 {} 项无效：{reason}", index + 1))
        };
        let id: i32 = gson::from_element(row).map_err(|_| bad("必须是整数用户 ID".to_owned()))?;
        if !seen.insert(id) {
            return Err(bad(format!("用户 ID {id} 重复")));
        }
        ids.push(id);
    }
    Ok(ids)
}
