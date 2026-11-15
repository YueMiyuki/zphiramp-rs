use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

use crate::util::lock;

pub const MAX_PER_ADDRESS: usize = 64;

#[derive(Debug)]
pub struct ConnectionLimiter {
    max_total: usize,
    state: Mutex<Counts>,
}

#[derive(Debug, Default)]
struct Counts {
    total: usize,
    per_address: HashMap<IpAddr, usize>,
}

impl ConnectionLimiter {
    pub fn new(max_total: usize) -> Self {
        Self {
            max_total,
            state: Mutex::new(Counts::default()),
        }
    }

    pub fn accept(&self, address: IpAddr) -> Result<(), String> {
        let mut counts = lock(&self.state);
        if counts.total >= self.max_total {
            return Err(format!("connection limit reached ({})", self.max_total));
        }
        let per = counts.per_address.get(&address).copied().unwrap_or(0);
        if per >= MAX_PER_ADDRESS {
            return Err(format!("too many connections from {address}"));
        }
        counts.total += 1;
        *counts.per_address.entry(address).or_insert(0) += 1;
        Ok(())
    }

    pub fn release(&self, address: IpAddr) {
        let mut counts = lock(&self.state);
        counts.total = counts.total.saturating_sub(1);
        if let Some(count) = counts.per_address.get_mut(&address) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.per_address.remove(&address);
            }
        }
    }

    pub fn count(&self) -> usize {
        lock(&self.state).total
    }

    pub fn count_for(&self, address: IpAddr) -> usize {
        lock(&self.state)
            .per_address
            .get(&address)
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_and_per_address_caps() {
        let limiter = ConnectionLimiter::new(100);
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        for _ in 0..MAX_PER_ADDRESS {
            limiter.accept(a).unwrap();
        }
        assert_eq!(
            limiter.accept(a).unwrap_err(),
            "too many connections from 10.0.0.1"
        );
        for _ in 0..(100 - MAX_PER_ADDRESS) {
            limiter.accept(b).unwrap();
        }
        assert_eq!(
            limiter.accept(b).unwrap_err(),
            "connection limit reached (100)"
        );
        limiter.release(a);
        limiter.accept(b).unwrap();
        assert_eq!(limiter.count(), 100);
    }

    #[test]
    fn g19_counter_never_undercounts_under_churn() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let limiter = Arc::new(ConnectionLimiter::new(10_000));
        let invisible = Arc::new(AtomicUsize::new(0));
        let addr: IpAddr = "192.0.2.7".parse().unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let limiter = limiter.clone();
                let invisible = invisible.clone();
                std::thread::spawn(move || {
                    for _ in 0..5_000 {
                        if limiter.accept(addr).is_ok() {
                            if limiter.count_for(addr) == 0 {
                                invisible.fetch_add(1, Ordering::SeqCst);
                            }
                            limiter.release(addr);
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(limiter.count(), 0);
        assert_eq!(limiter.count_for(addr), 0);
        assert_eq!(invisible.load(Ordering::SeqCst), 0);
    }
}
