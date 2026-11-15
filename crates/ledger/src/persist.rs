use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use zphira_common::{fs, json};

use crate::error::LedgerError;
use crate::import::ImportError;
use crate::util::lock;

pub(crate) trait Document: Send + 'static {
    type Content: Send + 'static;

    fn content(&self) -> Self::Content;

    fn install(&mut self, content: Self::Content);

    fn view(content: &Self::Content) -> impl Serialize + '_;

    fn rows(content: &Self::Content) -> usize;
}

pub(crate) struct Slot<D> {
    pub data: D,
    pub dirty: bool,
    pub mutations: u64,
}

// `persist` serializes file writes and is taken before `state`; `state` is not held across IO
// except by `import_with_backup`.
pub(crate) struct Persisted<D: Document> {
    path: PathBuf,
    what: &'static str,
    save_failed: &'static str,
    persist: Mutex<()>,
    state: Mutex<Slot<D>>,
}

impl<D: Document> Persisted<D> {
    pub fn new(path: PathBuf, what: &'static str, save_failed: &'static str, data: D) -> Self {
        Self {
            path,
            what,
            save_failed,
            persist: Mutex::new(()),
            state: Mutex::new(Slot {
                data,
                dirty: false,
                mutations: 0,
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut D) -> R) -> R {
        f(&mut lock(&self.state).data)
    }

    pub fn mutate<R>(&self, f: impl FnOnce(&mut D) -> R) -> R {
        let mut slot = lock(&self.state);
        let result = f(&mut slot.data);
        slot.dirty = true;
        slot.mutations += 1;
        result
    }

    pub fn flush(&self) -> Result<(), LedgerError> {
        let _persist = lock(&self.persist);
        // The flag is cleared before the snapshot: a mutation after it sets it again.
        let content = {
            let mut slot = lock(&self.state);
            if !slot.dirty {
                return Ok(());
            }
            slot.dirty = false;
            slot.data.content()
        };
        match write_file::<D>(&self.path, &content) {
            Ok(()) => Ok(()),
            Err(e) => {
                lock(&self.state).dirty = true;
                tracing::error!(error = %e, "{}", self.save_failed);
                Err(LedgerError::save(self.what, &self.path, e))
            }
        }
    }

    pub fn export(&self) -> Result<String, LedgerError> {
        let content = lock(&self.state).data.content();
        render::<D>(&content).map_err(|e| LedgerError::export(self.what, &self.path, e))
    }

    pub fn import_with_backup(
        &self,
        build: impl Fn(Option<&D::Content>) -> D::Content,
        backup: impl FnOnce(&str) -> std::io::Result<String>,
    ) -> Result<(usize, String), ImportError> {
        let _persist = lock(&self.persist);
        // The state lock stays held until the swap, so no mutation lands between the backup and
        // the swap.
        let mut slot = lock(&self.state);
        let current = slot.data.content();
        let content = build(Some(&current));
        let state = render::<D>(&current).map_err(|e| ImportError::Write(e.to_string()))?;
        let backup_file = backup(&state).map_err(|e| ImportError::Write(e.to_string()))?;
        write_file::<D>(&self.path, &content).map_err(|e| ImportError::Write(e.to_string()))?;
        let rows = D::rows(&content);
        slot.data.install(content);
        slot.dirty = false;
        Ok((rows, backup_file))
    }

    pub fn import(
        &self,
        needs_current: bool,
        build: impl Fn(Option<&D::Content>) -> D::Content,
    ) -> Result<usize, ImportError> {
        let _persist = lock(&self.persist);
        let (current, seen) = {
            let slot = lock(&self.state);
            (needs_current.then(|| slot.data.content()), slot.mutations)
        };
        let content = build(current.as_ref());
        write_file::<D>(&self.path, &content).map_err(|e| ImportError::Write(e.to_string()))?;

        let mut slot = lock(&self.state);
        // A merge is rebuilt over mutations that landed during the write; a replace discards them.
        let moved = slot.mutations != seen;
        let content = if moved && needs_current {
            build(Some(&slot.data.content()))
        } else {
            content
        };
        let rows = D::rows(&content);
        slot.data.install(content);
        slot.dirty = moved && needs_current;
        Ok(rows)
    }
}

fn render<D: Document>(content: &D::Content) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&D::view(content))
}

fn write_file<D: Document>(path: &Path, content: &D::Content) -> std::io::Result<()> {
    fs::write_json_pretty(path, &D::view(content))
}

pub(crate) enum Loaded<T> {
    Missing,
    Empty,
    Value(T),
}

pub(crate) fn read_file<T: serde::de::DeserializeOwned>(
    what: &'static str,
    path: &Path,
) -> Result<Loaded<T>, LedgerError> {
    if let Err(e) = fs::remove_stale_temp_files(path) {
        tracing::warn!(error = %e, "Could not remove stale temporary files of {}", path.display());
    }
    let text = fs::read_to_string_if_exists(path).map_err(|e| LedgerError::load(what, path, e))?;
    let Some(text) = text else {
        return Ok(Loaded::Missing);
    };
    match json::from_str_opt::<T>(&text) {
        Ok(Some(value)) => Ok(Loaded::Value(value)),
        Ok(None) => Ok(Loaded::Empty),
        Err(e) => Err(LedgerError::load(what, path, e)),
    }
}
