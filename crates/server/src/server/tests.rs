use std::sync::{Arc, Mutex};

use super::*;

#[derive(Default)]
struct Steps {
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Steps {
    fn note(&self, step: &'static str) {
        self.log.lock().unwrap().push(step);
    }

    fn steps(&self) -> Vec<&'static str> {
        self.log.lock().unwrap().clone()
    }
}

impl StopSteps for Steps {
    async fn stop_http(&mut self) {
        self.note("http");
    }

    async fn stop_game(&mut self) {
        self.note("game");
    }

    fn stop_background(&mut self) {
        self.note("background");
    }

    async fn flush_points(&mut self) {
        self.note("points");
    }

    async fn flush_rounds(&mut self) {
        self.note("rounds");
    }

    async fn flush_rooms(&mut self) {
        self.note("rooms");
    }

    async fn join_listener(&mut self) {
        self.note("listener");
    }
}

#[tokio::test]
async fn the_stop_runs_http_then_game_then_background_then_every_flush_last() {
    let mut steps = Steps::default();
    graceful_stop(&mut steps, Instant::now()).await;
    assert_eq!(
        steps.steps(),
        [
            "http",
            "game",
            "background",
            "points",
            "rounds",
            "rooms",
            "listener"
        ]
    );
}

#[tokio::test]
async fn a_second_stop_request_cannot_start_a_second_stop() {
    let shutdown = Shutdown::new();
    assert!(shutdown.request(Cause::Signal("SIGTERM")));
    assert!(!shutdown.request(Cause::Signal("SIGINT")));
    assert!(!shutdown.request(Cause::Console));
    assert_eq!(shutdown.requested().await.exit_code(), 0);
}

#[test]
fn uptime_is_minutes_and_remaining_seconds() {
    assert_eq!(uptime_line(Duration::from_millis(0)), "Uptime: 0m 0s");
    assert_eq!(uptime_line(Duration::from_millis(59_999)), "Uptime: 0m 59s");
    assert_eq!(uptime_line(Duration::from_millis(60_000)), "Uptime: 1m 0s");
    assert_eq!(
        uptime_line(Duration::from_secs(3 * 3600 + 25 * 60 + 7)),
        "Uptime: 205m 7s"
    );
}

#[test]
fn the_upstream_url_comes_from_the_environment_d5() {
    assert_eq!(phira_base_url(None), "https://phira.5wyxi.com/");
    assert_eq!(phira_base_url(Some("")), "https://phira.5wyxi.com/");
    assert_eq!(phira_base_url(Some("   ")), "https://phira.5wyxi.com/");
    assert_eq!(
        phira_base_url(Some("http://127.0.0.1:9/i/a/")),
        "http://127.0.0.1:9/i/a/"
    );
    assert_eq!(
        phira_base_url(Some("http://127.0.0.1:9/i/a")),
        "http://127.0.0.1:9/i/a/"
    );
    assert_eq!(phira_base_url(Some(" http://x.test ")), "http://x.test/");
}

#[tokio::test]
async fn bind_addresses_resolve_like_inet_address_get_by_name() {
    let any = resolve_bind_address("0.0.0.0", 12346).await.unwrap();
    assert_eq!(any, "0.0.0.0:12346".parse().unwrap());
    let v6 = resolve_bind_address("[::1]", 80).await.unwrap();
    assert_eq!(v6, "[::1]:80".parse().unwrap());
    let v6 = resolve_bind_address("::1", 80).await.unwrap();
    assert_eq!(v6, "[::1]:80".parse().unwrap());
    let empty = resolve_bind_address("", 7).await.unwrap();
    assert_eq!(empty, "127.0.0.1:7".parse().unwrap());
    let local = resolve_bind_address("localhost", 7).await.unwrap();
    assert!(local.is_ipv4(), "{local}");
}

#[test]
fn the_limit_flags_reach_the_stores_and_the_http_api() {
    let defaults = ServerArgs::default();
    assert_eq!(catalog_config(&defaults).max_pending_submissions, Some(50));
    assert_eq!(http_config(&defaults).login_rate_limit, Some(30));

    let tuned = ServerArgs {
        max_pending_submissions: Some(7),
        login_rate_limit: None,
        ..ServerArgs::default()
    };
    assert_eq!(catalog_config(&tuned).max_pending_submissions, Some(7));
    assert_eq!(http_config(&tuned).login_rate_limit, None);

    let unlimited = ServerArgs {
        max_pending_submissions: None,
        login_rate_limit: Some(2),
        ..ServerArgs::default()
    };
    assert_eq!(catalog_config(&unlimited).max_pending_submissions, None);
    assert_eq!(http_config(&unlimited).login_rate_limit, Some(2));
}

#[test]
fn a_start_failure_names_its_cause() {
    let error = context(
        "Failed to bind 127.0.0.1:1",
        io::Error::new(io::ErrorKind::AddrInUse, "Address already in use"),
    );
    assert_eq!(error.to_string(), "Failed to bind 127.0.0.1:1");
    assert_eq!(
        error.source().map(ToString::to_string).as_deref(),
        Some("Address already in use")
    );
}

#[tokio::test]
async fn aborting_a_start_that_never_got_going_is_harmless() {
    let mut handles = Handles::default();
    handles.abort_startup().await;
    let pending = tokio::time::timeout(Duration::from_millis(50), handles.failure()).await;
    assert!(pending.is_err());
}

mod failures {
    use std::net::{SocketAddr, TcpListener, TcpStream};

    use zphira_game::{GameOptions, TaskName};
    use zphira_http::{AppState, HttpServer, JwtService, Stores};
    use zphira_phira::PhiraClient;

    use super::*;
    use crate::testutil::{Fixture, captured, install_capture};

    struct Running {
        _fixture: Fixture,
        handles: Handles,
        game_addr: SocketAddr,
        http_addr: SocketAddr,
    }

    async fn running(options: GameOptions) -> Running {
        install_capture();
        let f = Fixture::with_options(options).await;
        let game = f.game.clone();
        let mut handles = Handles {
            game: Some(game.clone()),
            background: Some(game.start_background()),
            points_writer: Some(f.points.spawn_writer()),
            rounds_writer: Some(f.rounds.spawn_writer()),
            ..Handles::default()
        };
        let listener = game.bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let game_addr = listener.local_addr().unwrap();
        handles.accept = Some(listener.spawn());
        let state = AppState::builder(Stores {
            phira: PhiraClient::new("http://127.0.0.1:1/").unwrap(),
            pools: f.pools.clone(),
            index: f.index.clone(),
            submissions: f.submissions.clone(),
            points: f.points.clone(),
            records: f.rounds.clone(),
            admins: f.admins.clone(),
            jwt: JwtService::load(f.dir.path()).unwrap(),
        })
        .data_dir(f.dir.path())
        .game(game)
        .build();
        let mut http = HttpServer::start(state, "127.0.0.1", 0);
        let http_addr = http.await_started().await.unwrap();
        handles.http = Some(http);
        Running {
            _fixture: f,
            handles,
            game_addr,
            http_addr,
        }
    }

    fn is_open(addr: SocketAddr) -> bool {
        TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
    }

    async fn supervise_with_limit(r: &mut Running, shutdown: &Shutdown) -> i32 {
        let mut signals = Signals::install().unwrap();
        tokio::time::timeout(
            Duration::from_secs(30),
            supervise(&mut r.handles, shutdown, &mut signals, Instant::now()),
        )
        .await
        .expect("the server did not stop")
    }

    fn logged_error(text: &str) -> bool {
        captured()
            .iter()
            .any(|(level, _, m)| *level == tracing::Level::ERROR && m == text)
    }

    fn assert_stopped(r: &Running) {
        assert!(!is_open(r.game_addr), "the game port is still open");
        assert!(!is_open(r.http_addr), "the HTTP port is still open");
        assert!(r.handles.http.is_none());
        assert!(r.handles.background.is_none());
        assert!(r.handles.points_writer.is_none());
        assert!(r.handles.rounds_writer.is_none());
        assert!(r.handles.accept.is_none());
    }

    fn fast(task: TaskName) -> GameOptions {
        GameOptions {
            fail_task: Some(task),
            watchdog_interval: Duration::from_millis(20),
            room_store_interval: Duration::from_millis(20),
            ..GameOptions::default()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_requested_stop_exits_zero_and_logs_no_failure() {
        let mut r = running(GameOptions::default()).await;
        assert!(is_open(r.game_addr) && is_open(r.http_addr));
        let shutdown = Shutdown::new();
        shutdown.request(Cause::Console);
        assert_eq!(supervise_with_limit(&mut r, &shutdown).await, 0);
        assert_stopped(&r);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_panicking_watchdog_stops_the_server_with_code_one() {
        let mut r = running(fast(TaskName::Watchdog)).await;
        let code = supervise_with_limit(&mut r, &Shutdown::new()).await;
        assert_eq!(code, 1);
        assert!(logged_error(
            "Server task Watchdog ended unexpectedly (panicked: injected fault in Watchdog), stopping the server"
        ));
        assert_stopped(&r);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_panicking_room_store_writer_stops_the_server_with_code_one() {
        let mut r = running(fast(TaskName::RoomStoreWriter)).await;
        let code = supervise_with_limit(&mut r, &Shutdown::new()).await;
        assert_eq!(code, 1);
        assert!(logged_error(
            "Server task RoomStoreWriter ended unexpectedly (panicked: injected fault in RoomStoreWriter), stopping the server"
        ));
        assert_stopped(&r);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_accept_loop_that_ends_stops_the_server_with_code_one() {
        let mut r = running(GameOptions::default()).await;
        r.handles.game.as_ref().unwrap().stop_accepting();
        let code = supervise_with_limit(&mut r, &Shutdown::new()).await;
        assert_eq!(code, 1);
        assert!(logged_error(
            "Server task Listener ended unexpectedly (returned), stopping the server"
        ));
        assert_stopped(&r);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_http_server_that_ends_stops_the_server_with_code_one() {
        let mut r = running(GameOptions::default()).await;
        let blocker = TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = blocker.local_addr().unwrap();
        let old = r.handles.http.take().unwrap();
        old.stop().await;
        let state = AppState::builder(Stores {
            phira: PhiraClient::new("http://127.0.0.1:1/").unwrap(),
            pools: r._fixture.pools.clone(),
            index: r._fixture.index.clone(),
            submissions: r._fixture.submissions.clone(),
            points: r._fixture.points.clone(),
            records: r._fixture.rounds.clone(),
            admins: r._fixture.admins.clone(),
            jwt: JwtService::load(r._fixture.dir.path()).unwrap(),
        })
        .game(r.handles.game.clone().unwrap())
        .build();
        r.handles.http = Some(HttpServer::start(state, "127.0.0.1", busy.port()));
        let code = supervise_with_limit(&mut r, &Shutdown::new()).await;
        assert_eq!(code, 1);
        assert!(logged_error(
            "Server task HttpServer ended unexpectedly (failed to bind), stopping the server"
        ));
        assert!(!is_open(r.game_addr), "the game port is still open");
        assert!(r.handles.http.is_none() && r.handles.accept.is_none());
        drop(blocker);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_first_cause_decides_the_exit_code() {
        let mut r = running(fast(TaskName::Watchdog)).await;
        let shutdown = Shutdown::new();
        shutdown.request(Cause::Signal("SIGTERM"));
        assert_eq!(supervise_with_limit(&mut r, &shutdown).await, 0);
        assert_stopped(&r);
    }
}
