use std::sync::Arc;

use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    Console,
    Signal(&'static str),
    Failure,
}

impl Cause {
    pub fn exit_code(self) -> i32 {
        match self {
            Cause::Failure => 1,
            Cause::Console | Cause::Signal(_) => 0,
        }
    }
}

#[derive(Clone)]
pub struct Shutdown {
    tx: Arc<watch::Sender<Option<Cause>>>,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutdown {
    pub fn new() -> Self {
        Self {
            tx: Arc::new(watch::channel(None).0),
        }
    }

    /// `true` only for the first request.
    pub fn request(&self, cause: Cause) -> bool {
        self.tx.send_if_modified(|current| {
            if current.is_some() {
                return false;
            }
            *current = Some(cause);
            true
        })
    }

    pub fn is_requested(&self) -> bool {
        self.tx.borrow().is_some()
    }

    pub async fn requested(&self) -> Cause {
        let mut rx = self.tx.subscribe();
        loop {
            if let Some(cause) = *rx.borrow_and_update() {
                return cause;
            }
            if rx.changed().await.is_err() {
                // The sender lives in `self`, so this cannot happen; never spin on it.
                std::future::pending::<()>().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_first_request_wins() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_requested());
        assert!(shutdown.request(Cause::Signal("SIGTERM")));
        assert!(!shutdown.request(Cause::Console));
        assert!(!shutdown.request(Cause::Failure));
        assert_eq!(shutdown.requested().await, Cause::Signal("SIGTERM"));
        assert_eq!(shutdown.requested().await, Cause::Signal("SIGTERM"));
    }

    #[tokio::test]
    async fn a_waiter_wakes_up() {
        let shutdown = Shutdown::new();
        let waiter = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move { shutdown.requested().await })
        };
        tokio::task::yield_now().await;
        let from_thread = shutdown.clone();
        std::thread::spawn(move || from_thread.request(Cause::Console))
            .join()
            .unwrap();
        assert_eq!(waiter.await.unwrap(), Cause::Console);
    }

    #[test]
    fn exit_codes() {
        assert_eq!(Cause::Console.exit_code(), 0);
        assert_eq!(Cause::Signal("SIGINT").exit_code(), 0);
        assert_eq!(Cause::Failure.exit_code(), 1);
    }
}
