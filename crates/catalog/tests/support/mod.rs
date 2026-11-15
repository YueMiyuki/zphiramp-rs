#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use zphira_catalog::{
    AtomicFiles, BoxFuture, CatalogConfig, CatalogError, Clock, FileSink, Sleeper, Upstream,
    UpstreamError,
};
use zphira_phira::{ChartInfo, ChartListPage};

pub fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(rel)
}

pub fn read_fixture(rel: &str) -> String {
    std::fs::read_to_string(fixture_path(rel)).unwrap_or_else(|e| panic!("fixture {rel}: {e}"))
}

pub fn java_results(scenario: &str) -> BTreeMap<String, String> {
    read_fixture(&format!("{scenario}/result.txt"))
        .lines()
        .map(|line| {
            let (k, v) = line.split_once('\t').expect("key\tvalue");
            (k.to_owned(), unescape(v))
        })
        .collect()
}

pub fn java_log(scenario: &str) -> Vec<String> {
    read_fixture(&format!("{scenario}/log.txt"))
        .lines()
        .map(str::to_owned)
        .collect()
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            other => panic!("bad escape {other:?}"),
        }
    }
    out
}

pub fn data_dir(files: &[(&str, &str)], marker: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, rel) in files {
        std::fs::copy(fixture_path(rel), dir.path().join(name)).unwrap();
    }
    if marker {
        std::fs::write(
            dir.path()
                .join(zphira_catalog::files::DURATION_RESET_MARKER),
            "",
        )
        .unwrap();
    }
    dir
}

pub fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

pub fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap()
}

pub fn error_text(e: &CatalogError) -> String {
    let kind = match e {
        CatalogError::Invalid(_) => "Invalid",
        CatalogError::NotFound(_) => "NotFound",
        CatalogError::Upstream(_) => "Upstream",
        CatalogError::Storage { .. } => "Storage",
        _ => "Other",
    };
    format!("{kind}: {}", e.message())
}

/// A recorded outcome with its exception class replaced by the error kind used here.
pub fn recorded(text: &str) -> String {
    let Some((class, message)) = text.split_once(": ") else {
        return text.to_owned();
    };
    let kind = match class {
        "IllegalArgumentException" if message.starts_with("Pool not found") => "NotFound",
        "IllegalArgumentException" => "Invalid",
        "IllegalStateException" => "Storage",
        "IOException" => "Upstream",
        _ => return text.to_owned(),
    };
    format!("{kind}: {message}")
}

pub fn same_json(a: &str, b: &str) -> bool {
    fn equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
        use serde_json::Value;
        match (a, b) {
            (Value::String(x), Value::String(y)) => {
                x == y
                    || matches!(
                        (zphira_common::time::Timestamp::parse(x), zphira_common::time::Timestamp::parse(y)),
                        (Ok(p), Ok(q)) if p == q
                    )
            }
            (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equal(p, q))
            }
            (Value::Object(x), Value::Object(y)) => {
                x.keys().eq(y.keys()) && x.iter().zip(y).all(|((_, p), (_, q))| equal(p, q))
            }
            _ => a == b,
        }
    }
    let a: serde_json::Value = serde_json::from_str(a).expect("first document");
    let b: serde_json::Value = serde_json::from_str(b).expect("second document");
    equal(&a, &b)
}

pub fn list_text<T: std::fmt::Display>(items: &[T]) -> String {
    let parts: Vec<String> = items.iter().map(ToString::to_string).collect();
    format!("[{}]", parts.join(", "))
}

pub fn config(dir: &Path) -> CatalogConfig {
    CatalogConfig::new(dir).with_sleeper(Arc::new(RecordingSleeper::default()))
}

pub fn chart(
    id: i32,
    tags: &[&str],
    rating: f32,
    votes: i32,
    difficulty: f32,
    duration: Option<i32>,
) -> ChartInfo {
    let mut c = ChartInfo::default();
    c.id = id;
    c.name = Some(format!("C{id}"));
    c.tags = Some(tags.iter().map(|t| (*t).to_owned()).collect());
    c.rating = rating;
    c.rating_count = votes;
    c.difficulty = difficulty;
    c.set_duration_seconds(duration);
    c
}

pub fn chart_map_json(charts: &[ChartInfo]) -> String {
    let map: BTreeMap<i32, &ChartInfo> = charts.iter().map(|c| (c.id, c)).collect();
    serde_json::to_string_pretty(&map).unwrap()
}

pub fn not_found() -> UpstreamError {
    UpstreamError::new("HTTP 404: {\"detail\":\"Not found\"}", Some(404))
}

#[derive(Default)]
pub struct FakeUpstream {
    charts: Mutex<HashMap<i32, Result<Arc<ChartInfo>, UpstreamError>>>,
    pages: Mutex<HashMap<String, String>>,
    durations: Mutex<HashMap<i32, Result<i32, UpstreamError>>>,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    pub requests: Mutex<Vec<String>>,
    pub chart_calls: Mutex<Vec<i32>>,
    pub probe_calls: Mutex<Vec<i32>>,
    probes_running: AtomicUsize,
    pub max_probes_running: AtomicUsize,
}

#[derive(Default)]
pub struct Gate {
    pub arrived: Notify,
    open: Notify,
    opened: std::sync::atomic::AtomicBool,
}

impl Gate {
    pub fn open(&self) {
        self.opened.store(true, Ordering::SeqCst);
        self.open.notify_waiters();
    }

    async fn pass(&self) {
        self.arrived.notify_one();
        loop {
            let notified = self.open.notified();
            if self.opened.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

impl FakeUpstream {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn add_chart(&self, chart: ChartInfo) {
        self.charts
            .lock()
            .unwrap()
            .insert(chart.id, Ok(Arc::new(chart)));
    }

    pub fn add_chart_as(&self, requested: i32, chart: ChartInfo) {
        self.charts
            .lock()
            .unwrap()
            .insert(requested, Ok(Arc::new(chart)));
    }

    pub fn fail_chart(&self, id: i32, error: UpstreamError) {
        self.charts.lock().unwrap().insert(id, Err(error));
    }

    pub fn page(&self, path: &str, body: &str) {
        self.pages
            .lock()
            .unwrap()
            .insert(path.to_owned(), body.to_owned());
    }

    pub fn pages_from_fixture(&self, scenario: &str) {
        for line in read_fixture(&format!("{scenario}/stub.tsv")).lines() {
            let (path, body) = line.split_once('\t').unwrap();
            self.page(path, body);
        }
    }

    pub fn duration(&self, id: i32, result: Result<i32, UpstreamError>) {
        self.durations.lock().unwrap().insert(id, result);
    }

    pub fn gate(&self, key: &str) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        self.gates
            .lock()
            .unwrap()
            .insert(key.to_owned(), gate.clone());
        gate
    }

    async fn pass_gate(&self, key: &str) {
        let gate = self.gates.lock().unwrap().get(key).cloned();
        if let Some(gate) = gate {
            gate.pass().await;
        }
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Upstream for FakeUpstream {
    fn chart(&self, id: i32) -> BoxFuture<'_, Result<Arc<ChartInfo>, UpstreamError>> {
        Box::pin(async move {
            self.chart_calls.lock().unwrap().push(id);
            self.pass_gate(&format!("chart/{id}")).await;
            self.charts
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .unwrap_or_else(|| Err(not_found()))
        })
    }

    fn chart_page<'a>(
        &'a self,
        page: i32,
        division: Option<&'a str>,
    ) -> BoxFuture<'a, Result<ChartListPage, UpstreamError>> {
        Box::pin(async move {
            let path = match division {
                Some(d) if !d.trim().is_empty() => format!("chart?page={page}&division={d}"),
                _ => format!("chart?page={page}"),
            };
            self.requests.lock().unwrap().push(path.clone());
            self.pass_gate(&path).await;
            let body = self.pages.lock().unwrap().get(&path).cloned();
            match body {
                Some(body) => Ok(serde_json::from_str(&body).expect("stub page")),
                None => Err(not_found()),
            }
        })
    }

    fn probe_duration(&self, chart_id: i32) -> BoxFuture<'_, Result<i32, UpstreamError>> {
        Box::pin(async move {
            self.probe_calls.lock().unwrap().push(chart_id);
            let running = self.probes_running.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_probes_running.fetch_max(running, Ordering::SeqCst);
            self.pass_gate(&format!("probe/{chart_id}")).await;
            tokio::task::yield_now().await;
            self.probes_running.fetch_sub(1, Ordering::SeqCst);
            self.durations
                .lock()
                .unwrap()
                .get(&chart_id)
                .cloned()
                .unwrap_or_else(|| Err(UpstreamError::new("No audio entry in archive", None)))
        })
    }
}

#[derive(Default)]
pub struct RecordingSleeper {
    pub sleeps: Mutex<Vec<Duration>>,
}

impl Sleeper for RecordingSleeper {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        self.sleeps.lock().unwrap().push(duration);
        Box::pin(async {})
    }
}

pub struct FakeClock {
    now: Mutex<Instant>,
}

impl FakeClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Instant::now()),
        })
    }

    pub fn advance(&self, by: Duration) {
        *self.now.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
}

#[derive(Default)]
pub struct FailingFiles {
    failing: Mutex<HashSet<String>>,
    pub writes: Mutex<Vec<String>>,
}

impl FailingFiles {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn fail(&self, name: &str, fail: bool) {
        let mut failing = self.failing.lock().unwrap();
        if fail {
            failing.insert(name.to_owned());
        } else {
            failing.remove(name);
        }
    }
}

impl FileSink for FailingFiles {
    fn write_atomic(
        &self,
        path: &Path,
        write: &mut dyn FnMut(&mut dyn Write) -> io::Result<()>,
    ) -> io::Result<()> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        self.writes.lock().unwrap().push(name.clone());
        if self.failing.lock().unwrap().contains(&name) {
            return Err(io::Error::other("injected write failure"));
        }
        AtomicFiles.write_atomic(path, write)
    }
}

#[derive(Clone, Default)]
pub struct Logs(Arc<Mutex<Vec<String>>>);

impl Logs {
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

thread_local! {
    static CAPTURE: std::cell::RefCell<Option<Logs>> = const { std::cell::RefCell::new(None) };
}

// One global subscriber: per-thread defaults lose events in parallel tests.
struct CaptureLayer;

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if !event.metadata().target().starts_with("zphira_catalog") {
            return;
        }
        let level = *event.metadata().level();
        if level > Level::INFO {
            return;
        }
        CAPTURE.with(|capture| {
            if let Some(logs) = capture.borrow().as_ref() {
                let mut visitor = MessageVisitor(String::new());
                event.record(&mut visitor);
                logs.0
                    .lock()
                    .unwrap()
                    .push(format!("{level} {}", visitor.0));
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
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(CaptureLayer))
            .expect("no other global subscriber in tests");
    });
    let logs = Logs::default();
    CAPTURE.with(|capture| *capture.borrow_mut() = Some(logs.clone()));
    (CaptureGuard, logs)
}
