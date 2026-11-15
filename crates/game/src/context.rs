use std::collections::{BTreeMap, HashMap};
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zphira_catalog::{ChartIndex, ChartPool};
use zphira_common::args::ServerArgs;
use zphira_common::i18n::I18n;
use zphira_ledger::{Points, RoundRecords};

use crate::net::limiter::ConnectionLimiter;
use crate::player::Player;
use crate::room::{RoomRegistry, store};
use crate::services::{AcceptedRecords, GameOptions, Services, SettlementTracker};
use crate::upstream::GameUpstream;
use crate::util::{Rng, lock};
use std::future::Future as _;

pub struct GameDeps {
    pub args: ServerArgs,
    pub i18n: I18n,
    pub upstream: Arc<dyn GameUpstream>,
    pub catalog: ChartPool,
    pub index: Option<ChartIndex>,
    pub points: Points,
    pub rounds: RoundRecords,
    pub data_dir: PathBuf,
    pub options: GameOptions,
}

pub(crate) struct Channel {
    pub(crate) kill: CancellationToken,
}

pub(crate) struct GameInner {
    pub(crate) svc: Arc<Services>,
    pub(crate) rooms: Arc<RoomRegistry>,
    pub(crate) players: Mutex<BTreeMap<i32, Arc<Player>>>,
    pub(crate) limiter: ConnectionLimiter,
    pub(crate) channels: Mutex<HashMap<u64, Channel>>,
    pub(crate) channels_changed: Notify,
    pub(crate) next_id: AtomicU64,
    pub(crate) accepting: CancellationToken,
    pub(crate) listener_bound: AtomicBool,
    pub(crate) rooms_path: PathBuf,
    pub(crate) store_lock: Mutex<()>,
}

#[derive(Clone)]
pub struct GameContext {
    pub(crate) inner: Arc<GameInner>,
}

impl std::fmt::Debug for GameContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameContext").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskName {
    Watchdog,
    RoomStoreWriter,
    Listener,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    Returned,
    Panicked(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskExit {
    pub task: TaskName,
    pub outcome: TaskOutcome,
}

impl TaskExit {
    pub fn is_failure(&self) -> bool {
        !(self.task == TaskName::Listener && self.outcome == TaskOutcome::Returned)
    }
}

fn task_exit(task: TaskName, result: Result<(), tokio::task::JoinError>) -> TaskExit {
    let outcome = match result {
        Ok(()) => TaskOutcome::Returned,
        Err(e) if e.is_panic() => {
            let payload = e.into_panic();
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic with a non-string payload".to_owned());
            TaskOutcome::Panicked(message)
        }
        Err(_) => TaskOutcome::Cancelled,
    };
    TaskExit { task, outcome }
}

async fn first_exit(tasks: &mut Vec<(TaskName, JoinHandle<()>)>) -> TaskExit {
    std::future::poll_fn(|cx| {
        for i in 0..tasks.len() {
            if let std::task::Poll::Ready(result) = std::pin::Pin::new(&mut tasks[i].1).poll(cx) {
                let (task, _) = tasks.swap_remove(i);
                return std::task::Poll::Ready(task_exit(task, result));
            }
        }
        std::task::Poll::Pending
    })
    .await
}

#[derive(Debug)]
pub struct Background {
    tasks: Vec<(TaskName, JoinHandle<()>)>,
}

impl Background {
    /// Resolves when a task ended on its own (each reported once); cancel-safe.
    pub async fn finished(&mut self) -> TaskExit {
        first_exit(&mut self.tasks).await
    }

    pub fn stop(self) {
        for (_, task) in self.tasks {
            task.abort();
        }
    }
}

#[derive(Debug)]
pub struct ListenerTask {
    tasks: Vec<(TaskName, JoinHandle<()>)>,
}

impl ListenerTask {
    /// Resolves when the accept loop ended (each end reported once); cancel-safe.
    pub async fn finished(&mut self) -> TaskExit {
        first_exit(&mut self.tasks).await
    }
}

#[derive(Debug)]
pub struct Listener {
    listener: TcpListener,
    ctx: GameContext,
}

impl Listener {
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn run(self) {
        let accepting = self.ctx.inner.accepting.clone();
        loop {
            tokio::select! {
                () = accepting.cancelled() => break,
                accepted = self.listener.accept() => match accepted {
                    Ok((stream, peer)) => {
                        let _ = stream.set_nodelay(true);
                        let ctx = self.ctx.clone();
                        tokio::spawn(async move { ctx.serve_connection(stream, peer).await });
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to create a new channel from an accepted socket.");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }

    pub fn spawn(self) -> ListenerTask {
        let runtime = self.ctx.svc().runtime.clone();
        ListenerTask {
            tasks: vec![(TaskName::Listener, runtime.spawn(self.run()))],
        }
    }
}

impl GameContext {
    /// Must be called inside a Tokio runtime.
    pub fn new(deps: GameDeps) -> Self {
        let runtime = tokio::runtime::Handle::try_current()
            .expect("GameContext::new must be called inside a Tokio runtime");
        let clock = deps
            .options
            .wall_clock
            .clone()
            .unwrap_or_else(|| Arc::new(zphira_common::time::now_millis));
        let seed = deps
            .options
            .rng_seed
            .unwrap_or_else(|| (clock)() as u64 ^ 0x5DEE_CE66_D1CE_4E5B);
        let max_connections = deps.args.max_connections as usize;
        let svc = Arc::new(Services {
            args: deps.args,
            i18n: deps.i18n,
            upstream: deps.upstream,
            catalog: deps.catalog,
            index: deps.index,
            points: deps.points,
            rounds: deps.rounds,
            rooms_dirty: AtomicBool::new(false),
            accepted: Mutex::new(AcceptedRecords::default()),
            rng: Mutex::new(Rng::new(seed)),
            clock,
            settlements: SettlementTracker::default(),
            options: deps.options,
            runtime,
        });
        Self {
            inner: Arc::new(GameInner {
                svc,
                rooms: Arc::new(RoomRegistry::default()),
                players: Mutex::new(BTreeMap::new()),
                limiter: ConnectionLimiter::new(max_connections),
                channels: Mutex::new(HashMap::new()),
                channels_changed: Notify::new(),
                next_id: AtomicU64::new(1),
                accepting: CancellationToken::new(),
                listener_bound: AtomicBool::new(false),
                rooms_path: store::path_in(&deps.data_dir),
                store_lock: Mutex::new(()),
            }),
        }
    }

    pub(crate) fn svc(&self) -> &Arc<Services> {
        &self.inner.svc
    }

    pub(crate) fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn start_background(&self) -> Background {
        let runtime = &self.svc().runtime;
        let watchdog = {
            let ctx = self.clone();
            let interval = self.svc().options.watchdog_interval;
            runtime.spawn(async move {
                loop {
                    tokio::time::sleep(interval).await;
                    ctx.inject_fault(TaskName::Watchdog);
                    ctx.reclaim_empty_rooms();
                }
            })
        };
        let writer = {
            let ctx = self.clone();
            let interval = self.svc().options.room_store_interval;
            runtime.spawn(async move {
                loop {
                    tokio::time::sleep(interval).await;
                    ctx.inject_fault(TaskName::RoomStoreWriter);
                    ctx.flush_rooms().await;
                }
            })
        };
        Background {
            tasks: vec![
                (TaskName::Watchdog, watchdog),
                (TaskName::RoomStoreWriter, writer),
            ],
        }
    }

    fn inject_fault(&self, task: TaskName) {
        if self.svc().options.fail_task == Some(task) {
            panic!("injected fault in {task:?}");
        }
    }

    pub fn reclaim_empty_rooms(&self) {
        let minutes = self.svc().args.empty_room_ttl_minutes;
        if minutes == 0 {
            return;
        }
        let ttl = Duration::from_secs(u64::from(minutes) * 60);
        let now = tokio::time::Instant::now();
        for room in self.inner.rooms.all() {
            room.reclaim_if_idle(ttl, now);
        }
    }

    pub async fn bind(&self, addr: SocketAddr) -> io::Result<Listener> {
        let listener = TcpListener::bind(addr).await?;
        self.inner.listener_bound.store(true, Ordering::Release);
        Ok(Listener {
            listener,
            ctx: self.clone(),
        })
    }

    pub fn stop_accepting(&self) {
        self.inner.accepting.cancel();
    }

    pub(crate) fn register_channel(&self) -> (u64, CancellationToken) {
        let id = self.next_id();
        let kill = CancellationToken::new();
        lock(&self.inner.channels).insert(id, Channel { kill: kill.clone() });
        (id, kill)
    }

    pub(crate) fn unregister_channel(&self, id: u64) {
        lock(&self.inner.channels).remove(&id);
        self.inner.channels_changed.notify_waiters();
    }

    pub fn connection_count(&self) -> usize {
        lock(&self.inner.channels).len()
    }

    async fn channels_closed(&self, limit: Duration) {
        let wait = async {
            loop {
                let notified = self.inner.channels_changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if lock(&self.inner.channels).is_empty() {
                    return;
                }
                notified.await;
            }
        };
        let _ = tokio::time::timeout(limit, wait).await;
    }

    /// Stops accepting, abandons rounds in progress, disconnects everyone; flush stores after.
    pub async fn shutdown(&self) {
        let online = self.online_count();
        let channels = self.connection_count()
            + usize::from(self.inner.listener_bound.load(Ordering::Acquire));
        self.stop_accepting();
        for room in self.inner.rooms.all() {
            room.freeze();
        }
        let svc = self.svc().clone();
        svc.settlements_idle(svc.options.recovery_timeout + Duration::from_secs(5))
            .await;
        if online > 0 {
            tracing::info!("Kicking {} player(s)...", online);
            for player in self.online_player_handles() {
                self.kick(&player);
            }
        }
        if channels > 0 {
            tracing::info!("Closing {} channel(s)...", channels);
            let kills: Vec<CancellationToken> = lock(&self.inner.channels)
                .values()
                .map(|c| c.kill.clone())
                .collect();
            for kill in kills {
                kill.cancel();
            }
            self.channels_closed(Duration::from_secs(10)).await;
            tracing::info!("Channels closed.");
        }
    }

    pub(crate) fn online_player_handles(&self) -> Vec<Arc<Player>> {
        lock(&self.inner.players)
            .values()
            .filter(|p| p.is_online())
            .cloned()
            .collect()
    }

    pub(crate) fn find_player(&self, user_id: i32) -> Option<Arc<Player>> {
        lock(&self.inner.players).get(&user_id).cloned()
    }
}
