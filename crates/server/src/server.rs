use std::error::Error;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use zphira_catalog::{CatalogConfig, ChartIndex, ChartPool, Submissions, Upstream};
use zphira_common::cli::ServerArgs;
use zphira_common::i18n::I18n;
use zphira_game::{
    Background, GameContext, GameDeps, GameOptions, ListenerTask, TaskExit, TaskName, TaskOutcome,
};
use zphira_http::{AppState, HttpConfig, HttpServer, JwtService, ServerExit, Stores};
use zphira_ledger::{Admins, Points, RoundRecords, WriterHandle};
use zphira_phira::{DEFAULT_BASE_URL, PhiraClient};

use crate::console::Console;
use crate::console::commands::{Commands, LogOut, Services};
use crate::console::interactive::Interactive;
use crate::shutdown::{Cause, Shutdown};
use crate::signals::Signals;

pub const DATA_DIR: &str = "data";
pub const FRONTEND_DIR: &str = "frontend/dist";
pub const PHIRA_HOST_ENV: &str = "ZPHIRA_PHIRA_HOST";

/// The override from `ZPHIRA_PHIRA_HOST` when set and not blank, with a trailing `/`; else the default.
pub fn phira_base_url(env_value: Option<&str>) -> String {
    match env_value.map(str::trim) {
        Some(value) if !value.is_empty() => {
            let mut url = value.to_owned();
            if !url.ends_with('/') {
                url.push('/');
            }
            url
        }
        _ => DEFAULT_BASE_URL.to_owned(),
    }
}

pub type BootError = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
struct Context {
    message: String,
    source: BootError,
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for Context {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.source)
    }
}

fn context(message: impl Into<String>, source: impl Into<BootError>) -> BootError {
    Box::new(Context {
        message: message.into(),
        source: source.into(),
    })
}

pub async fn resolve_bind_address(host: &str, port: u16) -> io::Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await?.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{host}: no address")))
}

async fn blocking<T, F>(work: F) -> Result<T, BootError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, BootError> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(e) => Err(Box::new(e)),
    }
}

pub trait StopSteps {
    async fn stop_http(&mut self);
    async fn stop_game(&mut self);
    fn stop_background(&mut self);
    async fn flush_points(&mut self);
    async fn flush_rounds(&mut self);
    async fn flush_rooms(&mut self);
    async fn join_listener(&mut self);
}

pub fn uptime_line(uptime: Duration) -> String {
    let millis = uptime.as_millis();
    format!("Uptime: {}m {}s", millis / 60_000, (millis / 1000) % 60)
}

pub async fn graceful_stop<S: StopSteps>(steps: &mut S, started: Instant) {
    let shutdown_start = Instant::now();
    tracing::info!("Shutting down...");
    steps.stop_http().await;
    steps.stop_game().await;
    steps.stop_background();
    // Every store is flushed last, after the game stopped scoring and moving rooms.
    steps.flush_points().await;
    steps.flush_rounds().await;
    steps.flush_rooms().await;
    steps.join_listener().await;
    tracing::info!("{}", uptime_line(started.elapsed()));
    tracing::info!(
        "Shutdown completed in {}ms. Goodbye!",
        shutdown_start.elapsed().as_millis()
    );
}

#[derive(Default)]
pub struct Handles {
    game: Option<GameContext>,
    background: Option<Background>,
    points_writer: Option<WriterHandle>,
    rounds_writer: Option<WriterHandle>,
    http: Option<HttpServer>,
    accept: Option<ListenerTask>,
}

impl StopSteps for Handles {
    async fn stop_http(&mut self) {
        if let Some(http) = self.http.take() {
            http.stop().await;
        }
    }

    async fn stop_game(&mut self) {
        if let Some(game) = &self.game {
            game.shutdown().await;
        }
    }

    fn stop_background(&mut self) {
        if let Some(background) = self.background.take() {
            background.stop();
        }
    }

    async fn flush_points(&mut self) {
        if let Some(writer) = self.points_writer.take() {
            let _ = writer.stop_and_flush().await;
        }
    }

    async fn flush_rounds(&mut self) {
        if let Some(writer) = self.rounds_writer.take() {
            let _ = writer.stop_and_flush().await;
        }
    }

    async fn flush_rooms(&mut self) {
        if let Some(game) = &self.game {
            game.mark_rooms_dirty();
            game.flush_rooms().await;
        }
    }

    async fn join_listener(&mut self) {
        // Bounded, so a wedged accept loop cannot hang the stop; a task already reported by
        // `failure()` is gone from `accept`, because a task reports only once.
        if let Some(mut accept) = self.accept.take() {
            let _ = tokio::time::timeout(LISTENER_JOIN_LIMIT, accept.finished()).await;
        }
    }
}

const LISTENER_JOIN_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFailure {
    pub task: &'static str,
    pub outcome: String,
}

impl fmt::Display for TaskFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Server task {} ended unexpectedly ({}), stopping the server",
            self.task, self.outcome
        )
    }
}

impl TaskFailure {
    fn of_game(exit: &TaskExit) -> Self {
        let task = match exit.task {
            TaskName::Watchdog => "Watchdog",
            TaskName::RoomStoreWriter => "RoomStoreWriter",
            TaskName::Listener => "Listener",
        };
        let outcome = match &exit.outcome {
            TaskOutcome::Returned => "returned".to_owned(),
            TaskOutcome::Panicked(message) => format!("panicked: {message}"),
            TaskOutcome::Cancelled => "cancelled".to_owned(),
        };
        Self { task, outcome }
    }

    fn of_http(exit: &ServerExit) -> Self {
        Self {
            task: "HttpServer",
            outcome: exit.to_string(),
        }
    }
}

impl Handles {
    pub async fn abort_startup(&mut self) {
        if let Some(game) = &self.game {
            game.stop_accepting();
        }
        self.join_listener().await;
        self.stop_background();
        self.stop_http().await;
        self.flush_points().await;
        self.flush_rounds().await;
    }

    /// Resolves when a task the server cannot run without ended on its own; pending while all run.
    pub async fn failure(&mut self) -> TaskFailure {
        let Handles {
            background,
            accept,
            http,
            ..
        } = self;
        let background = async {
            match background.as_mut() {
                Some(background) => TaskFailure::of_game(&background.finished().await),
                None => std::future::pending().await,
            }
        };
        let accept = async {
            match accept.as_mut() {
                Some(accept) => TaskFailure::of_game(&accept.finished().await),
                None => std::future::pending().await,
            }
        };
        let http = async {
            match http.as_mut() {
                Some(http) => TaskFailure::of_http(&http.closed().await),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            failure = background => failure,
            failure = accept => failure,
            failure = http => failure,
        }
    }
}

pub struct RunContext {
    pub args: ServerArgs,
    pub terminal: Option<Arc<Interactive>>,
    pub boot: Instant,
    pub phira_url: String,
}

struct Stored {
    pools: ChartPool,
    index: ChartIndex,
    submissions: Submissions,
    points: Points,
    rounds: RoundRecords,
    phira: PhiraClient,
}

fn http_config(args: &ServerArgs) -> HttpConfig {
    HttpConfig {
        login_rate_limit: args.login_rate_limit,
        ..HttpConfig::default()
    }
}

fn catalog_config(args: &ServerArgs) -> CatalogConfig {
    CatalogConfig::new(DATA_DIR).with_max_pending_submissions(args.max_pending_submissions)
}

async fn load_stores(cx: &RunContext) -> Result<Stored, BootError> {
    let phira = PhiraClient::new(cx.phira_url.clone())
        .map_err(|e| context("Cannot create the Phira client", e))?;
    let config = catalog_config(&cx.args);
    let upstream: Arc<dyn Upstream> = Arc::new(phira.clone());

    tracing::info!("Preloading chart pool...");
    let pools = ChartPool::load(&config, upstream.clone()).await?;
    tracing::info!("Chart pool preloaded.");

    let index = ChartIndex::load(&config, upstream, &pools).await;
    tracing::info!("Chart index loaded: {} charts", index.indexed_count());

    let submissions = Submissions::load(&config).await;
    let rounds = blocking(|| RoundRecords::load(DATA_DIR).map_err(Into::into)).await?;
    tracing::info!("Round records loaded: {}", rounds.len());
    let points = blocking(|| Points::load(DATA_DIR).map_err(Into::into)).await?;
    Ok(Stored {
        pools,
        index,
        submissions,
        points,
        rounds,
        phira,
    })
}

async fn boot(
    cx: &RunContext,
    handles: &mut Handles,
    shutdown: &Shutdown,
) -> Result<(), BootError> {
    let args = &cx.args;
    tracing::info!("Booting up Phira Server...");

    let i18n = I18n::new(&args.default_language);
    tracing::info!("Default language: {}", args.default_language);

    let stored = load_stores(cx).await?;

    let game = GameContext::new(GameDeps {
        i18n,
        args: args.clone(),
        upstream: Arc::new(stored.phira.clone()),
        catalog: stored.pools.clone(),
        index: Some(stored.index.clone()),
        points: stored.points.clone(),
        rounds: stored.rounds.clone(),
        data_dir: DATA_DIR.into(),
        options: GameOptions::default(),
    });
    handles.game = Some(game.clone());
    game.restore_rooms().await;
    handles.background = Some(game.start_background());
    handles.points_writer = Some(stored.points.spawn_writer());
    handles.rounds_writer = Some(stored.rounds.spawn_writer());

    let admins = blocking(|| Admins::load(DATA_DIR).map_err(Into::into)).await?;
    let jwt = blocking(|| JwtService::load(DATA_DIR).map_err(Into::into)).await?;
    let state = AppState::builder(Stores {
        phira: stored.phira,
        pools: stored.pools.clone(),
        index: stored.index.clone(),
        submissions: stored.submissions.clone(),
        points: stored.points,
        records: stored.rounds,
        admins: admins.clone(),
        jwt,
    })
    .data_dir(DATA_DIR)
    .frontend_dir(Path::new(FRONTEND_DIR))
    .config(http_config(args))
    .game(game.clone())
    .build();
    handles.http = Some(HttpServer::start(
        state,
        args.http_host.clone(),
        args.http_port,
    ));

    tracing::info!("Initializing network...");
    let addr = resolve_bind_address(&args.host, args.port)
        .await
        .map_err(|e| context(format!("Cannot resolve {}", args.host), e))?;
    let listener = game
        .bind(addr)
        .await
        .map_err(|e| context(format!("Failed to bind {addr}"), e))?;
    handles.accept = Some(listener.spawn());
    tracing::info!("Listening on {}:{}", args.host, args.port);

    if let Some(http) = handles.http.as_mut() {
        http.await_started().await?;
    }

    let commands = Commands::new(
        Services {
            game,
            pools: stored.pools,
            index: stored.index,
            submissions: stored.submissions,
            admins,
        },
        Arc::new(LogOut),
    );
    let _ = Console::start(commands, cx.terminal.clone(), shutdown.clone());

    let total_ms = cx.boot.elapsed().as_millis();
    tracing::info!("Done ({:.3}s)!", total_ms as f64 / 1000.0);
    Ok(())
}

fn log_start_failure(error: &BootError) {
    let cause: &(dyn Error + 'static) = &**error;
    tracing::error!(error = cause, "Failed to start server");
}

pub async fn run(cx: RunContext) -> i32 {
    let started = Instant::now();
    let shutdown = Shutdown::new();
    let mut handles = Handles::default();
    let mut signals = match Signals::install() {
        Ok(signals) => signals,
        Err(e) => {
            log_start_failure(&context("Cannot install the signal handlers", e));
            return 1;
        }
    };

    let booted = tokio::select! {
        result = boot(&cx, &mut handles, &shutdown) => Some(result),
        signal = signals.recv() => {
            tracing::info!("Startup interrupted by {}, shutting down.", signal);
            None
        }
    };
    match booted {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            log_start_failure(&error);
            handles.abort_startup().await;
            return 1;
        }
        None => {
            handles.abort_startup().await;
            return 0;
        }
    }

    tracing::info!("Server is running. Type 'stop' to stop.");
    supervise(&mut handles, &shutdown, &mut signals, started).await
}

pub async fn supervise(
    handles: &mut Handles,
    shutdown: &Shutdown,
    signals: &mut Signals,
    started: Instant,
) -> i32 {
    let cause = tokio::select! {
        // A stop that was asked for wins over a failure that happens to be ready as well.
        biased;
        cause = shutdown.requested() => cause,
        signal = signals.recv() => Cause::Signal(signal),
        failure = handles.failure() => {
            tracing::error!("{}", failure);
            Cause::Failure
        }
    };
    shutdown.request(cause);
    graceful_stop(handles, started).await;
    // `signals` stays alive for the caller: a second signal during the stop is swallowed.
    cause.exit_code()
}

#[cfg(test)]
mod tests;
