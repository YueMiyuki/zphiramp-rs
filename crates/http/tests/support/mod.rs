#![allow(dead_code)]

pub mod game;
pub mod replay;

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use http::{HeaderMap, Request, Response, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use zphira_catalog::{CatalogConfig, ChartIndex, ChartPool, Submissions, Upstream};
use zphira_http::{AppState, HttpConfig, JwtService, Stores};
use zphira_ledger::{Admins, Points, RoundRecords};
use zphira_phira::{Options, PhiraClient};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[derive(Default)]
pub struct FakeBehaviour {
    pub failing_charts: Vec<i32>,
    pub user_delay: Option<Duration>,
}

pub struct FakePhira {
    pub base_url: String,
    pub behaviour: Arc<Mutex<FakeBehaviour>>,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub login_calls: Arc<AtomicUsize>,
}

fn index_charts() -> serde_json::Map<String, Value> {
    let text = std::fs::read_to_string(fixtures().join("seed/data/chart-index.json")).unwrap();
    match serde_json::from_str(&text).unwrap() {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

fn chart_json(base: &str, charts: &serde_json::Map<String, Value>, id: i32) -> String {
    let mut chart = match charts.get(&id.to_string()) {
        Some(Value::Object(c)) => {
            let mut c = c.clone();
            c.remove("duration_seconds");
            c
        }
        _ => match serde_json::from_str(&format!(
            "{{\"id\":{id},\"name\":\"C{id}\",\"level\":\"AT Lv.17\",\"difficulty\":17.5,\"charter\":\"ch{id}\",\"ranked\":false,\"reviewed\":true,\"stable\":true,\"stableRequest\":false,\"uploader\":0,\"tags\":[\"regular\"],\"rating\":0.9,\"ratingCount\":10,\"created\":\"2025-01-02T03:04:05.123456+08:00\",\"updated\":\"2025-03-04T05:06:07Z\",\"chartUpdated\":\"2025-03-04T05:06:00Z\"}}"
        ))
        .unwrap()
        {
            Value::Object(c) => c,
            _ => unreachable!(),
        },
    };
    chart.insert("file".into(), Value::String(format!("{base}files/{id}")));
    Value::Object(chart).to_string()
}

fn archive(id: i32) -> Vec<u8> {
    let seconds = 300 + id % 100;
    let mut mp3 = vec![0u8; usize::try_from(seconds).unwrap() * 16000];
    mp3[0] = 0xFF;
    mp3[1] = 0xFB;
    mp3[2] = 0x90;
    let mut crc = flate2::Crc::new();
    crc.update(&mp3);
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&mp3).unwrap();
    let compressed = encoder.finish().unwrap();
    let name = b"song.mp3";
    let mut out = Vec::new();
    let le16 = |v: u16| v.to_le_bytes();
    let le32 = |v: u32| v.to_le_bytes();
    out.extend_from_slice(&le32(0x0403_4b50));
    out.extend_from_slice(&le16(20));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(8));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le32(crc.sum()));
    out.extend_from_slice(&le32(compressed.len() as u32));
    out.extend_from_slice(&le32(mp3.len() as u32));
    out.extend_from_slice(&le16(name.len() as u16));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(name);
    out.extend_from_slice(&compressed);
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&le32(0x0201_4b50));
    out.extend_from_slice(&le16(20));
    out.extend_from_slice(&le16(20));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(8));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le32(crc.sum()));
    out.extend_from_slice(&le32(compressed.len() as u32));
    out.extend_from_slice(&le32(mp3.len() as u32));
    out.extend_from_slice(&le16(name.len() as u16));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le32(0));
    out.extend_from_slice(&le32(0));
    out.extend_from_slice(name);
    let cd_size = out.len() as u32 - cd_offset;
    out.extend_from_slice(&le32(0x0605_4b50));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(0));
    out.extend_from_slice(&le16(1));
    out.extend_from_slice(&le16(1));
    out.extend_from_slice(&le32(cd_size));
    out.extend_from_slice(&le32(cd_offset));
    out.extend_from_slice(&le16(0));
    out
}

fn reply(status: u16, body: impl Into<Body>) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(body.into())
        .unwrap()
}

impl FakePhira {
    pub async fn start() -> FakePhira {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/", listener.local_addr().unwrap());
        let behaviour = Arc::new(Mutex::new(FakeBehaviour::default()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let login_calls = Arc::new(AtomicUsize::new(0));
        let charts = Arc::new(index_charts());
        let archives: Arc<Mutex<HashMap<i32, Arc<Vec<u8>>>>> = Arc::default();
        let (b, r, l, base) = (
            behaviour.clone(),
            requests.clone(),
            login_calls.clone(),
            base_url.clone(),
        );
        let app = Router::new().fallback(move |request: Request<Body>| {
            let (behaviour, requests, logins, charts, archives, base) = (
                b.clone(),
                r.clone(),
                l.clone(),
                charts.clone(),
                archives.clone(),
                base.clone(),
            );
            async move {
                let path = request.uri().path().to_owned();
                requests
                    .lock()
                    .unwrap()
                    .push(format!("{} {}", request.method(), path));
                let range = request
                    .headers()
                    .get("Range")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let bearer = request
                    .headers()
                    .get("Authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .map(str::to_owned);
                let body = request.into_body().collect().await.unwrap().to_bytes();
                if path == "/login" {
                    logins.fetch_add(1, Ordering::SeqCst);
                    let v: Value = serde_json::from_slice(&body).unwrap();
                    let email = v["email"].as_str().unwrap_or("").to_owned();
                    let password = v["password"].as_str().unwrap_or("").to_owned();
                    return if password != "right" {
                        reply(401, "{\"detail\":\"wrong password\"}")
                    } else if email == "empty@x" {
                        reply(200, "")
                    } else if email == "html@x" {
                        reply(200, "<html>")
                    } else if email == "notoken@x" {
                        reply(200, "{\"id\":9}")
                    } else if email == "down@x" {
                        reply(503, "down")
                    } else {
                        let id: i32 = email
                            .chars()
                            .filter(char::is_ascii_digit)
                            .collect::<String>()
                            .parse()
                            .unwrap();
                        reply(
                            200,
                            format!("{{\"id\":{id},\"token\":\"phira-token-{id}\",\"refresh_token\":\"rt\",\"expire_at\":\"2026-01-01T00:00:00Z\"}}"),
                        )
                    };
                }
                if path == "/me" {
                    let parsed = bearer.as_deref().and_then(|t| {
                        let rest = t.strip_prefix("token-")?;
                        let (id, name) = rest.split_once('-')?;
                        Some((id.parse::<i32>().ok()?, name.to_owned()))
                    });
                    return match parsed {
                        Some((id, name)) => reply(200, format!("{{\"id\":{id},\"name\":\"{name}\"}}")),
                        None => reply(401, "{\"error\":\"invalid token\"}"),
                    };
                }
                if path == "/record" {
                    return reply(200, "[]");
                }
                if path.starts_with("/record/") {
                    return reply(404, "{\"detail\":\"Not found\"}");
                }
                if let Some(id) = path.strip_prefix("/user/") {
                    let delay = behaviour.lock().unwrap().user_delay;
                    if let Some(delay) = delay {
                        tokio::time::sleep(delay).await;
                    }
                    let id: i32 = id.parse().unwrap();
                    return reply(200, format!("{{\"id\":{id},\"name\":\"player{id}\"}}"));
                }
                if path == "/chart" {
                    return reply(200, "{\"count\":0,\"results\":[]}");
                }
                if let Some(id) = path.strip_prefix("/chart/") {
                    let id: i32 = id.parse().unwrap();
                    if behaviour.lock().unwrap().failing_charts.contains(&id) {
                        return reply(500, "boom");
                    }
                    if id >= 900_000 {
                        return reply(404, "{\"detail\":\"Not found\"}");
                    }
                    return reply(200, chart_json(&base, &charts, id));
                }
                if let Some(id) = path.strip_prefix("/files/") {
                    let id: i32 = id.parse().unwrap();
                    let bytes = archives
                        .lock()
                        .unwrap()
                        .entry(id)
                        .or_insert_with(|| Arc::new(archive(id)))
                        .clone();
                    let (mut start, mut end) = (0usize, bytes.len() - 1);
                    if let Some(range) = range {
                        let spec = &range[6..];
                        if let Some(suffix) = spec.strip_prefix('-') {
                            start = bytes.len().saturating_sub(suffix.parse::<usize>().unwrap());
                        } else {
                            let mut parts = spec.split('-');
                            start = parts.next().unwrap().parse().unwrap();
                            if let Some(e) = parts.next().filter(|e| !e.is_empty()) {
                                end = end.min(e.parse().unwrap());
                            }
                        }
                    }
                    return reply(206, bytes[start..=end].to_vec());
                }
                reply(404, "")
            }
        });
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        FakePhira {
            base_url,
            behaviour,
            requests,
            login_calls,
        }
    }

    pub fn client(&self) -> PhiraClient {
        PhiraClient::with_options(
            self.base_url.clone(),
            Options {
                retry_delay: Duration::from_millis(2),
                ..Options::default()
            },
        )
        .unwrap()
    }
}

pub struct Env {
    pub dir: tempfile::TempDir,
    pub game: zphira_game::GameContext,
    pub stores: Loaded,
    pub data: PathBuf,
    pub phira: FakePhira,
    pub state: Arc<AppState>,
    pub router: Router,
    pub admin: String,
    pub player: String,
    pub user5: String,
    pub fakeadmin2: String,
}

#[derive(Clone)]
pub struct Loaded {
    pub pools: ChartPool,
    pub index: ChartIndex,
    pub submissions: Submissions,
    pub points: Points,
    pub records: RoundRecords,
    pub admins: Admins,
}

pub async fn load_stores(data: &Path, client: &PhiraClient) -> Loaded {
    let config = CatalogConfig::new(data);
    let upstream: Arc<dyn Upstream> = Arc::new(client.clone());
    let pools = ChartPool::load(&config, upstream.clone()).await.unwrap();
    let index = ChartIndex::load(&config, upstream, &pools).await;
    let submissions = Submissions::load(&config).await;
    let points = Points::load(data).unwrap();
    let records = RoundRecords::load(data).unwrap();
    let admins = Admins::load(data).unwrap();
    Loaded {
        pools,
        index,
        submissions,
        points,
        records,
        admins,
    }
}

impl Env {
    pub async fn new() -> Env {
        Self::with_config(HttpConfig::default()).await
    }

    pub async fn with_config(config: HttpConfig) -> Env {
        Self::build(config, |_| {}).await
    }

    pub async fn build(config: HttpConfig, setup: impl FnOnce(&Path)) -> Env {
        Self::build_with_args(config, setup, zphira_common::args::ServerArgs::default()).await
    }

    pub async fn build_with_args(
        config: HttpConfig,
        setup: impl FnOnce(&Path),
        args: zphira_common::args::ServerArgs,
    ) -> Env {
        let dir = tempfile::tempdir().unwrap();
        copy_dir(&fixtures().join("seed"), dir.path());
        setup(dir.path());
        let data = dir.path().join("data");
        let phira = FakePhira::start().await;
        let client = phira.client();
        let loaded = load_stores(&data, &client).await;
        let jwt = JwtService::load(&data).unwrap();
        let game = zphira_game::GameContext::new(zphira_game::GameDeps {
            args,
            i18n: zphira_common::i18n::I18n::new("zh-CN"),
            upstream: Arc::new(client.clone()),
            catalog: loaded.pools.clone(),
            index: Some(loaded.index.clone()),
            points: loaded.points.clone(),
            rounds: loaded.records.clone(),
            data_dir: data.clone(),
            options: zphira_game::GameOptions::default(),
        });
        let state = AppState::builder(Stores {
            phira: client,
            pools: loaded.pools.clone(),
            index: loaded.index.clone(),
            submissions: loaded.submissions.clone(),
            points: loaded.points.clone(),
            records: loaded.records.clone(),
            admins: loaded.admins.clone(),
            jwt: jwt.clone(),
        })
        .data_dir(&data)
        .frontend_dir(dir.path().join("frontend/dist"))
        .game(game.clone())
        .config(config)
        .build();
        let router = zphira_http::router(state.clone());
        Env {
            game,
            stores: loaded,
            admin: jwt.issue_token(1, true),
            player: jwt.issue_token(2, false),
            user5: jwt.issue_token(5, false),
            fakeadmin2: jwt.issue_token(2, true),
            dir,
            data,
            phira,
            state,
            router,
        }
    }

    pub fn resolve(&self, s: &str) -> String {
        let mut out = s
            .replace("{admin}", &self.admin)
            .replace("{player}", &self.player)
            .replace("{user5}", &self.user5)
            .replace("{fakeadmin2}", &self.fakeadmin2);
        while let Some(start) = out.find("{backup:") {
            let end = start + out[start..].find('}').unwrap();
            let spec = &out[start + 8..end];
            let (kind, index) = spec.split_once(':').unwrap();
            let names = self.backups(kind);
            let index: usize = index.parse().unwrap();
            let name = names.get(index).cloned().unwrap_or_else(|| "none".into());
            out.replace_range(start..=end, &name);
        }
        out
    }

    pub fn backups(&self, kind: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.data.join("backups"))
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with(&format!("{kind}-")))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names.reverse();
        names
    }

    pub async fn send(&self, request: Request<Body>) -> Reply {
        send(&self.router, request).await
    }

    pub async fn call(
        &self,
        method: &str,
        target: &str,
        token: Option<&str>,
        body: Option<&str>,
    ) -> Reply {
        let mut builder = Request::builder().method(method).uri(target);
        if let Some(token) = token {
            builder = builder.header("Authorization", format!("Bearer {token}"));
        }
        if body.is_some() {
            builder = builder.header("Content-Type", "application/json");
        }
        let request = builder
            .body(Body::from(body.unwrap_or("").to_owned()))
            .unwrap();
        self.send(request).await
    }

    pub async fn get(&self, target: &str) -> Reply {
        self.call("GET", target, Some(&self.admin.clone()), None)
            .await
    }
}

#[derive(Debug)]
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

pub async fn send(router: &Router, mut request: Request<Body>) -> Reply {
    if request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_none()
    {
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
    }
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let mut body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    if headers
        .get("content-encoding")
        .is_some_and(|v| v.as_bytes() == b"gzip")
    {
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&body[..]), &mut out).unwrap();
        body = out;
    }
    Reply {
        status,
        headers,
        body,
    }
}

pub fn error_body(reason: &str) -> Value {
    serde_json::json!({"ok": false, "reason": reason})
}
