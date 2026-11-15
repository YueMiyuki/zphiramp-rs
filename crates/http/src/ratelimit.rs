use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

const SWEEP_AT: usize = 4096;

pub(crate) struct LoginLimiter {
    limit: Option<usize>,
    window: Duration,
    attempts: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
}

impl LoginLimiter {
    pub(crate) fn new(limit: Option<u32>, window: Duration) -> Self {
        Self {
            limit: limit.map(|n| n as usize),
            window,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn try_acquire(&self, address: IpAddr, now: Instant) -> bool {
        let Some(limit) = self.limit else {
            return true;
        };
        let mut attempts = self.attempts.lock().unwrap_or_else(PoisonError::into_inner);
        if attempts.len() >= SWEEP_AT {
            let window = self.window;
            attempts.retain(|_, times| {
                times
                    .back()
                    .is_some_and(|last| now.saturating_duration_since(*last) < window)
            });
        }
        let times = attempts.entry(address).or_default();
        while times
            .front()
            .is_some_and(|first| now.saturating_duration_since(*first) >= self.window)
        {
            times.pop_front();
        }
        if times.len() >= limit {
            return false;
        }
        times.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_window() {
        let limiter = LoginLimiter::new(Some(3), Duration::from_secs(60));
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let other: IpAddr = "10.0.0.2".parse().unwrap();
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(limiter.try_acquire(ip, t0 + Duration::from_secs(i)));
        }
        assert!(!limiter.try_acquire(ip, t0 + Duration::from_secs(10)));
        assert!(limiter.try_acquire(other, t0 + Duration::from_secs(10)));
        assert!(limiter.try_acquire(ip, t0 + Duration::from_secs(60)));
        assert!(!limiter.try_acquire(ip, t0 + Duration::from_secs(60)));
        assert!(limiter.try_acquire(ip, t0 + Duration::from_secs(62)));
    }

    #[test]
    fn no_limit() {
        let limiter = LoginLimiter::new(None, Duration::from_secs(60));
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let t0 = Instant::now();
        assert!((0..1000).all(|_| limiter.try_acquire(ip, t0)));
    }
}
