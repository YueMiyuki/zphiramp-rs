use std::borrow::Borrow;
use std::hash::Hash;
use std::time::Duration;

use moka::sync::Cache;

pub(crate) struct TtlCache<K, V> {
    inner: Cache<K, V>,
}

impl<K, V> Clone for TtlCache<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<K, V> TtlCache<K, V>
where
    K: Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub(crate) fn new(expire_after_write: Duration, maximum_size: u64) -> Self {
        Self {
            inner: Cache::builder()
                .time_to_live(expire_after_write)
                .max_capacity(maximum_size)
                .build(),
        }
    }

    pub(crate) fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.inner.get(key)
    }

    pub(crate) fn insert(&self, key: K, value: V) {
        self.inner.insert(key, value);
    }

    #[cfg(test)]
    pub(crate) fn limits(&self) -> (Duration, u64) {
        let policy = self.inner.policy();
        (
            policy.time_to_live().expect("ttl is set"),
            policy.max_capacity().expect("capacity is set"),
        )
    }

    #[cfg(test)]
    pub(crate) fn sync(&self) {
        self.inner.run_pending_tasks();
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_expire_after_write() {
        let cache: TtlCache<i32, i32> = TtlCache::new(Duration::from_millis(150), 10);
        cache.insert(1, 10);
        assert_eq!(cache.get(&1), Some(10));
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(cache.get(&1), Some(10));
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(cache.get(&1), None);
        cache.insert(1, 11);
        assert_eq!(cache.get(&1), Some(11));
    }

    #[test]
    fn size_is_bounded() {
        let cache: TtlCache<i32, i32> = TtlCache::new(Duration::from_secs(60), 50);
        for i in 0..2000 {
            cache.insert(i, i);
        }
        cache.sync();
        assert!(cache.entry_count() <= 50, "{}", cache.entry_count());
    }

    #[test]
    fn borrowed_lookup_needs_no_allocation_of_the_key() {
        let cache: TtlCache<String, i32> = TtlCache::new(Duration::from_secs(60), 10);
        cache.insert("token".to_owned(), 1);
        assert_eq!(cache.get("token"), Some(1));
        assert_eq!(cache.get("other"), None);
    }
}
