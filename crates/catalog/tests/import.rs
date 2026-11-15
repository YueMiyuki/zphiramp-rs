mod support;

use std::path::Path;
use std::sync::{Arc, Mutex};

use support::*;
use zphira_catalog::files::{CACHE_FILE, POOL_FILE, SUBMISSIONS_FILE};
use zphira_catalog::{CatalogConfig, ChartPool, ImportMode, ImportResult, Submissions, Upstream};

fn backup_into(
    dir: &Path,
    calls: Arc<Mutex<Vec<String>>>,
) -> impl FnOnce(&Path) -> std::io::Result<String> + Send + 'static {
    let target = dir.join("backups").join("copy.json");
    move |live: &Path| {
        calls
            .lock()
            .unwrap()
            .push(live.file_name().unwrap().to_string_lossy().into_owned());
        if !live.exists() {
            return Ok(String::new());
        }
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::copy(live, &target)?;
        Ok(target.to_string_lossy().into_owned())
    }
}

fn no_backup(_: &Path) -> std::io::Result<String> {
    panic!("a rejected document must not reach the backup")
}

fn java_import(value: &str) -> (i32, i32) {
    let imported = value
        .split("imported=")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap();
    let total = value
        .split("total=")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap();
    (imported.parse().unwrap(), total.parse().unwrap())
}

fn pools_line(pool: &ChartPool) -> String {
    pool.list_pools()
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

async fn pool_with_defaults(upstream: Arc<FakeUpstream>) -> (tempfile::TempDir, ChartPool) {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let up: Arc<dyn Upstream> = upstream;
    let pool = ChartPool::load(&config(dir.path()), up).await.unwrap();
    (dir, pool)
}

#[tokio::test]
async fn pools_merge_and_replace_match_java() {
    let (_guard, logs) = capture_logs();
    let (dir, pool) = pool_with_defaults(FakeUpstream::new()).await;
    let java = java_results("import_pools");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let before = read(dir.path(), POOL_FILE);

    let merged = pool
        .import_json(
            r#"{"pools":[{"id":1,"chart_ids":[30474],"category":"TB","order":4},{"id":77,"chart_ids":[42058,42058]}]}"#,
            ImportMode::Merge,
            backup_into(dir.path(), calls.clone()),
        )
        .await
        .unwrap();
    assert_eq!((merged.imported, merged.total), java_import(&java["merge"]));
    assert_eq!(
        merged.backup_file,
        dir.path().join("backups/copy.json").to_string_lossy()
    );
    assert_eq!(
        read(dir.path(), "backups/copy.json"),
        before,
        "the backup is the previous file"
    );
    assert_eq!(
        read(dir.path(), POOL_FILE),
        read_fixture("import_pools/after_merge/chart-pools.json")
    );
    assert_eq!(pools_line(&pool), java["afterMerge"]);

    logs.clear();
    let replaced = pool
        .import_json(
            r#"{"pools":[{"id":3,"chart_ids":[33530],"default_flag":true},{"id":4}]}"#,
            ImportMode::Replace,
            backup_into(dir.path(), calls.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        (replaced.imported, replaced.total),
        java_import(&java["replace"])
    );
    assert_eq!(
        read(dir.path(), POOL_FILE),
        read_fixture("import_pools/after_replace/chart-pools.json")
    );
    assert_eq!(pools_line(&pool), java["afterReplace"]);
    assert_eq!(pool.export_json().unwrap(), java["export"]);
    assert_eq!(logs.lines(), ["WARN Pool 4 is empty, rooms will skip it"]);
    assert_eq!(
        *calls.lock().unwrap(),
        ["chart-pools.json", "chart-pools.json"]
    );
    assert!(pool.find_pool(0).is_none());
    assert_eq!(pool.get_default_pools().len(), 1);
}

#[tokio::test]
async fn pools_import_fetches_unknown_charts_before_the_lock() {
    let upstream = FakeUpstream::new();
    upstream.add_chart(chart(9001, &["regular"], 0.9, 10, 17.0, None));
    let (dir, pool) = pool_with_defaults(upstream.clone()).await;
    let result = pool
        .import_json(
            r#"{"pools":[{"id":5,"chart_ids":[9001,30474,9001]}]}"#,
            ImportMode::Merge,
            |_: &Path| Ok(String::new()),
        )
        .await
        .unwrap();
    assert_eq!(
        result,
        ImportResult {
            imported: 3,
            total: 3,
            backup_file: String::new()
        }
    );
    assert_eq!(*upstream.chart_calls.lock().unwrap(), [9001]);
    assert!(pool.is_chart_info_loaded(9001));
    assert!(read(dir.path(), CACHE_FILE).contains("\"9001\": {"));
    assert_eq!(pool.find_pool(5).unwrap().chart_ids, [9001, 30474]);
}

#[tokio::test]
async fn bad_pool_documents_change_nothing() {
    let upstream = FakeUpstream::new();
    upstream.add_chart_as(556, chart(557, &["plain"], 0.9, 10, 3.0, None));
    let (dir, pool) = pool_with_defaults(upstream.clone()).await;
    let file = read(dir.path(), POOL_FILE);
    let list = pool.list_pools();
    let cases: &[(&str, ImportMode, &str)] = &[
        ("", ImportMode::Replace, "内容为空"),
        ("{\"pools\":", ImportMode::Replace, "不是有效的 JSON"),
        ("[1,2]", ImportMode::Merge, "缺少数组字段 pools"),
        (
            "{\"kind\":\"pools\",\"content\":\"...\"}",
            ImportMode::Replace,
            "缺少数组字段 pools",
        ),
        (
            "{\"pools\":[]}",
            ImportMode::Replace,
            "pools 至少需要一个谱池",
        ),
        (
            "{\"pools\":[null]}",
            ImportMode::Replace,
            "pools[0] 无效：不是对象",
        ),
        (
            "{\"pools\":[{\"id\":1},{\"chart_ids\":[1]}]}",
            ImportMode::Merge,
            "pools[1] 无效：缺少 id",
        ),
        (
            "{\"pools\":[{\"id\":1,\"category\":\"tb\"}]}",
            ImportMode::Merge,
            "pools[0] 无效：未知类别 \"tb\"",
        ),
        (
            "{\"pools\":[{\"id\":1},{\"id\":1}]}",
            ImportMode::Replace,
            "pools[1] 无效：谱池 id 1 重复",
        ),
        (
            "{\"pools\":[{\"id\":1,\"chart_ids\":[30474]},{\"id\":2,\"chart_ids\":[42058,555]}]}",
            ImportMode::Replace,
            "pools[1] 无效：谱面 555 无法获取：HTTP 404: {\"detail\":\"Not found\"}",
        ),
        (
            "{\"pools\":[{\"id\":1,\"chart_ids\":[556]}]}",
            ImportMode::Merge,
            "pools[0] 无效：谱面 556 无法获取：Phira returned chart 557 for request 556",
        ),
    ];
    for (document, mode, message) in cases {
        let err = pool
            .import_json(document, *mode, no_backup)
            .await
            .unwrap_err();
        assert_eq!(
            error_text(&err),
            format!("Invalid: {message}"),
            "{document}"
        );
        assert_eq!(read(dir.path(), POOL_FILE), file);
        assert_eq!(pool.list_pools(), list);
    }
    let err = pool
        .import_json(
            "{\"pools\":[{\"id\":1,\"chart_ids\":[\"x\"]}]}",
            ImportMode::Replace,
            no_backup,
        )
        .await
        .unwrap_err();
    assert!(err.message().starts_with("pools[0] 无效："), "{err}");
    let kept = pool
        .import_json("{\"pools\":[]}", ImportMode::Merge, |_: &Path| {
            Ok(String::new())
        })
        .await
        .unwrap();
    assert_eq!((kept.imported, kept.total), (2, 2));
    assert_eq!(pool.list_pools(), list);
}

#[tokio::test]
async fn submissions_replace_matches_java_and_merge_replaces_rows() {
    let java = java_results("import_submissions");
    let doc = r#"{"submissions":[{"pool_id":0,"chart_id":1008,"status":"REJECTED","submitters":[{"user_id":8,"name":"Bob"}],"created_at":"2025-01-01T00:00Z"},{"pool_id":2,"chart_id":1,"submitters":[]}]}"#;
    let views = |s: &Submissions| {
        let line = |pool: i32| {
            s.list_by_pool(pool, None)
                .iter()
                .map(|v| {
                    let who: Vec<String> = v
                        .submitters
                        .iter()
                        .map(|x| {
                            format!("{}={}", x.user_id, x.name.clone().unwrap_or("null".into()))
                        })
                        .collect();
                    format!(
                        "{}/{}/{}/[{}]/{}/{}",
                        v.pool_id,
                        v.chart_id,
                        v.status.name(),
                        who.join(", "),
                        v.reviewer_id.map_or("null".to_owned(), |r| r.to_string()),
                        v.reason.clone().unwrap_or("null".into())
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        format!("{} | {}", line(0), line(2))
    };

    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let s = Submissions::load(&CatalogConfig::new(dir.path())).await;
    s.submit(0, &[1008, 1009], 7, Some("Alice")).await.unwrap();

    let merged = s
        .import_json(
            doc,
            ImportMode::Merge,
            backup_into(dir.path(), calls.clone()),
        )
        .await
        .unwrap();
    assert_eq!(java_import(&java["merge"]), (4, 4));
    assert_eq!((merged.imported, merged.total), (3, 3));
    assert_eq!(
        views(&s),
        "0/1008/REJECTED/[8=Bob]/null/null 0/1009/PENDING/[7=Alice]/null/null | 2/1/PENDING/[]/null/null"
    );
    assert_eq!(s.export_json().unwrap(), read(dir.path(), SUBMISSIONS_FILE));

    let replaced = s
        .import_json(
            doc,
            ImportMode::Replace,
            backup_into(dir.path(), calls.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        (replaced.imported, replaced.total),
        java_import(&java["replace"])
    );
    assert_eq!(views(&s), java["afterReplace"]);
    let reloaded = Submissions::load(&CatalogConfig::new(dir.path())).await;
    assert_eq!(views(&reloaded), java["afterReplace"]);
    let java_file = read_fixture("import_submissions/after_replace/submissions.json");
    write(dir.path(), "java.json", &java_file);
    std::fs::rename(
        dir.path().join("java.json"),
        dir.path().join(SUBMISSIONS_FILE),
    )
    .unwrap();
    let from_java = Submissions::load(&CatalogConfig::new(dir.path())).await;
    assert_eq!(views(&from_java), java["afterReplace"]);
    assert_eq!(
        *calls.lock().unwrap(),
        ["submissions.json", "submissions.json"]
    );
}

#[tokio::test]
async fn bad_submission_documents_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = Submissions::load(&CatalogConfig::new(dir.path())).await;
    s.submit(0, &[1], 7, Some("A")).await.unwrap();
    let file = read(dir.path(), SUBMISSIONS_FILE);
    for (document, message) in [
        ("   ", "内容为空"),
        ("[]", "缺少数组字段 submissions"),
        ("\"text\"", "JSON 顶层必须是对象或数组"),
        ("{\"submissions\":[null]}", "submissions[0] 无效：不是对象"),
        (
            "{\"submissions\":[{\"pool_id\":0}]}",
            "submissions[0] 无效：缺少 chart_id",
        ),
        (
            "{\"submissions\":[{\"pool_id\":0,\"chart_id\":1,\"status\":\"DONE\"}]}",
            "submissions[0] 无效：未知状态 \"DONE\"",
        ),
        (
            "{\"submissions\":[{\"pool_id\":0,\"chart_id\":1},{\"pool_id\":0,\"chart_id\":1}]}",
            "submissions[1] 无效：池 0 谱面 1 重复",
        ),
    ] {
        for mode in [ImportMode::Replace, ImportMode::Merge] {
            let err = s.import_json(document, mode, no_backup).await.unwrap_err();
            assert_eq!(
                error_text(&err),
                format!("Invalid: {message}"),
                "{document}"
            );
            assert_eq!(read(dir.path(), SUBMISSIONS_FILE), file);
            assert_eq!(s.list_by_pool(0, None).len(), 1);
        }
    }
    assert!(
        s.import_json("[1,2]", ImportMode::Replace, no_backup)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_failed_import_write_changes_nothing() {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let files = FailingFiles::new();
    let cfg = config(dir.path()).with_files(files.clone());
    let up: Arc<dyn Upstream> = FakeUpstream::new();
    let pool = ChartPool::load(&cfg, up).await.unwrap();
    let subs = Submissions::load(&cfg).await;
    subs.submit(0, &[1], 7, None).await.unwrap();
    let pools_before = (read(dir.path(), POOL_FILE), pool.list_pools());
    let subs_before = (
        read(dir.path(), SUBMISSIONS_FILE),
        subs.list_by_pool(0, None),
    );
    files.fail(POOL_FILE, true);
    files.fail(SUBMISSIONS_FILE, true);

    let err = pool
        .import_json(
            "{\"pools\":[{\"id\":9}]}",
            ImportMode::Replace,
            |_: &Path| Ok(String::new()),
        )
        .await
        .unwrap_err();
    assert_eq!(error_text(&err), "Storage: injected write failure");
    assert_eq!(
        (read(dir.path(), POOL_FILE), pool.list_pools()),
        pools_before
    );
    let err = subs
        .import_json("{\"submissions\":[]}", ImportMode::Replace, |_: &Path| {
            Ok(String::new())
        })
        .await
        .unwrap_err();
    assert!(matches!(err, zphira_catalog::CatalogError::Storage { .. }));
    assert_eq!(
        (
            read(dir.path(), SUBMISSIONS_FILE),
            subs.list_by_pool(0, None)
        ),
        subs_before
    );

    files.fail(POOL_FILE, false);
    let err = pool
        .import_json(
            "{\"pools\":[{\"id\":9}]}",
            ImportMode::Replace,
            |_: &Path| Err(std::io::Error::other("disk full")),
        )
        .await
        .unwrap_err();
    assert_eq!(error_text(&err), "Storage: disk full");
    assert_eq!(
        (read(dir.path(), POOL_FILE), pool.list_pools()),
        pools_before
    );
}
