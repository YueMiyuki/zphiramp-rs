use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;
use tokio::sync::Notify;
use zphira_catalog::{
    BoxFuture, CatalogConfig, ChartIndex, ChartPool, Sleeper, Submissions, Upstream, UpstreamError,
};
use zphira_common::args::ServerArgs;
use zphira_common::gson;
use zphira_common::i18n::I18n;
use zphira_game::{GameContext, GameDeps, GameOptions, GameUpstream};
use zphira_ledger::{Admins, Points, RoundRecords};
use zphira_phira::{ChartInfo, ChartListPage, GameRecord, UserInfo};

use crate::console::commands::{Commands, Flow, Out, Services, normalize};

pub fn chart(
    id: i32,
    name: &str,
    level: &str,
    tag: &str,
    rating: f32,
    votes: i32,
) -> Arc<ChartInfo> {
    Arc::new(
        gson::from_str(&format!(
            r#"{{"id":{id},"name":"{name}","level":"{level}","difficulty":17.5,"tags":["{tag}"],"rating":{rating},"ratingCount":{votes}}}"#
        ))
        .unwrap(),
    )
}

#[derive(Default)]
pub struct FakeUpstream {
    charts: Mutex<HashMap<i32, Arc<ChartInfo>>>,
    durations: Mutex<HashMap<i32, i32>>,
    hold_pages: AtomicBool,
    release_pages: Notify,
    pages: Mutex<Vec<Option<String>>>,
}

impl FakeUpstream {
    pub fn add(&self, chart: Arc<ChartInfo>) {
        self.charts.lock().unwrap().insert(chart.id, chart);
    }

    pub fn page_divisions(&self) -> Vec<Option<String>> {
        self.pages.lock().unwrap().clone()
    }

    pub fn set_duration(&self, chart_id: i32, seconds: i32) {
        self.durations.lock().unwrap().insert(chart_id, seconds);
    }

    pub fn hold_pages(&self) {
        self.hold_pages.store(true, Ordering::SeqCst);
    }

    pub fn release_pages(&self) {
        self.hold_pages.store(false, Ordering::SeqCst);
        self.release_pages.notify_waiters();
    }
}

impl Upstream for FakeUpstream {
    fn chart(&self, id: i32) -> BoxFuture<'_, Result<Arc<ChartInfo>, UpstreamError>> {
        let found = self.charts.lock().unwrap().get(&id).cloned();
        Box::pin(async move {
            found.ok_or_else(|| UpstreamError::new(format!("HTTP 404: chart {id}"), Some(404)))
        })
    }

    fn chart_page<'a>(
        &'a self,
        _page: i32,
        division: Option<&'a str>,
    ) -> BoxFuture<'a, Result<ChartListPage, UpstreamError>> {
        self.pages.lock().unwrap().push(division.map(str::to_owned));
        Box::pin(async move {
            while self.hold_pages.load(Ordering::SeqCst) {
                let released = self.release_pages.notified();
                tokio::pin!(released);
                released.as_mut().enable();
                if !self.hold_pages.load(Ordering::SeqCst) {
                    break;
                }
                released.await;
            }
            Ok(ChartListPage::default())
        })
    }

    fn probe_duration(&self, chart_id: i32) -> BoxFuture<'_, Result<i32, UpstreamError>> {
        let found = self.durations.lock().unwrap().get(&chart_id).copied();
        Box::pin(async move {
            found.ok_or_else(|| UpstreamError::new(format!("no audio for {chart_id}"), None))
        })
    }
}

impl GameUpstream for FakeUpstream {
    fn user_by_token<'a>(
        &'a self,
        _token: &'a str,
    ) -> BoxFuture<'a, Result<Arc<UserInfo>, UpstreamError>> {
        Box::pin(async { Err(UpstreamError::new("HTTP 401: unauthorized", Some(401))) })
    }

    fn record(&self, id: i32) -> BoxFuture<'_, Result<Arc<GameRecord>, UpstreamError>> {
        Box::pin(async move {
            Err(UpstreamError::new(
                format!("HTTP 404: record {id}"),
                Some(404),
            ))
        })
    }

    fn recent_records(
        &self,
        _player: i32,
    ) -> BoxFuture<'_, Result<Arc<Vec<GameRecord>>, UpstreamError>> {
        Box::pin(async { Ok(Arc::new(Vec::new())) })
    }
}

struct NoSleep;

impl Sleeper for NoSleep {
    fn sleep(&self, _duration: Duration) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}

#[derive(Default)]
pub struct Recorder(Mutex<Vec<String>>);

impl Recorder {
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

impl Out for Recorder {
    fn info(&self, message: String) {
        self.0.lock().unwrap().push(format!("INFO {message}"));
    }

    fn warn(&self, message: String) {
        self.0.lock().unwrap().push(format!("WARN {message}"));
    }
}

pub struct Fixture {
    pub dir: TempDir,
    pub upstream: Arc<FakeUpstream>,
    pub out: Arc<Recorder>,
    pub game: GameContext,
    pub pools: ChartPool,
    pub index: ChartIndex,
    pub submissions: Submissions,
    pub points: Points,
    pub rounds: RoundRecords,
    pub admins: Admins,
    pub cmds: Commands,
}

const POOLS_JSON: &str = r#"{"pools":[
{"id":1,"favorite_id":77,"default_flag":true,"chart_ids":[1001,1002,1003],"category":"MANUAL","rounds_per_stay":2,"order":10,"submission_open":false},
{"id":2,"default_flag":true,"chart_ids":[2001],"category":"REGULAR","order":20,"submission_open":false},
{"id":3,"default_flag":false,"chart_ids":[3001],"category":"TB","order":30,"submission_open":true},
{"id":4,"default_flag":false,"chart_ids":[],"category":"CONFIGURED","order":40,"submission_open":false}
]}"#;

impl Fixture {
    pub async fn new() -> Self {
        Self::with_options(GameOptions::default()).await
    }

    pub async fn with_options(options: GameOptions) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        std::fs::write(data.join("chart-pools.json"), POOLS_JSON).unwrap();
        std::fs::write(data.join("admins.json"), "[105]").unwrap();

        let upstream = Arc::new(FakeUpstream::default());
        let charts = [
            chart(1001, "Spasmodic", "IN Lv.15", "regular", 0.95, 10),
            chart(1002, "Rain", "AT Lv.17", "regular", 0.90, 8),
            chart(1003, "Zero", "HD Lv.12", "regular", 0.70, 2),
            chart(2001, "Two", "IN Lv.16", "regular", 0.88, 9),
            chart(3001, "Long One", "IN Lv.14", "plain", 0.91, 12),
            chart(5001, "Plain A", "AT Lv.17", "plain", 0.99, 20),
            chart(5002, "Plain B", "AT Lv.18", "plain", 0.85, 6),
            chart(5003, "Plain C", "AT Lv.15", "plain", 0.5, 1),
        ];
        let mut index_rows = Vec::new();
        for c in &charts {
            upstream.add(c.clone());
            index_rows.push(format!(
                "\"{}\":{}",
                c.id,
                gson::to_string_compact(&**c).unwrap()
            ));
        }
        std::fs::write(
            data.join("chart-index.json"),
            format!("{{{}}}", index_rows.join(",")),
        )
        .unwrap();

        let config = CatalogConfig::new(data).with_sleeper(Arc::new(NoSleep));
        let upstream_dyn: Arc<dyn Upstream> = upstream.clone();
        let pools = ChartPool::load(&config, upstream_dyn.clone())
            .await
            .unwrap();
        let index = ChartIndex::load(&config, upstream_dyn, &pools).await;
        let submissions = Submissions::load(&config).await;
        let points = Points::load(data).unwrap();
        let rounds = RoundRecords::load(data).unwrap();
        let admins = Admins::load(data).unwrap();
        let game = GameContext::new(GameDeps {
            i18n: I18n::new("zh-CN"),
            args: ServerArgs::default(),
            upstream: upstream.clone(),
            catalog: pools.clone(),
            index: Some(index.clone()),
            points: points.clone(),
            rounds: rounds.clone(),
            data_dir: data.to_path_buf(),
            options,
        });
        let out = Arc::new(Recorder::default());
        let cmds = Commands::new(
            Services {
                game: game.clone(),
                pools: pools.clone(),
                index: index.clone(),
                submissions: submissions.clone(),
                admins: admins.clone(),
            },
            out.clone(),
        );
        Self {
            dir,
            upstream,
            out,
            game,
            pools,
            index,
            submissions,
            points,
            rounds,
            admins,
            cmds,
        }
    }

    pub async fn run(&self, line: &str) -> Vec<String> {
        self.run_flow(line).await.1
    }

    pub async fn run_flow(&self, line: &str) -> (Flow, Vec<String>) {
        self.out.take();
        let flow = match normalize(line) {
            Some(command) => self.cmds.run(command).await,
            None => Flow::Continue,
        };
        (flow, self.out.take())
    }
}

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

pub type Captured = (Level, String, String);

static CAPTURED: Mutex<Vec<Captured>> = Mutex::new(Vec::new());

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

struct Capture;

impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut message = Message(String::new());
        event.record(&mut message);
        CAPTURED.lock().unwrap().push((
            *event.metadata().level(),
            event.metadata().target().to_owned(),
            message.0,
        ));
    }
}

pub fn install_capture() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Capture))
            .unwrap();
    });
}

pub fn captured() -> Vec<Captured> {
    CAPTURED.lock().unwrap().clone()
}
