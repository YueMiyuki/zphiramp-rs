use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(zphira_common::time::now_millis)
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Poisoning is ignored: the guarded data is consistent between statements.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
