use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct LedgerError {
    action: Action,
    what: &'static str,
    path: PathBuf,
    source: Box<dyn Error + Send + Sync>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Load,
    Save,
    Export,
}

impl LedgerError {
    pub(crate) fn load(
        what: &'static str,
        path: &Path,
        source: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> Self {
        Self::new(Action::Load, what, path, source)
    }

    pub(crate) fn save(
        what: &'static str,
        path: &Path,
        source: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> Self {
        Self::new(Action::Save, what, path, source)
    }

    pub(crate) fn export(
        what: &'static str,
        path: &Path,
        source: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> Self {
        Self::new(Action::Export, what, path, source)
    }

    fn new(
        action: Action,
        what: &'static str,
        path: &Path,
        source: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> Self {
        Self {
            action,
            what,
            path: path.to_path_buf(),
            source: source.into(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verb = match self.action {
            Action::Load => "load",
            Action::Save => "save",
            Action::Export => "export",
        };
        write!(f, "Failed to {verb} {}", self.what)
    }
}

impl Error for LedgerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.source)
    }
}
