mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use support::*;
use zphira_catalog::files::{CACHE_FILE, DURATION_RESET_MARKER, INDEX_FILE, POOL_FILE};
use zphira_catalog::{
    CatalogConfig, CatalogError, ChartIndex, ChartPool, PROBE_FAILURE_MEMORY, PoolCategory,
    RefreshOutcome, ScreeningRule, Upstream,
};
use zphira_phira::ChartInfo;

struct Setup {
    dir: tempfile::TempDir,
    upstream: Arc<FakeUpstream>,
    sleeper: Arc<RecordingSleeper>,
    clock: Arc<FakeClock>,
    pool: ChartPool,
    index: ChartIndex,
}

async fn setup(index_fixture: Option<&str>, marker: bool) -> Setup {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        marker,
    );
    if let Some(rel) = index_fixture {
        std::fs::copy(fixture_path(rel), dir.path().join(INDEX_FILE)).unwrap();
    }
    setup_in(dir).await
}

async fn setup_in(dir: tempfile::TempDir) -> Setup {
    let upstream = FakeUpstream::new();
    let sleeper = Arc::new(RecordingSleeper::default());
    let clock = FakeClock::new();
    let cfg = CatalogConfig::new(dir.path())
        .with_sleeper(sleeper.clone())
        .with_clock(clock.clone());
    let up: Arc<dyn Upstream> = upstream.clone();
    let pool = ChartPool::load(&cfg, up.clone()).await.unwrap();
    let index = ChartIndex::load(&cfg, up, &pool).await;
    Setup {
        dir,
        upstream,
        sleeper,
        clock,
        pool,
        index,
    }
}

fn ids(charts: &[Arc<ChartInfo>]) -> Vec<i32> {
    charts.iter().map(|c| c.id).collect()
}

fn path(s: &Setup, name: &str) -> String {
    read(s.dir.path(), name)
}

#[tokio::test]
async fn saved_index_matches_java_byte_for_byte() {
    let s = setup(Some("index_save/input/chart-index.json"), true).await;
    let java = java_results("index_save");
    assert_eq!(s.index.indexed_count().to_string(), java["indexed"]);
    assert_eq!(
        java_list(&ids(&s.index.search(&ScreeningRule::regular(), 10))),
        java["search"]
    );
    s.upstream
        .page("chart?page=1&division=plain", r#"{"count":0,"results":[]}"#);
    let outcome = s.index.refresh(Some("plain")).await.unwrap();
    assert_eq!(outcome, RefreshOutcome::Refreshed { indexed: 5 });
    assert_eq!(
        path(&s, INDEX_FILE),
        read_fixture("index_save/output/chart-index.json")
    );
    assert_eq!(s.index.remote_count(), 0);
    assert!(s.sleeper.sleeps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unreadable_index_starts_empty_with_the_java_warning() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    std::fs::copy(
        fixture_path("index_unreadable/input/chart-index.json"),
        dir.path().join(INDEX_FILE),
    )
    .unwrap();
    let s = setup_in(dir).await;
    assert_eq!(
        s.index.indexed_count().to_string(),
        java_results("index_unreadable")["indexed"]
    );
    // zphira-common names the map key in the error path; the recorded line does not.
    let expected: Vec<String> = java_log("index_unreadable")
        .into_iter()
        .map(|l| l.replace("path $.)", "path $.2)"))
        .collect();
    assert_eq!(logs.lines(), expected);
    assert_eq!(
        path(&s, INDEX_FILE),
        read_fixture("index_unreadable/input/chart-index.json")
    );
    assert_eq!(s.index.remote_count(), -1);
}

#[tokio::test]
async fn sweeps_match_java() {
    for (scenario, division) in [
        ("sweep_fresh_no_plain", None),
        ("sweep_incremental", None),
        ("sweep_division", Some("plain")),
        ("sweep_empty_page", None),
    ] {
        let (_guard, logs) = capture_logs();
        let input = format!("{scenario}/input/chart-index.json");
        let has_input = fixture_path(&input).exists();
        let s = setup(has_input.then_some(input.as_str()), true).await;
        s.upstream.pages_from_fixture(scenario);
        let java = java_results(scenario);

        let outcome = s.index.refresh(division).await.unwrap();
        assert_eq!(
            format!("ok {}", outcome.indexed()),
            java["refresh"],
            "{scenario}"
        );
        assert!(matches!(outcome, RefreshOutcome::Refreshed { .. }));
        assert_eq!(
            s.upstream.requests().join(" "),
            java["requests"],
            "{scenario}"
        );
        assert_eq!(
            s.index.indexed_count().to_string(),
            java["indexed"],
            "{scenario}"
        );
        assert_eq!(
            s.index.remote_count().to_string(),
            java["remote"],
            "{scenario}"
        );
        assert_eq!(
            s.index.is_refreshing().to_string(),
            java["refreshing"],
            "{scenario}"
        );
        assert_eq!(
            path(&s, INDEX_FILE),
            read_fixture(&format!("{scenario}/output/chart-index.json")),
            "{scenario}"
        );
        assert_eq!(logs.lines(), java_log(scenario), "{scenario}");

        let sleeps = s.sleeper.sleeps.lock().unwrap().clone();
        let ms: Vec<u128> = sleeps.iter().map(Duration::as_millis).collect();
        match scenario {
            "sweep_fresh_no_plain" => assert_eq!(ms, [4000, 60, 60]),
            "sweep_incremental" => assert_eq!(ms, [60]),
            "sweep_division" => assert_eq!(ms, [60]),
            "sweep_empty_page" => assert_eq!(ms, [60]),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn a_fresh_index_is_swept_in_full() {
    let (_guard, logs) = capture_logs();
    let s = setup(None, true).await;
    s.upstream.pages_from_fixture("sweep_fresh");
    let java = java_results("sweep_fresh");

    let outcome = s.index.refresh(None).await.unwrap();
    assert_eq!(outcome, RefreshOutcome::Refreshed { indexed: 155 });
    let mut expected = vec![
        "chart?page=1&division=plain".to_owned(),
        "chart?page=2&division=plain".to_owned(),
    ];
    expected.extend((1..=50).map(|p| format!("chart?page={p}")));
    assert_eq!(s.upstream.requests(), expected);
    assert_eq!(s.index.remote_count().to_string(), java["remote"]);
    assert_eq!(
        logs.lines(),
        [
            "INFO Chart index is empty, running a full sweep (slow on first run)",
            "INFO Chart index refreshed: 155 charts (division=all)",
        ]
    );
    let ms: Vec<u128> = s
        .sleeper
        .sleeps
        .lock()
        .unwrap()
        .iter()
        .map(Duration::as_millis)
        .collect();
    let mut pauses = vec![60, 4000];
    pauses.extend(std::iter::repeat_n(60, 49));
    assert_eq!(ms, pauses);
    let java_file: std::collections::BTreeMap<i32, ChartInfo> =
        zphira_common::gson::from_str(&read_fixture("sweep_fresh/output/chart-index.json"))
            .unwrap();
    let rust_file: std::collections::BTreeMap<i32, ChartInfo> =
        zphira_common::gson::from_str(&path(&s, INDEX_FILE)).unwrap();
    assert_eq!(rust_file.len(), 155);
    for (id, chart) in &java_file {
        assert_eq!(rust_file.get(id), Some(chart), "chart {id}");
    }
}

#[tokio::test]
async fn a_non_empty_index_is_swept_incrementally() {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    index_with(
        dir.path(),
        &[chart(9999, &["regular"], 0.9, 10, 17.0, None)],
    );
    let s = setup_in(dir).await;
    s.upstream.pages_from_fixture("sweep_fresh");
    let java = java_results("sweep_fresh");
    assert_eq!(
        s.index.refresh(None).await.unwrap(),
        RefreshOutcome::Refreshed { indexed: 126 }
    );
    assert_eq!(s.upstream.requests().join(" "), java["requests"]);
    let ms: Vec<u128> = s
        .sleeper
        .sleeps
        .lock()
        .unwrap()
        .iter()
        .map(Duration::as_millis)
        .collect();
    let mut pauses = vec![60, 4000];
    pauses.extend(std::iter::repeat_n(60, 40));
    assert_eq!(ms, pauses);
}

#[tokio::test]
async fn a_failed_sweep_keeps_what_it_merged() {
    let (_guard, logs) = capture_logs();
    let s = setup(None, true).await;
    s.upstream.page(
        "chart?page=1&division=plain",
        &format!(
            "{{\"count\":9,\"results\":[{},{}]}}",
            zphira_common::gson::to_string_compact(&chart(1, &["plain"], 0.9, 10, 3.0, None))
                .unwrap(),
            zphira_common::gson::to_string_compact(&chart(2, &["plain"], 0.9, 10, 3.0, None))
                .unwrap()
        ),
    );
    let err = s.index.refresh(None).await.unwrap_err();
    assert_eq!(
        java_error(&err),
        "IOException: HTTP 404: {\"detail\":\"Not found\"}"
    );
    assert!(!s.index.is_refreshing());
    assert_eq!(s.index.indexed_count(), 2);
    let saved: std::collections::BTreeMap<i32, ChartInfo> =
        zphira_common::gson::from_str(&path(&s, INDEX_FILE)).unwrap();
    assert_eq!(saved.keys().copied().collect::<Vec<_>>(), [1, 2]);
    assert!(logs.lines().is_empty());
}

#[tokio::test]
async fn merge_keeps_probed_durations_and_takes_fresh_data() {
    let s = setup(Some("sweep_incremental/input/chart-index.json"), true).await;
    s.upstream.pages_from_fixture("sweep_incremental");
    s.index.refresh(None).await.unwrap();
    let ten = s.index.get(10).unwrap();
    assert_eq!(ten.name.as_deref(), Some("Ten v3"));
    assert_eq!(ten.duration_seconds(), Some(333));
    assert_eq!(s.index.get(11).unwrap().name.as_deref(), Some("Eleven v2"));
    assert!(
        s.index.get(30).is_none(),
        "the sweep stopped at the page with nothing new"
    );
}

#[tokio::test]
async fn a_second_refresh_does_nothing_while_one_runs() {
    let (_guard, logs) = capture_logs();
    let s = setup(Some("sweep_incremental/input/chart-index.json"), true).await;
    s.upstream.pages_from_fixture("sweep_incremental");
    s.upstream
        .add_chart(chart(5000, &["regular"], 0.9, 10, 17.0, None));
    s.upstream.duration(1, Ok(222));
    let gate = s.upstream.gate("chart?page=1");

    let arrived = gate.arrived.notified();
    let running = {
        let index = s.index.clone();
        tokio::spawn(async move { index.refresh(None).await })
    };
    arrived.await;
    assert!(s.index.is_refreshing());
    assert_eq!(
        s.index.refresh(None).await.unwrap(),
        RefreshOutcome::AlreadyRunning { indexed: 3 }
    );
    assert!(!s.index.refresh_async(Some("plain".to_owned())));
    assert_eq!(s.index.search(&ScreeningRule::configured(), 10).len(), 1);
    assert_eq!(
        s.index.probe_durations(&[s.index.get(1).unwrap()], 1).await,
        0,
        "known duration"
    );
    let eleven = s.index.get(11).unwrap();
    s.upstream.duration(11, Ok(100));
    assert_eq!(s.index.probe_durations(&[eleven], 5).await, 1);
    assert!(s.index.ensure_indexed(&[5000]).await[0].result.is_ok());
    assert_eq!(
        s.upstream.requests(),
        ["chart?page=1"],
        "nothing was queued"
    );

    gate.open();
    assert_eq!(
        running.await.unwrap().unwrap(),
        RefreshOutcome::Refreshed { indexed: 6 }
    );
    assert!(!s.index.is_refreshing());
    assert_eq!(
        s.index.get(11).unwrap().duration_seconds(),
        Some(100),
        "probe result survived the merge"
    );

    s.upstream
        .page("chart?page=1&division=plain", r#"{"count":0,"results":[]}"#);
    assert!(s.index.refresh_async(Some("plain".to_owned())));
    assert!(s.index.is_refreshing());
    while s.index.is_refreshing() {
        tokio::task::yield_now().await;
    }
    assert!(
        logs.lines()
            .contains(&"INFO Chart index refreshed: 6 charts (division=plain)".to_owned())
    );
}

#[tokio::test]
async fn a_failed_refresh_is_reported_and_clears_the_flag() {
    let (_guard, logs) = capture_logs();
    let s = setup(None, true).await;
    let err = s.index.refresh(None).await.unwrap_err();
    assert_eq!(
        java_error(&err),
        "IOException: HTTP 404: {\"detail\":\"Not found\"}"
    );
    assert!(!s.index.is_refreshing());
    assert!(!s.dir.path().join(INDEX_FILE).exists(), "nothing saved");

    assert!(s.index.refresh_async(None));
    while s.index.is_refreshing() {
        tokio::task::yield_now().await;
    }
    assert_eq!(logs.lines(), ["ERROR Chart index refresh failed"]);
}

#[tokio::test]
async fn search_orders_by_rating_then_id_and_limits() {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    write(
        dir.path(),
        INDEX_FILE,
        &chart_map_json(&[
            chart(9, &["plain"], 0.9, 10, 1.0, None),
            chart(3, &["plain"], 0.95, 10, 1.0, None),
            chart(7, &["plain"], 0.9, 10, 1.0, None),
            chart(1, &["plain"], 0.85, 10, 1.0, None),
            chart(2, &["plain"], 0.85, 4, 1.0, None),
            chart(4, &["regular"], 0.99, 10, 1.0, None),
        ]),
    );
    let s = setup_in(dir).await;
    let rule = ScreeningRule::configured();
    assert_eq!(ids(&s.index.search(&rule, usize::MAX)), [3, 7, 9, 1]);
    assert_eq!(ids(&s.index.search(&rule, 2)), [3, 7]);
    assert_eq!(ids(&s.index.search(&rule, 0)), Vec::<i32>::new());
    assert_eq!(s.index.count_matches(&rule), 4);
    assert_eq!(ids(&s.index.all()), [1, 2, 3, 4, 7, 9]);
    assert_eq!(s.index.get(4).unwrap().rating, 0.99f32);
}

fn index_with(dir: &Path, charts: &[ChartInfo]) {
    write(dir, INDEX_FILE, &chart_map_json(charts));
}

#[tokio::test]
async fn probing_spends_the_budget_on_unknown_durations_in_order() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let charts: Vec<ChartInfo> = (1..=8)
        .map(|id| chart(id, &["plain"], 0.9, 10, 1.0, (id == 2).then_some(50)))
        .collect();
    index_with(dir.path(), &charts);
    let s = setup_in(dir).await;
    for id in 1..=8 {
        s.upstream.duration(id, Ok(id * 100));
    }
    let all = s.index.all();
    logs.clear();
    assert_eq!(s.index.probe_durations(&all, 3).await, 3);
    assert_eq!(*s.upstream.probe_calls.lock().unwrap(), [1, 3, 4]);
    assert_eq!(
        logs.lines(),
        ["INFO Duration probe finished: 3/3 resolved, 0 failed"]
    );
    assert_eq!(s.index.get(3).unwrap().duration_seconds(), Some(300));
    assert!(path(&s, INDEX_FILE).contains("\"duration_seconds\": 400"));
    logs.clear();
    let done: Vec<_> = all
        .iter()
        .filter(|c| c.duration_seconds().is_some())
        .cloned()
        .collect();
    assert_eq!(s.index.probe_durations(&done, 10).await, 0);
    assert!(logs.lines().is_empty());
    assert_eq!(s.index.probe_durations(&all, 100).await, 4);
    assert!(
        s.upstream
            .max_probes_running
            .load(std::sync::atomic::Ordering::SeqCst)
            <= 4
    );
}

#[tokio::test]
async fn the_probe_budget_is_capped_at_500() {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let charts: Vec<ChartInfo> = (1..=520)
        .map(|id| chart(id, &["plain"], 0.9, 10, 1.0, None))
        .collect();
    index_with(dir.path(), &charts);
    let s = setup_in(dir).await;
    for id in 1..=520 {
        s.upstream.duration(id, Ok(1));
    }
    assert_eq!(s.index.probe_durations(&s.index.all(), 10_000).await, 500);
    assert_eq!(s.upstream.probe_calls.lock().unwrap().len(), 500);
    let running = s
        .upstream
        .max_probes_running
        .load(std::sync::atomic::Ordering::SeqCst);
    assert!((2..=4).contains(&running), "ran {running} at once");
}

#[tokio::test]
async fn failed_probes_are_skipped_for_six_hours() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let charts: Vec<ChartInfo> = (1..=4)
        .map(|id| chart(id, &["regular"], 1.0 - id as f32 * 0.01, 10, 18.0, None))
        .collect();
    index_with(dir.path(), &charts);
    let s = setup_in(dir).await;
    s.upstream.duration(3, Ok(400));
    s.upstream.duration(4, Ok(500));
    let candidates = s.index.search(&ScreeningRule::tb().without_duration(), 8);
    assert_eq!(ids(&candidates), [1, 2, 3, 4]);

    logs.clear();
    assert_eq!(s.index.probe_durations(&candidates, 2).await, 0);
    let mut lines = logs.lines();
    lines[..2].sort();
    assert_eq!(
        lines,
        [
            "WARN Duration probe failed for chart 1: No audio entry in archive",
            "WARN Duration probe failed for chart 2: No audio entry in archive",
            "INFO Duration probe finished: 0/2 resolved, 2 failed",
        ]
    );
    assert!(s.index.recently_failed(1) && s.index.recently_failed(2));
    logs.clear();
    assert_eq!(
        s.index.probe_durations(&candidates, 2).await,
        2,
        "the budget reaches 3 and 4"
    );
    let mut calls = s.upstream.probe_calls.lock().unwrap().clone();
    calls.sort();
    assert_eq!(calls, [1, 2, 3, 4]);
    assert_eq!(
        logs.lines(),
        [
            "INFO Skipped 2 chart(s) whose duration probe failed in the last 6 hours",
            "INFO Duration probe finished: 2/2 resolved, 0 failed",
        ]
    );
    assert_eq!(s.pool.list_pools().len(), 2);
    assert_eq!(s.index.search(&ScreeningRule::tb(), 10).len(), 2);

    s.clock
        .advance(PROBE_FAILURE_MEMORY - Duration::from_secs(1));
    assert_eq!(s.index.probe_durations(&candidates, 2).await, 0);
    assert_eq!(
        s.upstream.probe_calls.lock().unwrap().len(),
        4,
        "still remembered"
    );
    s.clock.advance(Duration::from_secs(1));
    assert!(!s.index.recently_failed(1));
    s.upstream.duration(1, Ok(361));
    assert_eq!(s.index.probe_durations(&candidates, 2).await, 1);
    let calls = s.upstream.probe_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 6);
    let mut last = calls[4..].to_vec();
    last.sort();
    assert_eq!(last, [1, 2], "both remembered charts are tried again");
    assert!(!s.index.recently_failed(1), "a success clears the memory");
    assert!(s.index.recently_failed(2));
}

#[tokio::test]
async fn a_probe_result_is_recorded_in_both_stores() {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    index_with(
        dir.path(),
        &[chart(42058, &["regular"], 0.85, 5, 16.9, None)],
    );
    let s = setup_in(dir).await;
    let in_index = s.index.get(42058).unwrap();
    let in_pool = s.pool.chart_info(42058).unwrap();
    assert!(!Arc::ptr_eq(&in_index, &in_pool));
    s.upstream.duration(42058, Ok(421));

    assert_eq!(s.index.probe_durations(&[in_index], 1).await, 1);
    assert_eq!(
        in_pool.duration_seconds(),
        Some(421),
        "the room's object sees it"
    );
    assert!(path(&s, INDEX_FILE).contains("\"duration_seconds\": 421"));
    let cache = path(&s, CACHE_FILE);
    let entry = &cache[cache.find("\"42058\": {").unwrap()..];
    assert!(entry[..entry.find('}').unwrap()].contains("\"duration_seconds\": 421"));
}

#[tokio::test]
async fn probe_duration_async_runs_once_in_the_background() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    index_with(
        dir.path(),
        &[chart(42058, &["regular"], 0.85, 5, 16.9, None)],
    );
    let s = setup_in(dir).await;
    s.upstream.duration(42058, Ok(222));
    let gate = s.upstream.gate("probe/42058");
    let in_room = s.pool.get_chart_info(42058).unwrap();
    let arrived = gate.arrived.notified();
    s.index.probe_duration_async(&in_room);
    s.index.probe_duration_async(&in_room);
    arrived.await;
    s.index.probe_duration_async(&in_room);
    assert_eq!(
        *s.upstream.probe_calls.lock().unwrap(),
        [42058],
        "one probe in flight"
    );
    gate.open();
    while !logs.lines().iter().any(|l| l.starts_with("INFO Probed")) {
        tokio::task::yield_now().await;
    }
    assert_eq!(logs.lines(), ["INFO Probed duration of chart 42058: 222s"]);
    assert_eq!(in_room.duration_seconds(), Some(222));
    assert_eq!(s.index.get(42058).unwrap().duration_seconds(), Some(222));
    assert!(path(&s, INDEX_FILE).contains("\"duration_seconds\": 222"));
    assert!(
        path(&s, CACHE_FILE)
            .matches("\"duration_seconds\": 222")
            .count()
            == 1
    );

    s.index.probe_duration_async(&in_room);
    tokio::task::yield_now().await;
    assert_eq!(s.upstream.probe_calls.lock().unwrap().len(), 1);

    let other = s.pool.get_chart_info(52087).unwrap();
    logs.clear();
    s.index.probe_duration_async(&other);
    while logs.lines().is_empty() {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        logs.lines(),
        ["WARN Duration probe failed for chart 52087: No audio entry in archive"]
    );
    assert!(s.index.recently_failed(52087));
    s.index.probe_duration_async(&other);
    tokio::task::yield_now().await;
    assert_eq!(*s.upstream.probe_calls.lock().unwrap(), [42058, 52087]);
}

#[tokio::test]
async fn durations_from_the_java_server_are_cleared_once() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[
            (CACHE_FILE, "pool_defaults/input/chart-info-cache.json"),
            (INDEX_FILE, "index_save/input/chart-index.json"),
        ],
        false,
    );
    let marker = dir.path().join(DURATION_RESET_MARKER);
    let s = setup_in(dir).await;
    assert!(marker.exists());
    assert!(
        s.pool
            .get_chart_info(30474)
            .unwrap()
            .duration_seconds()
            .is_none()
    );
    assert!(s.index.all().iter().all(|c| c.duration_seconds().is_none()));
    assert!(!path(&s, INDEX_FILE).contains("duration_seconds"));
    assert!(!path(&s, CACHE_FILE).contains("duration_seconds"));
    assert_eq!(
        logs.lines(),
        ["INFO Cleared 2 chart durations probed by an older server; they will be probed again"]
    );

    s.upstream.duration(1000, Ok(400));
    assert_eq!(
        s.index
            .probe_durations(&[s.index.get(1000).unwrap()], 1)
            .await,
        1
    );
    let Setup { dir, .. } = s;
    logs.clear();
    let again = setup_in(dir).await;
    assert_eq!(again.index.get(1000).unwrap().duration_seconds(), Some(400));
    assert!(logs.lines().is_empty());

    let empty = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture_path("pool_defaults/input/chart-info-cache.json"),
        empty.path().join(CACHE_FILE),
    )
    .unwrap();
    let fresh = setup_in(empty).await;
    assert!(fresh.dir.path().join(DURATION_RESET_MARKER).exists());
    assert!(!fresh.dir.path().join(INDEX_FILE).exists());
    assert!(fresh.dir.path().join(POOL_FILE).exists());
}

#[tokio::test]
async fn ensure_indexed_answers_per_chart() {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    index_with(
        dir.path(),
        &[chart(1000, &["regular"], 0.9, 10, 17.0, None)],
    );
    let s = setup_in(dir).await;
    s.upstream
        .add_chart(chart(2000, &["plain"], 0.9, 10, 3.0, None));
    s.upstream
        .add_chart_as(5000, chart(5001, &["plain"], 0.9, 10, 3.0, None));
    s.upstream.fail_chart(
        4000,
        zphira_catalog::UpstreamError::new("HTTP 503: busy", Some(503)),
    );

    let results = s
        .index
        .ensure_indexed(&[1000, 2000, 3000, 2000, 4000])
        .await;
    let summary: Vec<(i32, bool)> = results
        .iter()
        .map(|r| (r.chart_id, r.result.is_ok()))
        .collect();
    assert_eq!(
        summary,
        [
            (1000, true),
            (2000, true),
            (3000, false),
            (2000, true),
            (4000, false)
        ]
    );
    match &results[2].result {
        Err(CatalogError::Upstream(e)) => assert!(e.is_not_found()),
        other => panic!("{other:?}"),
    }
    match &results[4].result {
        Err(CatalogError::Upstream(e)) => {
            assert_eq!((e.status(), e.message()), (Some(503), "HTTP 503: busy"))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(*s.upstream.chart_calls.lock().unwrap(), [2000, 3000, 4000]);
    assert_eq!(s.index.get(2000).unwrap().name.as_deref(), Some("C2000"));
    assert!(path(&s, INDEX_FILE).contains("\"2000\": {"));
    assert_eq!(s.index.indexed_count(), 2);
    assert!(s.pool.find_pool(0).is_some());
    assert_eq!(s.pool.pool_ids_by_category(PoolCategory::Manual), [0, 1]);

    let mismatch = s.index.ensure_indexed(&[5000]).await;
    match &mismatch[0].result {
        Err(CatalogError::Upstream(e)) => {
            assert_eq!(e.message(), "Phira returned chart 5001 for request 5000");
            assert!(!e.is_not_found());
        }
        other => panic!("{other:?}"),
    }
    assert!(s.index.get(5000).is_none() && s.index.get(5001).is_none());
}

fn page_json(count: i32, charts: &[ChartInfo]) -> String {
    let rows: Vec<String> = charts
        .iter()
        .map(|c| zphira_common::gson::to_string_compact(c).unwrap())
        .collect();
    format!("{{\"count\":{count},\"results\":[{}]}}", rows.join(","))
}

#[tokio::test]
async fn a_forced_full_sweep_reaches_what_an_incremental_one_misses() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    index_with(
        dir.path(),
        &[
            chart(1, &["plain"], 0.9, 10, 3.0, Some(200)),
            chart(10, &["regular"], 0.9, 10, 17.0, Some(333)),
            chart(11, &["regular"], 0.9, 10, 17.0, None),
        ],
    );
    let s = setup_in(dir).await;
    let plain = |id| chart(id, &["plain"], 0.9, 10, 3.0, None);
    let regular = |id| chart(id, &["regular"], 0.9, 10, 17.0, None);
    s.upstream.page(
        "chart?page=1&division=plain",
        &page_json(3, &[plain(1), plain(2)]),
    );
    s.upstream
        .page("chart?page=2&division=plain", &page_json(3, &[plain(3)]));
    s.upstream
        .page("chart?page=1", &page_json(6, &[regular(10), regular(11)]));
    s.upstream
        .page("chart?page=2", &page_json(6, &[regular(12), regular(13)]));
    s.upstream
        .page("chart?page=3", &page_json(6, &[regular(14), regular(15)]));

    assert_eq!(
        s.index.refresh(None).await.unwrap(),
        RefreshOutcome::Refreshed { indexed: 3 }
    );
    assert_eq!(s.upstream.requests(), ["chart?page=1"]);
    s.upstream.requests.lock().unwrap().clear();
    s.sleeper.sleeps.lock().unwrap().clear();
    logs.clear();

    assert_eq!(
        s.index.refresh_full().await.unwrap(),
        RefreshOutcome::Refreshed { indexed: 9 }
    );
    assert_eq!(
        s.upstream.requests(),
        [
            "chart?page=1&division=plain",
            "chart?page=2&division=plain",
            "chart?page=1",
            "chart?page=2",
            "chart?page=3",
        ]
    );
    let ms: Vec<u128> = s
        .sleeper
        .sleeps
        .lock()
        .unwrap()
        .iter()
        .map(Duration::as_millis)
        .collect();
    assert_eq!(
        ms,
        [60, 4000, 60, 60],
        "plain p1, the pause, default p1 and p2"
    );
    assert_eq!(
        logs.lines(),
        [
            "INFO Running a full chart index sweep on request",
            "INFO Chart index refreshed: 9 charts (division=all)",
        ]
    );
    assert_eq!(ids(&s.index.all()), [1, 2, 3, 10, 11, 12, 13, 14, 15]);
    assert_eq!(s.index.get(1).unwrap().duration_seconds(), Some(200));
    assert_eq!(s.index.get(10).unwrap().duration_seconds(), Some(333));
    let saved: std::collections::BTreeMap<i32, ChartInfo> =
        zphira_common::gson::from_str(&path(&s, INDEX_FILE)).unwrap();
    assert_eq!(saved.len(), 9);
    assert_eq!(saved[&10].duration_seconds(), Some(333));
}

#[tokio::test]
async fn a_forced_full_sweep_runs_alone_and_saves_on_failure() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    index_with(dir.path(), &[chart(1, &["plain"], 0.9, 10, 3.0, None)]);
    let s = setup_in(dir).await;
    let plain = |id| chart(id, &["plain"], 0.9, 10, 3.0, None);
    s.upstream.page(
        "chart?page=1&division=plain",
        &page_json(2, &[plain(1), plain(2)]),
    );
    let gate = s.upstream.gate("chart?page=1&division=plain");
    let arrived = gate.arrived.notified();
    let running = {
        let index = s.index.clone();
        tokio::spawn(async move { index.refresh_full().await })
    };
    arrived.await;
    assert!(s.index.is_refreshing());
    assert!(!s.index.refresh_full_async());
    assert!(!s.index.refresh_async(None));
    assert_eq!(
        s.index.refresh_full().await.unwrap(),
        RefreshOutcome::AlreadyRunning { indexed: 1 }
    );
    assert_eq!(
        s.index.refresh(None).await.unwrap(),
        RefreshOutcome::AlreadyRunning { indexed: 1 }
    );
    gate.open();
    let err = running.await.unwrap().unwrap_err();
    assert_eq!(
        java_error(&err),
        "IOException: HTTP 404: {\"detail\":\"Not found\"}"
    );
    assert!(!s.index.is_refreshing());
    let saved: std::collections::BTreeMap<i32, ChartInfo> =
        zphira_common::gson::from_str(&path(&s, INDEX_FILE)).unwrap();
    assert_eq!(saved.keys().copied().collect::<Vec<_>>(), [1, 2]);

    logs.clear();
    assert!(s.index.refresh_full_async());
    while s.index.is_refreshing() {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        logs.lines(),
        [
            "INFO Running a full chart index sweep on request",
            "ERROR Chart index refresh failed",
        ]
    );
}
