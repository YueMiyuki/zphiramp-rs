use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::error::LedgerError;

pub const SAVE_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) type Flusher = Arc<dyn Fn() -> Result<(), LedgerError> + Send + Sync>;

pub struct WriterHandle {
    stop: Arc<Notify>,
    task: Option<JoinHandle<()>>,
    flush: Option<Flusher>,
    what: &'static str,
}

impl std::fmt::Debug for WriterHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterHandle")
            .field("what", &self.what)
            .field("running", &self.task.is_some())
            .finish()
    }
}

impl WriterHandle {
    pub async fn stop(mut self) {
        self.shutdown().await;
    }

    pub async fn stop_and_flush(mut self) -> Result<(), LedgerError> {
        self.shutdown().await;
        let Some(flush) = self.flush.take() else {
            return Ok(());
        };
        match tokio::task::spawn_blocking(move || flush()).await {
            Ok(result) => result,
            Err(join) => {
                tracing::error!(error = %join, "Failed to persist {}", self.what);
                Ok(())
            }
        }
    }

    async fn shutdown(&mut self) {
        self.stop.notify_one();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for WriterHandle {
    fn drop(&mut self) {
        self.stop.notify_one();
    }
}

pub(crate) fn spawn(what: &'static str, interval: Duration, flush: Flusher) -> WriterHandle {
    let stop = Arc::new(Notify::new());
    let task = {
        let stop = Arc::clone(&stop);
        let flush = Arc::clone(&flush);
        tokio::spawn(async move {
            loop {
                // A stop request that arrives during a write stays as a `Notify` permit and ends
                // the next wait.
                if tokio::time::timeout(interval, stop.notified())
                    .await
                    .is_ok()
                {
                    break;
                }
                let flush = Arc::clone(&flush);
                if let Err(join) = tokio::task::spawn_blocking(move || flush()).await {
                    tracing::error!(error = %join, "Failed to persist {what}");
                }
            }
        })
    };
    WriterHandle {
        stop,
        task: Some(task),
        flush: Some(flush),
        what,
    }
}
