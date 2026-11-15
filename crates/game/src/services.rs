use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use zphira_catalog::{ChartIndex, ChartPool};
use zphira_common::args::ServerArgs;
use zphira_common::i18n::I18n;
use zphira_ledger::{Points, RoundRecords};

use crate::upstream::GameUpstream;
use crate::util::{Rng, lock};

pub type WallClock = Arc<dyn Fn() -> i64 + Send + Sync>;

#[derive(Clone)]
pub struct GameOptions {
    pub suspend_timeout: Duration,
    pub pre_auth_deadline: Duration,
    pub proxy_header_timeout: Duration,
    pub handshake_timeout: Duration,
    pub recovery_timeout: Duration,
    pub recovery_concurrency: usize,
    pub room_store_interval: Duration,
    pub watchdog_interval: Duration,
    pub rng_seed: Option<u64>,
    pub wall_clock: Option<WallClock>,
    #[doc(hidden)]
    pub fail_task: Option<crate::TaskName>,
}

impl Default for GameOptions {
    fn default() -> Self {
        Self {
            suspend_timeout: Duration::from_secs(3600),
            pre_auth_deadline: Duration::from_secs(20),
            proxy_header_timeout: Duration::from_millis(5000),
            handshake_timeout: zphira_protocol::HANDSHAKE_TIMEOUT,
            recovery_timeout: Duration::from_secs(30),
            recovery_concurrency: 8,
            room_store_interval: Duration::from_secs(5),
            watchdog_interval: Duration::from_secs(30),
            rng_seed: None,
            wall_clock: None,
            fail_task: None,
        }
    }
}

impl std::fmt::Debug for GameOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameOptions")
            .field("suspend_timeout", &self.suspend_timeout)
            .field("pre_auth_deadline", &self.pre_auth_deadline)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub(crate) struct AcceptedRecords {
    set: HashSet<i32>,
    order: VecDeque<i32>,
}

impl AcceptedRecords {
    const CAPACITY: usize = 65_536;

    pub(crate) fn contains(&self, id: i32) -> bool {
        self.set.contains(&id)
    }

    pub(crate) fn insert(&mut self, id: i32) {
        if self.set.insert(id) {
            self.order.push_back(id);
            while self.order.len() > Self::CAPACITY {
                if let Some(old) = self.order.pop_front() {
                    self.set.remove(&old);
                }
            }
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct SettlementTracker {
    running: AtomicUsize,
    idle: Notify,
}

pub(crate) struct SettlementGuard(Arc<Services>);

impl Drop for SettlementGuard {
    fn drop(&mut self) {
        if self.0.settlements.running.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.settlements.idle.notify_waiters();
        }
    }
}

pub(crate) struct Services {
    pub(crate) args: ServerArgs,
    pub(crate) i18n: I18n,
    pub(crate) upstream: Arc<dyn GameUpstream>,
    pub(crate) catalog: ChartPool,
    pub(crate) index: Option<ChartIndex>,
    pub(crate) points: Points,
    pub(crate) rounds: RoundRecords,
    pub(crate) rooms_dirty: AtomicBool,
    pub(crate) accepted: Mutex<AcceptedRecords>,
    pub(crate) rng: Mutex<Rng>,
    pub(crate) clock: WallClock,
    pub(crate) settlements: SettlementTracker,
    pub(crate) options: GameOptions,
    pub(crate) runtime: tokio::runtime::Handle,
}

impl Services {
    pub(crate) fn now_millis(&self) -> i64 {
        (self.clock)()
    }

    pub(crate) fn mark_rooms_dirty(&self) {
        self.rooms_dirty.store(true, Ordering::Release);
    }

    pub(crate) fn random_below(&self, bound: usize) -> usize {
        lock(&self.rng).below(bound)
    }

    pub(crate) fn default_language(&self) -> &str {
        &self.args.default_language
    }

    pub(crate) fn track_settlement(self: &Arc<Self>) -> SettlementGuard {
        self.settlements.running.fetch_add(1, Ordering::AcqRel);
        SettlementGuard(self.clone())
    }

    pub(crate) async fn settlements_idle(&self, limit: Duration) {
        let wait = async {
            loop {
                let notified = self.settlements.idle.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.settlements.running.load(Ordering::Acquire) == 0 {
                    return;
                }
                notified.await;
            }
        };
        let _ = tokio::time::timeout(limit, wait).await;
    }
}
