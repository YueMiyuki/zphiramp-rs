use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use zphira_common::fs::{self, AtomicWriteOptions};

use crate::upstream::BoxFuture;

#[derive(Clone)]
pub struct CatalogConfig {
    pub data_dir: PathBuf,
    pub files: Arc<dyn FileSink>,
    pub sleeper: Arc<dyn Sleeper>,
    pub clock: Arc<dyn Clock>,
    pub max_pending_submissions: Option<u32>,
}

impl CatalogConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            files: Arc::new(AtomicFiles),
            sleeper: Arc::new(TokioSleeper),
            clock: Arc::new(SystemClock),
            max_pending_submissions: Some(crate::submissions::DEFAULT_MAX_PENDING_SUBMISSIONS),
        }
    }

    pub fn with_files(mut self, files: Arc<dyn FileSink>) -> Self {
        self.files = files;
        self
    }

    pub fn with_sleeper(mut self, sleeper: Arc<dyn Sleeper>) -> Self {
        self.sleeper = sleeper;
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_max_pending_submissions(mut self, cap: Option<u32>) -> Self {
        self.max_pending_submissions = cap;
        self
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.data_dir.join(name)
    }
}

pub trait FileSink: Send + Sync + 'static {
    fn write_atomic(
        &self,
        path: &Path,
        write: &mut dyn FnMut(&mut dyn Write) -> io::Result<()>,
    ) -> io::Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AtomicFiles;

impl FileSink for AtomicFiles {
    fn write_atomic(
        &self,
        path: &Path,
        write: &mut dyn FnMut(&mut dyn Write) -> io::Result<()>,
    ) -> io::Result<()> {
        fs::write_atomic_with(path, &AtomicWriteOptions::new(), |w| write(w))
    }
}

pub trait Sleeper: Send + Sync + 'static {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
