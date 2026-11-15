#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncWriteExt, DuplexStream};
use tokio::sync::Notify;
use tokio_util::codec::Framed;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use zphira_catalog::{BoxFuture as CatBoxFuture, CatalogConfig, ChartIndex, ChartPool};
use zphira_common::args::ServerArgs;
use zphira_common::gson;
use zphira_common::i18n::I18n;
use zphira_common::time::JavaOffsetDateTime;
use zphira_game::{BoxFuture, GameContext, GameDeps, GameOptions, GameUpstream, UpstreamError};
use zphira_ledger::{Points, RoundRecords};
use zphira_phira::{ChartInfo, ChartListPage, GameRecord, UserInfo};
use zphira_protocol::{
    ClientBoundPacket, ClientCodec, GameState, Message, PacketResult, ServerBoundPacket,
};

thread_local! {
    static CAPTURE: std::cell::RefCell<Option<Logs>> = const { std::cell::RefCell::new(None) };
}

#[derive(Clone, Default)]
pub struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.lines().iter().any(|l| l.contains(needle))
    }

    pub fn count(&self, needle: &str) -> usize {
        self.lines().iter().filter(|l| l.contains(needle)).count()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

struct CaptureLayer;

struct MessageVisitor {
    message: String,
    error: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "error" => self.error = Some(format!("{value:?}")),
            _ => {}
        }
    }
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if !event.metadata().target().starts_with("zphira") {
            return;
        }
        let level = *event.metadata().level();
        if level > Level::INFO {
            return;
        }
        CAPTURE.with(|capture| {
            if let Some(logs) = capture.borrow().as_ref() {
                let mut visitor = MessageVisitor {
                    message: String::new(),
                    error: None,
                };
                event.record(&mut visitor);
                let mut line = format!("{level} {}", visitor.message);
                if let Some(error) = visitor.error {
                    line.push_str(&format!(" | error={error}"));
                }
                logs.0.lock().unwrap().push(line);
            }
        });
    }
}

pub struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURE.with(|capture| *capture.borrow_mut() = None);
    }
}

pub fn capture_logs() -> (CaptureGuard, Logs) {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(CaptureLayer),
        );
    });
    let logs = Logs::default();
    CAPTURE.with(|capture| *capture.borrow_mut() = Some(logs.clone()));
    (CaptureGuard, logs)
}

pub fn chart(id: i32, name: &str, level: &str) -> Arc<ChartInfo> {
    Arc::new(
        gson::from_str(&format!(
            r#"{{"id":{id},"name":"{name}","level":"{level}"}}"#
        ))
        .unwrap(),
    )
}

#[derive(Default)]
pub struct FakeCatalog {
    charts: Mutex<HashMap<i32, Arc<ChartInfo>>>,
}

impl FakeCatalog {
    pub fn add(&self, chart: Arc<ChartInfo>) {
        self.charts.lock().unwrap().insert(chart.id, chart);
    }
}

impl zphira_catalog::Upstream for FakeCatalog {
    fn chart(&self, id: i32) -> CatBoxFuture<'_, Result<Arc<ChartInfo>, UpstreamError>> {
        let found = self.charts.lock().unwrap().get(&id).cloned();
        Box::pin(async move {
            found.ok_or_else(|| UpstreamError::new(format!("HTTP 404: chart {id}"), Some(404)))
        })
    }

    fn chart_page<'a>(
        &'a self,
        _page: i32,
        _division: Option<&'a str>,
    ) -> CatBoxFuture<'a, Result<ChartListPage, UpstreamError>> {
        Box::pin(async { Ok(ChartListPage::default()) })
    }

    fn probe_duration(&self, chart_id: i32) -> CatBoxFuture<'_, Result<i32, UpstreamError>> {
        Box::pin(async move { Err(UpstreamError::new(format!("no audio for {chart_id}"), None)) })
    }
}

#[derive(Default)]
pub struct Gate {
    open: Mutex<bool>,
    notify: Notify,
    pub reached: Notify,
    pub waiting: AtomicUsize,
}

impl Gate {
    pub async fn pass(&self) {
        self.waiting.fetch_add(1, Ordering::SeqCst);
        self.reached.notify_waiters();
        loop {
            let notified = self.notify.notified();
            if *self.open.lock().unwrap() {
                break;
            }
            notified.await;
        }
        self.waiting.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.notify.notify_waiters();
    }

    pub async fn wait_reached(&self) {
        while self.waiting.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    }
}

#[derive(Default)]
pub struct FakeUpstream {
    pub users: Mutex<HashMap<String, UserInfo>>,
    pub records: Mutex<HashMap<i32, GameRecord>>,
    pub failing_records: Mutex<HashSet<i32>>,
    pub recent: Mutex<HashMap<i32, Vec<GameRecord>>>,
    pub failing_recent: Mutex<HashSet<i32>>,
    pub token_gate: Mutex<Option<Arc<Gate>>>,
    pub record_gate: Mutex<Option<Arc<Gate>>>,
    pub recent_gate: Mutex<Option<Arc<Gate>>>,
    pub calls: Mutex<Vec<String>>,
}

impl FakeUpstream {
    pub fn user(&self, token: &str, id: i32, name: &str) {
        let user: UserInfo = gson::from_str(&format!(r#"{{"id":{id},"name":"{name}"}}"#)).unwrap();
        self.users.lock().unwrap().insert(token.to_owned(), user);
    }

    pub fn user_with_language(&self, token: &str, id: i32, name: &str, language: &str) {
        let user: UserInfo = gson::from_str(&format!(
            r#"{{"id":{id},"name":"{name}","language":"{language}"}}"#
        ))
        .unwrap();
        self.users.lock().unwrap().insert(token.to_owned(), user);
    }

    pub fn record(&self, record: GameRecord) {
        self.records.lock().unwrap().insert(record.id, record);
    }

    pub fn recent(&self, player: i32, records: Vec<GameRecord>) {
        self.recent.lock().unwrap().insert(player, records);
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl GameUpstream for FakeUpstream {
    fn user_by_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Arc<UserInfo>, UpstreamError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("me {token}"));
            let gate = self.token_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.pass().await;
            }
            self.users
                .lock()
                .unwrap()
                .get(token)
                .cloned()
                .map(Arc::new)
                .ok_or_else(|| {
                    UpstreamError::new(r#"HTTP 401: {"error":"invalid token"}"#, Some(401))
                })
        })
    }

    fn record(&self, id: i32) -> BoxFuture<'_, Result<Arc<GameRecord>, UpstreamError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("record {id}"));
            let gate = self.record_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.pass().await;
            }
            if self.failing_records.lock().unwrap().contains(&id) {
                return Err(UpstreamError::new("HTTP request timed out", None));
            }
            self.records
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .map(Arc::new)
                .ok_or_else(|| UpstreamError::new("HTTP 404: not found", Some(404)))
        })
    }

    fn recent_records(
        &self,
        player: i32,
    ) -> BoxFuture<'_, Result<Arc<Vec<GameRecord>>, UpstreamError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!("recent {player}"));
            let gate = self.recent_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.pass().await;
            }
            if self.failing_recent.lock().unwrap().contains(&player) {
                return Err(UpstreamError::new("HTTP 503: down", Some(503)));
            }
            Ok(Arc::new(
                self.recent
                    .lock()
                    .unwrap()
                    .get(&player)
                    .cloned()
                    .unwrap_or_default(),
            ))
        })
    }
}

pub fn game_record(
    id: i32,
    player: i32,
    chart: i32,
    score: i32,
    accuracy: f32,
    std: f32,
    time_millis: i64,
) -> GameRecord {
    GameRecord {
        id,
        player,
        chart,
        score,
        accuracy,
        std,
        std_score: 90.5,
        perfect: 100,
        good: 2,
        bad: 1,
        miss: 0,
        time: JavaOffsetDateTime::from_epoch_millis(time_millis, 0),
        ..GameRecord::default()
    }
}

pub const EPOCH_MILLIS: i64 = 1_760_000_000_000;

pub struct Setup {
    pub args: ServerArgs,
    pub options: GameOptions,
    pub pools_json: String,
    pub rooms_json: Option<String>,
    pub charts: Vec<Arc<ChartInfo>>,
}

pub const CHART_ALPHA: i32 = 1;
pub const CHART_BETA: i32 = 2;

pub fn default_charts() -> Vec<Arc<ChartInfo>> {
    vec![
        chart(1, "Alpha", "IN 12"),
        chart(2, "Beta", "AT 14"),
        chart(3, "Gamma", "HD 9"),
        chart(4, "Delta", "EZ 3"),
        chart(5, "Epsilon", "IN 15"),
    ]
}

pub fn default_pools_json() -> String {
    r#"{"pools":[
      {"id":1,"default_flag":true,"chart_ids":[1,2],"category":"MANUAL","order":0},
      {"id":2,"default_flag":false,"chart_ids":[3,4],"category":"MANUAL","order":1}
    ]}"#
    .to_owned()
}

impl Default for Setup {
    fn default() -> Self {
        let options = GameOptions {
            rng_seed: Some(7),
            ..GameOptions::default()
        };
        Self {
            args: ServerArgs::default(),
            options,
            pools_json: default_pools_json(),
            rooms_json: None,
            charts: default_charts(),
        }
    }
}

pub struct World {
    pub ctx: GameContext,
    pub upstream: Arc<FakeUpstream>,
    pub catalog_upstream: Arc<FakeCatalog>,
    pub catalog: ChartPool,
    pub points: Points,
    pub rounds: RoundRecords,
    pub dir: tempfile::TempDir,
    pub logs: Logs,
    _capture: CaptureGuard,
    next_port: AtomicUsize,
}

impl World {
    pub async fn new() -> World {
        World::with(Setup::default()).await
    }

    pub async fn with(setup: Setup) -> World {
        let (capture, logs) = capture_logs();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("chart-pools.json"), &setup.pools_json).unwrap();
        std::fs::write(
            dir.path()
                .join(zphira_catalog::files::DURATION_RESET_MARKER),
            "",
        )
        .unwrap();
        if let Some(rooms) = &setup.rooms_json {
            std::fs::write(dir.path().join("rooms.json"), rooms).unwrap();
        }
        let catalog_upstream = Arc::new(FakeCatalog::default());
        for chart in &setup.charts {
            catalog_upstream.add(chart.clone());
        }
        let config = CatalogConfig::new(dir.path());
        let cat_up: Arc<dyn zphira_catalog::Upstream> = catalog_upstream.clone();
        let catalog = ChartPool::load(&config, cat_up.clone()).await.unwrap();
        let index = ChartIndex::load(&config, cat_up, &catalog).await;
        let points = Points::load(dir.path()).unwrap();
        let rounds = RoundRecords::load(dir.path()).unwrap();
        let upstream = Arc::new(FakeUpstream::default());
        let start = tokio::time::Instant::now();
        let mut options = setup.options;
        if options.wall_clock.is_none() {
            options.wall_clock = Some(Arc::new(move || {
                EPOCH_MILLIS + start.elapsed().as_millis() as i64
            }));
        }
        let ctx = GameContext::new(GameDeps {
            args: setup.args,
            i18n: I18n::new("zh-CN"),
            upstream: upstream.clone(),
            catalog: catalog.clone(),
            index: Some(index),
            points: points.clone(),
            rounds: rounds.clone(),
            data_dir: dir.path().to_path_buf(),
            options,
        });
        logs.clear();
        World {
            ctx,
            upstream,
            catalog_upstream,
            catalog,
            points,
            rounds,
            dir,
            logs,
            _capture: capture,
            next_port: AtomicUsize::new(40000),
        }
    }

    pub fn now_millis(&self) -> i64 {
        EPOCH_MILLIS
    }

    pub fn data(&self) -> &Path {
        self.dir.path()
    }

    pub fn peer(&self) -> SocketAddr {
        let port = self.next_port.fetch_add(1, Ordering::SeqCst);
        SocketAddr::new("127.0.0.1".parse().unwrap(), port as u16)
    }

    pub fn raw(&self, peer: SocketAddr, buffer: usize) -> DuplexStream {
        let (client, server) = tokio::io::duplex(buffer);
        let ctx = self.ctx.clone();
        tokio::spawn(async move { ctx.serve_connection(server, peer).await });
        client
    }

    pub async fn connect(&self) -> Client {
        self.connect_from(self.peer()).await
    }

    pub async fn connect_from(&self, peer: SocketAddr) -> Client {
        let mut stream = self.raw(peer, 4 * 1024 * 1024);
        stream.write_all(&[1]).await.unwrap();
        Client::new(stream, peer)
    }

    pub async fn login(&self, id: i32, name: &str) -> Client {
        let token = format!("token-{id}-{name}");
        self.upstream.user(&token, id, name);
        let mut client = self.connect().await;
        client.send(ServerBoundPacket::Authenticate { token }).await;
        let reply = client.expect_next().await;
        assert!(
            reply.starts_with(&format!("Authenticate ok {id} {name} room=-")),
            "login of {name} failed: {reply}"
        );
        let welcome: Vec<String> = client.expect_count(6).await;
        assert_eq!(welcome.last().map(String::as_str), Some(SEP), "{welcome:?}");
        client.id = id;
        client
    }

    pub fn round_records(&self) -> Vec<zphira_ledger::RoundRecord> {
        self.rounds
            .query(1000, 0, None, None)
            .into_iter()
            .map(|r| (*r).clone())
            .collect()
    }
}

pub struct Client {
    pub framed: Framed<DuplexStream, ClientCodec>,
    pub peer: SocketAddr,
    pub id: i32,
}

impl Client {
    pub fn new(stream: DuplexStream, peer: SocketAddr) -> Client {
        Client {
            framed: Framed::new(stream, ClientCodec::new()),
            peer,
            id: 0,
        }
    }

    pub async fn send(&mut self, packet: ServerBoundPacket) {
        self.framed.send(packet).await.unwrap();
    }

    pub async fn recv(&mut self) -> Option<ClientBoundPacket> {
        match self.framed.next().await {
            Some(Ok(p)) => Some(p),
            _ => None,
        }
    }

    /// Paused clock only: there the 1 ms timeout is a quiescence barrier.
    pub async fn drain(&mut self) -> Vec<ClientBoundPacket> {
        let mut out = Vec::new();
        while let Ok(Some(Ok(p))) =
            tokio::time::timeout(Duration::from_millis(1), self.framed.next()).await
        {
            out.push(p);
        }
        out
    }

    pub async fn expect_next(&mut self) -> String {
        match tokio::time::timeout(MUST_ARRIVE, self.framed.next()).await {
            Ok(Some(Ok(p))) => render(&p),
            Ok(other) => panic!("connection ended while a packet was expected: {other:?}"),
            Err(_) => panic!("no packet within {MUST_ARRIVE:?}"),
        }
    }

    pub async fn expect_count(&mut self, n: usize) -> Vec<String> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.expect_next().await);
        }
        out
    }

    pub async fn expect_until(&mut self, until: impl Fn(&str) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            let line = self.expect_next().await;
            let done = until(&line);
            out.push(line);
            if done {
                return out;
            }
        }
    }

    /// Everything queued for this client before the server handled a ping.
    pub async fn barrier(&mut self) -> Vec<String> {
        self.send(ServerBoundPacket::Ping).await;
        let mut lines = self.expect_until(|l| l == "Pong").await;
        lines.pop();
        lines
    }

    pub async fn lines(&mut self) -> Vec<String> {
        self.drain().await.iter().map(render).collect()
    }

    pub async fn closed(&mut self) -> bool {
        loop {
            match tokio::time::timeout(Duration::from_secs(30), self.framed.next()).await {
                Ok(None) | Ok(Some(Err(_))) => return true,
                Ok(Some(Ok(_))) => continue,
                Err(_) => return false,
            }
        }
    }

    pub async fn closed_now(&mut self) -> bool {
        matches!(
            tokio::time::timeout(Duration::from_millis(1), self.framed.next()).await,
            Ok(None) | Ok(Some(Err(_)))
        )
    }

    pub async fn until_closed(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(30), self.framed.next()).await {
                Ok(Some(Ok(p))) => out.push(render(&p)),
                Ok(None) | Ok(Some(Err(_))) => return out,
                Err(_) => panic!("connection still open; got {out:?}"),
            }
        }
    }

    pub async fn shutdown(mut self) {
        let _ = self.framed.get_mut().shutdown().await;
    }
}

fn state(s: &GameState) -> String {
    match s {
        GameState::SelectChart(None) => "SelectChart".to_owned(),
        GameState::SelectChart(Some(id)) => format!("SelectChart({id})"),
        GameState::WaitForReady => "WaitForReady".to_owned(),
        GameState::Playing => "Playing".to_owned(),
    }
}

fn result<T>(name: &str, r: &PacketResult<T>) -> String {
    match r.failed_message() {
        Some(m) => format!("{name} failed: {m}"),
        None => format!("{name} ok"),
    }
}

pub fn render(packet: &ClientBoundPacket) -> String {
    match packet {
        ClientBoundPacket::Pong => "Pong".to_owned(),
        ClientBoundPacket::Authenticate(r) => match r.result() {
            Some(ok) => format!(
                "Authenticate ok {} {} room={}",
                ok.user_profile.user_id(),
                ok.user_profile.user_name(),
                ok.room_info.as_ref().map_or_else(
                    || "-".to_owned(),
                    |i| format!(
                        "{}:{}:{}",
                        i.room_id,
                        state(&i.state),
                        i.users
                            .iter()
                            .map(|u| format!("{}{}", u.user_id(), if u.monitor { "m" } else { "" }))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                )
            ),
            None => result("Authenticate", r),
        },
        ClientBoundPacket::Chat(r) => result("Chat", r),
        ClientBoundPacket::Touches { id, frames } => format!("Touches {id} x{}", frames.len()),
        ClientBoundPacket::Judges { id, judges } => format!("Judges {id} x{}", judges.len()),
        ClientBoundPacket::Message(m) => match m {
            Message::Chat { user: -1, content } => format!("> {content}"),
            Message::Chat { user, content } => format!("[{user}] {content}"),
            Message::JoinRoom { user, name } => format!("msg JoinRoom {user} {name}"),
            Message::LeaveRoom { user, name } => format!("msg LeaveRoom {user} {name}"),
            Message::SelectChart { user, name, id } => {
                format!("msg SelectChart {user} {name} {id}")
            }
            Message::Ready { user } => format!("msg Ready {user}"),
            Message::CancelReady { user } => format!("msg CancelReady {user}"),
            Message::Abort { user } => format!("msg Abort {user}"),
            Message::StartPlaying => "msg StartPlaying".to_owned(),
            Message::GameEnd => "msg GameEnd".to_owned(),
            Message::CycleRoom { cycle } => format!("msg CycleRoom {cycle}"),
            other => format!("msg {other:?}"),
        },
        ClientBoundPacket::ChangeState(s) => format!("state {}", state(s)),
        ClientBoundPacket::ChangeHost { host } => format!("host {host}"),
        ClientBoundPacket::CreateRoom(r) => result("CreateRoom", r),
        ClientBoundPacket::JoinRoom(r) => match r.result() {
            Some(ok) => format!(
                "JoinRoom ok {} [{}] live={}",
                state(&ok.game_state),
                ok.users
                    .iter()
                    .map(|u| format!("{}{}", u.user_id(), if u.monitor { "m" } else { "" }))
                    .collect::<Vec<_>>()
                    .join(","),
                ok.live
            ),
            None => result("JoinRoom", r),
        },
        ClientBoundPacket::OnJoinRoom(u) => format!(
            "OnJoinRoom {} {}{}",
            u.user_id(),
            u.user_name(),
            if u.monitor { " monitor" } else { "" }
        ),
        ClientBoundPacket::LeaveRoom(r) => result("LeaveRoom", r),
        ClientBoundPacket::LockRoom(r) => result("LockRoom", r),
        ClientBoundPacket::CycleRoom(r) => result("CycleRoom", r),
        ClientBoundPacket::SelectChart(r) => result("SelectChart", r),
        ClientBoundPacket::RequestStart(r) => result("RequestStart", r),
        ClientBoundPacket::Ready(r) => result("Ready", r),
        ClientBoundPacket::CancelReady(r) => result("CancelReady", r),
        ClientBoundPacket::Played(r) => result("Played", r),
        ClientBoundPacket::Abort(r) => result("Abort", r),
    }
}

pub fn only<'a>(lines: &'a [String], prefix: &str) -> Vec<&'a str> {
    lines
        .iter()
        .filter(|l| l.starts_with(prefix))
        .map(String::as_str)
        .collect()
}

pub const SEP: &str = "> ————————————————————————————————————————";

pub fn vote_board(room: &str, points: i32, rank: usize, alpha: usize, beta: usize) -> Vec<String> {
    vec![
        SEP.to_owned(),
        format!("> 当前积分：{points}，积分排名：#{rank}"),
        format!("> {room} 本轮谱池 #1"),
        "> 使用选谱操作投票，只能选择下列歌曲。票数最高者开局。".to_owned(),
        format!("> [{alpha}票] Alpha  |  Lv.IN 12  |  ID:1"),
        format!("> [{beta}票] Beta  |  Lv.AT 14  |  ID:2"),
        SEP.to_owned(),
    ]
}

/// Paused clock only: returns once every task is idle.
pub async fn settle() {
    tokio::time::sleep(Duration::from_millis(1)).await;
}

pub const MUST_ARRIVE: Duration = Duration::from_secs(20);

pub fn s(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| (*l).to_owned()).collect()
}
