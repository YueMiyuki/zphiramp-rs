mod support;

use std::fmt::Display;
use std::sync::Arc;
use std::time::{Duration, Instant};

use support::*;
use zphira_catalog::files::{CACHE_FILE, INDEX_FILE, POOL_FILE};
use zphira_catalog::{
    CatalogError, ChartIndex, ChartPool, PoolCategory, PoolSnapshot, ScreeningRule, Upstream,
};

fn pools_line(pools: &[Arc<PoolSnapshot>]) -> String {
    pools
        .iter()
        .map(|p| {
            format!(
                "{}:{}:{}:{}:{}",
                p.id,
                p.order,
                p.category,
                p.default_flag,
                list_text(&p.chart_ids)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn done(r: Result<(), CatalogError>) -> String {
    match r {
        Ok(()) => "ok".to_owned(),
        Err(e) => error_text(&e),
    }
}

fn attempt<T: Display>(r: Result<T, CatalogError>) -> String {
    match r {
        Ok(v) => format!("ok {v}"),
        Err(e) => error_text(&e),
    }
}

fn unit(r: Result<(), CatalogError>) -> Result<&'static str, CatalogError> {
    r.map(|()| "")
}

fn list(r: Result<Vec<i32>, CatalogError>) -> Result<String, CatalogError> {
    r.map(|ids| list_text(&ids))
}

async fn load(dir: &std::path::Path, upstream: Arc<dyn Upstream>) -> ChartPool {
    ChartPool::load(&config(dir), upstream).await.unwrap()
}

#[tokio::test]
async fn built_in_defaults_match_java() {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let pool = load(dir.path(), FakeUpstream::new()).await;
    let java = java_results("pool_defaults");

    assert_eq!(
        read(dir.path(), POOL_FILE),
        read_fixture("pool_defaults/output/chart-pools.json")
    );
    assert!(same_json(
        &read(dir.path(), CACHE_FILE),
        &read_fixture("pool_defaults/output/chart-info-cache.json")
    ));
    assert_eq!(pools_line(&pool.list_pools()), java["listPools"]);
    assert_eq!(pools_line(&pool.get_default_pools()), java["defaultPools"]);
    assert_eq!(
        pool.chart_info(30474)
            .unwrap()
            .duration_seconds()
            .map(|d| d.to_string())
            .unwrap(),
        java["chart30474"]
    );
    assert_eq!(
        format!(
            "{} {}",
            pool.is_chart_info_loaded(42058),
            pool.is_chart_info_loaded(1)
        ),
        java["isLoaded"]
    );
    assert!(pool.chart_info(1).is_none());
    assert_eq!(pool.export_json().unwrap(), read(dir.path(), POOL_FILE));
}

#[tokio::test]
async fn built_in_defaults_fetch_missing_charts() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = FakeUpstream::new();
    for id in [30474, 42058, 33530, 52087] {
        upstream.add_chart(chart(id, &["regular"], 0.9, 10, 17.0, None));
    }
    let pool = load(dir.path(), upstream.clone()).await;
    assert_eq!(
        *upstream.chart_calls.lock().unwrap(),
        [30474, 42058, 33530, 52087]
    );
    assert!(pool.is_chart_info_loaded(52087));
    assert!(read(dir.path(), CACHE_FILE).contains("\"52087\": {"));

    let dir = tempfile::tempdir().unwrap();
    let err = ChartPool::load(&config(dir.path()), FakeUpstream::new())
        .await
        .err()
        .unwrap();
    assert_eq!(
        error_text(&err),
        "Upstream: HTTP 404: {\"detail\":\"Not found\"}"
    );
}

#[tokio::test]
async fn load_normalizes_like_java() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[
            (CACHE_FILE, "pool_normalize/input/chart-info-cache.json"),
            (POOL_FILE, "pool_normalize/input/chart-pools.json"),
        ],
        true,
    );
    let pool = load(dir.path(), FakeUpstream::new()).await;
    let java = java_results("pool_normalize");
    assert_eq!(
        read(dir.path(), POOL_FILE),
        read_fixture("pool_normalize/output/chart-pools.json")
    );
    assert!(same_json(
        &read(dir.path(), CACHE_FILE),
        &read_fixture("pool_normalize/output/chart-info-cache.json")
    ));
    assert_eq!(pools_line(&pool.list_pools()), java["listPools"]);
    assert_eq!(pools_line(&pool.get_default_pools()), java["defaultPools"]);
    assert_eq!(
        pools_line(&pool.resolve_pools(&[8, 5, 7, 8, 99, 2])),
        java["resolve"]
    );
    assert_eq!(
        list_text(&pool.pool_ids_by_category(PoolCategory::Manual)),
        java["byCategory"]
    );
    assert_eq!(pools_line(&pool.list_open_for_submission()), java["open"]);
    assert_eq!(logs.lines(), java_log("pool_normalize"));
}

#[tokio::test]
async fn load_edge_cases_match_java() {
    let java = java_results("pool_load_edges");
    for case in [
        "null_pools",
        "empty_file",
        "null_doc",
        "empty_object",
        "empty_list",
        "only_null_rows",
        "truncated",
        "bad_chart_id",
        "null_chart_id",
    ] {
        let dir = data_dir(
            &[
                (CACHE_FILE, "pool_defaults/input/chart-info-cache.json"),
                (POOL_FILE, &format!("pool_load_edges/input/{case}.json")),
            ],
            true,
        );
        let result = ChartPool::load(&config(dir.path()), FakeUpstream::new()).await;
        let expected = &java[case];
        match result {
            Ok(pool) => {
                assert_eq!(
                    format!("ok {}", pools_line(&pool.list_pools())),
                    *expected,
                    "{case}"
                );
                assert_eq!(
                    read(dir.path(), POOL_FILE),
                    read_fixture(&format!("pool_load_edges/output/{case}.json")),
                    "{case}"
                );
            }
            Err(e) => {
                assert!(
                    !expected.starts_with("ok"),
                    "{case}: Rust failed with {e}, Java: {expected}"
                );
                match case {
                    "truncated" | "bad_chart_id" | "null_chart_id" => {
                        assert!(matches!(e, CatalogError::Storage { .. }), "{case}: {e:?}")
                    }
                    _ => assert_eq!(error_text(&e), recorded(expected), "{case}"),
                }
                assert_eq!(
                    read(dir.path(), POOL_FILE),
                    read_fixture(&format!("pool_load_edges/input/{case}.json")),
                    "{case}"
                );
            }
        }
    }
}

#[tokio::test]
async fn operations_match_java() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[
            (CACHE_FILE, "pool_ops/input/chart-info-cache.json"),
            (INDEX_FILE, "pool_ops/input/chart-index.json"),
        ],
        true,
    );
    let upstream = FakeUpstream::new();
    let up: Arc<dyn Upstream> = upstream.clone();
    let cfg = config(dir.path());
    let pool = ChartPool::load(&cfg, up.clone()).await.unwrap();
    let index = ChartIndex::load(&cfg, up, &pool).await;
    let java = java_results("pool_ops");
    let mut rust = std::collections::BTreeMap::new();
    let mut put = |k: &str, v: String| {
        rust.insert(k.to_owned(), v);
    };

    put(
        "addPool10",
        attempt(unit(
            pool.add_pool_with(
                10,
                &[30474, 42058, 30474],
                Some(PoolCategory::Regular),
                Some(15),
                Some(3),
                None,
            )
            .await,
        )),
    );
    put(
        "addPoolDup",
        attempt(unit(
            pool.add_pool_with(10, &[], None, None, None, None).await,
        )),
    );
    put("addPoolEmpty", attempt(unit(pool.add_pool(20, &[]).await)));
    put(
        "emptyPools",
        attempt(list(
            pool.create_empty_pools(2, Some(PoolCategory::Tb), Some(15), Some(1))
                .await,
        )),
    );
    put(
        "emptyPoolsNull",
        attempt(list(pool.create_empty_pools(1, None, None, None).await)),
    );
    put(
        "emptyPools0",
        attempt(list(pool.create_empty_pools(0, None, None, None).await)),
    );
    put(
        "emptyPools51",
        attempt(list(pool.create_empty_pools(51, None, None, None).await)),
    );
    put(
        "update10",
        done(
            pool.update_pool(
                10,
                Some(PoolCategory::Configured),
                Some(-1),
                Some(5),
                Some(7),
                Some(true),
            )
            .await,
        ),
    );
    put(
        "update10b",
        done(
            pool.update_pool(10, None, Some(9), None, Some(-3), None)
                .await,
        ),
    );
    put(
        "update1",
        done(
            pool.update_pool(1, None, None, Some(-1), None, Some(false))
                .await,
        ),
    );
    put(
        "updateMissing",
        done(pool.update_pool(99, None, None, None, None, None).await),
    );
    put("favorite0", done(pool.set_favorite_id(0, Some(42)).await));
    put("favorite10", done(pool.set_favorite_id(10, None).await));
    put("default1", done(pool.set_default_flag(1, true).await));
    put("removeChart", done(pool.remove_chart(0, 42058).await));
    put("removeChartAbsent", done(pool.remove_chart(0, 1).await));
    put("removeChartMissing", done(pool.remove_chart(99, 1).await));
    put(
        "addCharts",
        attempt(pool.add_charts(1, &[30474, 33530, 30474, 42058]).await),
    );
    put("addChartsNone", attempt(pool.add_charts(1, &[30474]).await));
    put(
        "addChartsMissingPool",
        attempt(pool.add_charts(99, &[30474]).await),
    );
    let many: Vec<i32> = (0..501).map(|i| 100_000 + i).collect();
    put("addCharts501", attempt(pool.add_charts(1, &many).await));
    let gen_ids = |r: Result<zphira_catalog::GenerateOutcome, CatalogError>| {
        r.map(|o| list_text(&o.pool_ids))
    };
    put(
        "generateBad",
        attempt(gen_ids(
            pool.generate_pools(&index, &ScreeningRule::regular(), 0, None)
                .await,
        )),
    );
    put(
        "generate",
        attempt(gen_ids(
            pool.generate_pools(&index, &ScreeningRule::regular(), 2, Some(3))
                .await,
        )),
    );
    put(
        "generateTb",
        attempt(gen_ids(
            pool.generate_pools(&index, &ScreeningRule::tb(), 15, None)
                .await,
        )),
    );
    put(
        "generateNone",
        attempt(gen_ids(
            pool.generate_pools(&index, &ScreeningRule::configured(), 15, None)
                .await,
        )),
    );
    put(
        "byCategory",
        list_text(&pool.pool_ids_by_category(PoolCategory::Regular)),
    );
    put("open", pools_line(&pool.list_open_for_submission()));
    put("isOpen10", attempt(pool.is_submission_open(10)));
    put("isOpenMissing", attempt(pool.is_submission_open(99)));
    put("removePoolMissing", done(pool.remove_pool(99).await));
    put("removePool20", done(pool.remove_pool(20).await));
    put("default1off", done(pool.set_default_flag(1, false).await));
    put("removeLastDefault0", done(pool.remove_pool(0).await));
    put("default1on", done(pool.set_default_flag(1, true).await));
    put("removeDefault0", done(pool.remove_pool(0).await));
    put(
        "default1unchanged",
        done(pool.set_default_flag(1, true).await),
    );
    put("listPools", pools_line(&pool.list_pools()));
    put("defaultPools", pools_line(&pool.get_default_pools()));
    assert_eq!(
        read(dir.path(), POOL_FILE),
        read_fixture("pool_ops/output/chart-pools.json")
    );
    assert!(same_json(
        &read(dir.path(), CACHE_FILE),
        &read_fixture("pool_ops/output/chart-info-cache.json")
    ));
    let remaining = pool.list_pools();
    for p in &remaining[1..] {
        put(
            &format!("removeRest{}", p.id),
            done(pool.remove_pool(p.id).await),
        );
    }
    put("removeLast", done(pool.remove_pool(remaining[0].id).await));
    put("finalPools", pools_line(&pool.list_pools()));
    assert_eq!(
        read(dir.path(), POOL_FILE),
        read_fixture("pool_ops/final/chart-pools.json")
    );

    for (key, expected) in &java {
        assert_eq!(rust.get(key), Some(&recorded(expected)), "result {key}");
    }
    assert_eq!(rust.len(), java.len());
    assert_eq!(logs.lines(), java_log("pool_ops"));
    assert!(upstream.chart_calls.lock().unwrap().is_empty());
}

async fn two_pools(upstream: Arc<FakeUpstream>) -> (tempfile::TempDir, ChartPool) {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let pool = load(dir.path(), upstream).await;
    (dir, pool)
}

#[tokio::test]
async fn batch_add_is_all_or_nothing() {
    let upstream = FakeUpstream::new();
    upstream.add_chart(chart(777, &["regular"], 0.9, 10, 17.0, None));
    let (dir, pool) = two_pools(upstream.clone()).await;
    let before = read(dir.path(), POOL_FILE);
    let cache_before = read(dir.path(), CACHE_FILE);

    let err = pool
        .add_charts(0, &[52087, 777, 999, 778])
        .await
        .unwrap_err();
    assert_eq!(
        error_text(&err),
        "Upstream: HTTP 404: {\"detail\":\"Not found\"}"
    );
    assert!(matches!(err, CatalogError::Upstream(_)));
    assert_eq!(*upstream.chart_calls.lock().unwrap(), [777, 999]);
    assert_eq!(pool.find_pool(0).unwrap().chart_ids, [30474, 42058]);
    assert_eq!(read(dir.path(), POOL_FILE), before);
    assert_eq!(read(dir.path(), CACHE_FILE), cache_before);
    assert!(!pool.is_chart_info_loaded(777));

    upstream.add_chart(chart(999, &["plain"], 0.9, 10, 3.0, None));
    upstream.add_chart(chart(778, &["plain"], 0.9, 10, 3.0, None));
    assert_eq!(
        pool.add_charts(0, &[52087, 777, 999, 778, 777])
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        pool.find_pool(0).unwrap().chart_ids,
        [30474, 42058, 52087, 777, 999, 778]
    );
    for id in [777, 999, 778] {
        assert!(pool.is_chart_info_loaded(id));
        assert!(read(dir.path(), CACHE_FILE).contains(&format!("\"{id}\": {{")));
    }
    assert_eq!(pool.export_json().unwrap(), read(dir.path(), POOL_FILE));
}

#[tokio::test]
async fn add_pool_rules() {
    let upstream = FakeUpstream::new();
    upstream.add_chart(chart(5, &["plain"], 0.9, 10, 3.0, None));
    let (dir, pool) = two_pools(upstream.clone()).await;
    let err = pool.add_pool(1, &[404]).await.unwrap_err();
    assert_eq!(error_text(&err), "Invalid: Pool already exists: 1");
    assert!(upstream.chart_calls.lock().unwrap().is_empty());
    assert!(pool.add_pool(7, &[5, 404]).await.is_err());
    assert!(pool.find_pool(7).is_none());
    pool.add_pool_with(7, &[5, 5], Some(PoolCategory::Tb), Some(15), None, None)
        .await
        .unwrap();
    let created = pool.find_pool(7).unwrap();
    assert_eq!(created.chart_ids, [5]);
    assert_eq!(created.category, PoolCategory::Tb);
    assert_eq!(created.order, 10, "max(order) + 10");
    assert!(!created.default_flag && !created.submission_open);
    assert!(read(dir.path(), POOL_FILE).contains("\"id\": 7"));
}

#[tokio::test]
async fn update_pool_negative_numbers_reset_and_order_clamps() {
    let (_dir, pool) = two_pools(FakeUpstream::new()).await;
    pool.update_pool(
        1,
        Some(PoolCategory::Tb),
        Some(15),
        Some(2),
        Some(30),
        Some(true),
    )
    .await
    .unwrap();
    let p = pool.find_pool(1).unwrap();
    assert_eq!(
        (
            p.category,
            p.size_limit,
            p.rounds_per_stay,
            p.order,
            p.submission_open
        ),
        (PoolCategory::Tb, Some(15), Some(2), 30, true)
    );
    pool.update_pool(1, None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(*pool.find_pool(1).unwrap(), *p, "None keeps every field");
    pool.update_pool(1, None, Some(-1), Some(-7), Some(-5), Some(false))
        .await
        .unwrap();
    let p = pool.find_pool(1).unwrap();
    assert_eq!(
        (p.size_limit, p.rounds_per_stay, p.order, p.submission_open),
        (None, None, 0, false)
    );
    pool.update_pool(1, None, Some(0), Some(0), None, None)
        .await
        .unwrap();
    let p = pool.find_pool(1).unwrap();
    assert_eq!(
        (p.size_limit, p.rounds_per_stay),
        (Some(0), Some(0)),
        "zero is kept"
    );
}

fn regular_index_charts(count: i32) -> Vec<zphira_phira::ChartInfo> {
    (0..count)
        .map(|i| {
            chart(
                5000 + i,
                &["regular"],
                0.99 - i as f32 * 0.0001,
                10,
                18.0,
                None,
            )
        })
        .collect()
}

async fn pool_and_index(
    charts: &[zphira_phira::ChartInfo],
) -> (tempfile::TempDir, ChartPool, ChartIndex) {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    write(dir.path(), INDEX_FILE, &chart_map_json(charts));
    let upstream: Arc<dyn Upstream> = FakeUpstream::new();
    let cfg = config(dir.path());
    let pool = ChartPool::load(&cfg, upstream.clone()).await.unwrap();
    let index = ChartIndex::load(&cfg, upstream, &pool).await;
    (dir, pool, index)
}

#[tokio::test]
async fn generation_slices_best_rated_first_and_is_idempotent() {
    let (dir, pool, index) = pool_and_index(&regular_index_charts(5)).await;
    let rule = ScreeningRule::regular();
    let outcome = pool
        .generate_pools(&index, &rule, 2, Some(3))
        .await
        .unwrap();
    assert_eq!(outcome.pool_ids, [2, 3, 4]);
    assert_eq!(outcome.leftover, 0);
    let slices: Vec<Vec<i32>> = outcome
        .pool_ids
        .iter()
        .map(|id| pool.find_pool(*id).unwrap().chart_ids.clone())
        .collect();
    assert_eq!(slices, [vec![5000, 5001], vec![5002, 5003], vec![5004]]);
    let first = pool.find_pool(2).unwrap();
    assert_eq!(
        (
            first.category,
            first.size_limit,
            first.rounds_per_stay,
            first.order
        ),
        (PoolCategory::Regular, Some(2), Some(3), 10)
    );
    assert_eq!(pool.find_pool(4).unwrap().order, 30);
    assert!(pool.is_chart_info_loaded(5004));
    assert!(read(dir.path(), CACHE_FILE).contains("\"5004\": {"));

    let again = pool
        .generate_pools(&index, &rule, 2, Some(3))
        .await
        .unwrap();
    assert_eq!(again, zphira_catalog::GenerateOutcome::default());
    assert_eq!(pool.list_pools().len(), 5);

    pool.add_pool_with(50, &[5000], Some(PoolCategory::Tb), None, None, None)
        .await
        .unwrap();
    pool.remove_chart(2, 5001).await.unwrap();
    let third = pool.generate_pools(&index, &rule, 2, None).await.unwrap();
    assert_eq!(third.pool_ids, [51]);
    assert_eq!(pool.find_pool(51).unwrap().chart_ids, [5001]);

    let err = pool
        .generate_pools(&index, &rule, 0, None)
        .await
        .unwrap_err();
    assert_eq!(
        error_text(&err),
        "Invalid: Pool size limit must be positive"
    );
}

#[tokio::test]
async fn generation_stops_at_200_pools_and_reports_the_leftover() {
    let (_guard, logs) = capture_logs();
    let (_dir, pool, index) = pool_and_index(&regular_index_charts(203)).await;
    logs.clear();
    let outcome = pool
        .generate_pools(&index, &ScreeningRule::regular(), 1, None)
        .await
        .unwrap();
    assert_eq!(outcome.pool_ids.len(), 200);
    assert_eq!(outcome.pool_ids[0], 2);
    assert_eq!(outcome.leftover, 3);
    assert_eq!(
        logs.lines(),
        [
            "WARN Stopped at 200 generated pools; 3 charts were left over. Raise sizeLimit or generate in several passes.",
            "INFO Generated 200 REGULAR pools from 203 charts (sizeLimit=1)",
        ]
    );
    let next = pool
        .generate_pools(&index, &ScreeningRule::regular(), 1, None)
        .await
        .unwrap();
    assert_eq!(next.pool_ids, [202, 203, 204]);
    assert_eq!(next.leftover, 0);
    assert_eq!(pool.find_pool(204).unwrap().chart_ids, [5202]);
}

#[tokio::test]
async fn every_published_chart_has_its_info_loaded() {
    let (_dir, pool, index) = pool_and_index(&regular_index_charts(7)).await;
    pool.generate_pools(&index, &ScreeningRule::regular(), 3, None)
        .await
        .unwrap();
    for snapshot in pool.list_pools() {
        let published = pool.find_pool(snapshot.id).unwrap();
        assert!(
            published
                .chart_ids
                .iter()
                .all(|&c| pool.is_chart_info_loaded(c))
        );
        let infos: Vec<_> = published
            .chart_ids
            .iter()
            .map(|&c| pool.chart_info(c).unwrap())
            .collect();
        assert!(
            published
                .chart_ids
                .iter()
                .all(|&c| ChartPool::contains(c, &infos))
        );
    }
    assert!(!ChartPool::contains(1, &[]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_fetch_does_not_delay_the_room_path() {
    let upstream = FakeUpstream::new();
    upstream.add_chart(chart(777, &["regular"], 0.9, 10, 17.0, None));
    let gate = upstream.gate("chart/777");
    let (_dir, pool) = two_pools(upstream.clone()).await;

    let arrived = gate.arrived.notified();
    let adder = {
        let pool = pool.clone();
        tokio::spawn(async move { pool.add_charts(0, &[777]).await })
    };
    tokio::time::timeout(Duration::from_secs(5), arrived)
        .await
        .expect("fetch started");

    let reader = {
        let pool = pool.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            for _ in 0..1000 {
                let current = pool.find_pool(0).unwrap();
                assert!(
                    current
                        .chart_ids
                        .iter()
                        .all(|&c| pool.is_chart_info_loaded(c))
                );
                assert!(pool.chart_info(30474).is_some());
                assert_eq!(pool.get_default_pools().len(), 1);
                assert_eq!(pool.list_pools().len(), 2);
                assert_eq!(pool.resolve_pools(&[0, 1]).len(), 2);
            }
            start.elapsed()
        })
    };
    let elapsed = reader.join().unwrap();
    assert!(
        elapsed < Duration::from_secs(1),
        "1000 rounds of reads took {elapsed:?}"
    );

    tokio::time::timeout(Duration::from_secs(5), pool.set_favorite_id(1, Some(9)))
        .await
        .expect("pool lock is not held during the fetch")
        .unwrap();
    assert!(!adder.is_finished());
    assert_eq!(pool.find_pool(0).unwrap().chart_ids, [30474, 42058]);

    gate.open();
    assert_eq!(adder.await.unwrap().unwrap(), 1);
    assert_eq!(pool.find_pool(0).unwrap().chart_ids, [30474, 42058, 777]);
    assert!(pool.is_chart_info_loaded(777));
    assert_eq!(pool.find_pool(1).unwrap().favorite_id, Some(9));
}

#[tokio::test]
async fn a_failed_write_changes_nothing() {
    let upstream = FakeUpstream::new();
    upstream.add_chart(chart(777, &["regular"], 0.9, 10, 17.0, None));
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    write(
        dir.path(),
        INDEX_FILE,
        &chart_map_json(&regular_index_charts(3)),
    );
    let files = FailingFiles::new();
    let cfg = config(dir.path()).with_files(files.clone());
    let up: Arc<dyn Upstream> = upstream.clone();
    let pool = ChartPool::load(&cfg, up.clone()).await.unwrap();
    let index = ChartIndex::load(&cfg, up, &pool).await;
    files.fail(POOL_FILE, true);

    let file_before = read(dir.path(), POOL_FILE);
    let list_before = pool.list_pools();
    let check = |what: &str, err: CatalogError, expected: &str| {
        assert_eq!(error_text(&err), expected, "{what}");
        assert_eq!(pool.list_pools(), list_before, "{what}: memory");
        assert_eq!(
            pool.find_pool(0),
            Some(list_before[0].clone()),
            "{what}: snapshot"
        );
        assert_eq!(read(dir.path(), POOL_FILE), file_before, "{what}: file");
        assert_eq!(pool.export_json().unwrap(), file_before, "{what}: export");
    };
    let unchecked = "Storage: Failed to save chart pools";
    let checked = "Storage: injected write failure";
    check(
        "add_charts",
        pool.add_charts(0, &[777, 52087]).await.unwrap_err(),
        checked,
    );
    check(
        "add_pool",
        pool.add_pool(9, &[]).await.unwrap_err(),
        checked,
    );
    check(
        "remove_pool",
        pool.remove_pool(1).await.unwrap_err(),
        unchecked,
    );
    check(
        "remove_chart",
        pool.remove_chart(0, 30474).await.unwrap_err(),
        unchecked,
    );
    check(
        "set_favorite_id",
        pool.set_favorite_id(0, Some(1)).await.unwrap_err(),
        unchecked,
    );
    check(
        "set_default_flag",
        pool.set_default_flag(1, true).await.unwrap_err(),
        unchecked,
    );
    check(
        "update_pool",
        pool.update_pool(0, None, Some(3), None, None, None)
            .await
            .unwrap_err(),
        unchecked,
    );
    check(
        "create_empty_pools",
        pool.create_empty_pools(2, None, None, None)
            .await
            .unwrap_err(),
        unchecked,
    );
    check(
        "generate_pools",
        pool.generate_pools(&index, &ScreeningRule::regular(), 2, None)
            .await
            .unwrap_err(),
        checked,
    );
    let err = pool
        .import_json(
            r#"{"pools":[{"id":5}]}"#,
            zphira_catalog::ImportMode::Replace,
            |_: &std::path::Path| Ok(String::new()),
        )
        .await
        .unwrap_err();
    check("import_json", err, checked);
    let err = pool.remove_pool(1).await.unwrap_err();
    assert!(
        std::error::Error::source(&err).is_some(),
        "the write error is the source"
    );

    assert!(pool.is_chart_info_loaded(777));

    upstream.add_chart(chart(778, &["regular"], 0.9, 10, 17.0, None));
    files.fail(POOL_FILE, false);
    files.fail(CACHE_FILE, true);
    check(
        "add_charts (cache)",
        pool.add_charts(0, &[778]).await.unwrap_err(),
        checked,
    );
    assert!(!pool.is_chart_info_loaded(778));
    assert!(!read(dir.path(), CACHE_FILE).contains("\"778\""));

    files.fail(CACHE_FILE, false);
    assert_eq!(pool.add_charts(0, &[777, 778]).await.unwrap(), 2);
    assert_eq!(pool.export_json().unwrap(), read(dir.path(), POOL_FILE));
}

#[tokio::test]
async fn an_unreadable_chart_cache_is_rebuilt() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(&[], true);
    let damaged = "{\"30474\":{\"id\":30474},\"42";
    write(dir.path(), CACHE_FILE, damaged);
    let upstream = FakeUpstream::new();
    for id in [30474, 42058, 33530, 52087] {
        upstream.add_chart(chart(id, &["regular"], 0.9, 10, 17.0, None));
    }
    let pool = load(dir.path(), upstream.clone()).await;
    let reason =
        serde_json::from_str::<std::collections::BTreeMap<i32, zphira_phira::ChartInfo>>(damaged)
            .unwrap_err()
            .to_string();
    assert_eq!(
        logs.lines(),
        [format!(
            "WARN Chart info cache is unreadable ({reason}), starting empty; the charts of the pools will be fetched again"
        )]
    );
    assert_eq!(
        *upstream.chart_calls.lock().unwrap(),
        [30474, 42058, 33530, 52087]
    );
    assert!(pool.is_chart_info_loaded(52087));
    let rebuilt: std::collections::BTreeMap<i32, zphira_phira::ChartInfo> =
        serde_json::from_str(&read(dir.path(), CACHE_FILE)).unwrap();
    assert_eq!(
        rebuilt.keys().copied().collect::<Vec<_>>(),
        [30474, 33530, 42058, 52087]
    );

    let dir = data_dir(&[], true);
    std::fs::create_dir(dir.path().join(CACHE_FILE)).unwrap();
    let err = ChartPool::load(&config(dir.path()), FakeUpstream::new())
        .await
        .err()
        .unwrap();
    assert!(matches!(err, CatalogError::Storage { .. }), "{err:?}");
}

#[tokio::test]
async fn a_chart_answered_under_another_id_is_refused() {
    let upstream = FakeUpstream::new();
    upstream.add_chart(chart(776, &["regular"], 0.9, 10, 17.0, None));
    upstream.add_chart_as(777, chart(778, &["regular"], 0.9, 10, 17.0, None));
    upstream.add_chart_as(779, zphira_phira::ChartInfo::default());
    let (dir, pool) = two_pools(upstream.clone()).await;
    let file = read(dir.path(), POOL_FILE);

    let err = pool.add_charts(0, &[776, 777]).await.unwrap_err();
    assert_eq!(
        error_text(&err),
        "Upstream: Phira returned chart 778 for request 777"
    );
    let err = pool.add_pool(5, &[779]).await.unwrap_err();
    assert_eq!(
        error_text(&err),
        "Upstream: Phira returned chart 0 for request 779"
    );
    assert_eq!(read(dir.path(), POOL_FILE), file);
    assert!(pool.find_pool(5).is_none());
    for id in [0, 777, 778, 779] {
        assert!(!pool.is_chart_info_loaded(id), "{id}");
    }

    let dir = data_dir(&[], true);
    write(
        dir.path(),
        POOL_FILE,
        "{\"pools\":[{\"id\":1,\"default_flag\":true,\"chart_ids\":[777]}]}",
    );
    let err = ChartPool::load(&config(dir.path()), upstream)
        .await
        .err()
        .unwrap();
    assert_eq!(
        error_text(&err),
        "Upstream: Phira returned chart 778 for request 777"
    );
}

#[tokio::test]
async fn pool_order_cannot_overflow() {
    let (_dir, pool) = two_pools(FakeUpstream::new()).await;
    pool.update_pool(1, None, None, None, Some(i32::MAX), None)
        .await
        .unwrap();
    assert_eq!(
        pool.find_pool(1).unwrap().order,
        zphira_catalog::MAX_POOL_ORDER
    );
    assert_eq!(
        pool.create_empty_pools(2, None, None, None).await.unwrap(),
        [2, 3]
    );
    let orders: Vec<i32> = pool.list_pools().iter().map(|p| p.order).collect();
    assert_eq!(orders, [0, 1_000_000_000, 1_000_000_010, 1_000_000_020]);

    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    write(
        dir.path(),
        POOL_FILE,
        "{\"pools\":[{\"id\":0,\"default_flag\":true,\"chart_ids\":[30474],\"order\":2147483647}]}",
    );
    let pool = load(dir.path(), FakeUpstream::new()).await;
    pool.create_empty_pools(2, None, None, None).await.unwrap();
    let listed: Vec<(i32, i32)> = pool.list_pools().iter().map(|p| (p.id, p.order)).collect();
    assert_eq!(listed, [(0, i32::MAX), (1, i32::MAX), (2, i32::MAX)]);
}

#[tokio::test]
async fn create_sets_the_favorite_in_the_same_write() {
    let upstream = FakeUpstream::new();
    upstream.add_chart(chart(5, &["plain"], 0.9, 10, 3.0, None));
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let files = FailingFiles::new();
    let up: Arc<dyn Upstream> = upstream;
    let pool = ChartPool::load(&config(dir.path()).with_files(files.clone()), up)
        .await
        .unwrap();
    let file = read(dir.path(), POOL_FILE);

    files.fail(POOL_FILE, true);
    let err = pool
        .add_pool_with(
            7,
            &[5],
            Some(PoolCategory::Configured),
            Some(15),
            Some(3),
            Some(42),
        )
        .await
        .unwrap_err();
    assert_eq!(error_text(&err), "Storage: injected write failure");
    assert!(pool.find_pool(7).is_none());
    assert_eq!(read(dir.path(), POOL_FILE), file);

    files.fail(POOL_FILE, false);
    files.writes.lock().unwrap().clear();
    pool.add_pool_with(
        7,
        &[5],
        Some(PoolCategory::Configured),
        Some(15),
        Some(3),
        Some(42),
    )
    .await
    .unwrap();
    let created = pool.find_pool(7).unwrap();
    assert_eq!(
        (
            created.favorite_id,
            created.category,
            created.size_limit,
            created.rounds_per_stay
        ),
        (Some(42), PoolCategory::Configured, Some(15), Some(3))
    );
    let writes = files.writes.lock().unwrap().clone();
    assert_eq!(
        writes.iter().filter(|w| *w == POOL_FILE).count(),
        1,
        "{writes:?}"
    );
    assert!(read(dir.path(), POOL_FILE).contains("\"favorite_id\": 42"));
    pool.add_pool(8, &[]).await.unwrap();
    assert_eq!(pool.find_pool(8).unwrap().favorite_id, None);
    assert_eq!(pool.export_json().unwrap(), read(dir.path(), POOL_FILE));
}
