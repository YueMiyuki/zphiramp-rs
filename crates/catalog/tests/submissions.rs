mod support;

use std::sync::Arc;

use support::*;
use zphira_catalog::files::{CACHE_FILE, POOL_FILE, SUBMISSIONS_FILE};
use zphira_catalog::{
    ApproveOutcome, CatalogConfig, ChartPool, DEFAULT_MAX_PENDING_SUBMISSIONS, SubmissionStatus,
    SubmissionView, Submissions, SubmitResult, Upstream,
};

fn status(s: SubmissionStatus) -> &'static str {
    s.name()
}

fn opt<T: ToString>(v: &Option<T>) -> String {
    v.as_ref().map_or("null".to_owned(), ToString::to_string)
}

fn views(views: &[SubmissionView]) -> String {
    views
        .iter()
        .map(|v| {
            let submitters: Vec<String> = v
                .submitters
                .iter()
                .map(|s| format!("{}={}", s.user_id, opt(&s.name)))
                .collect();
            format!(
                "{}/{}/{}/[{}]/{}/{}",
                v.pool_id,
                v.chart_id,
                status(v.status),
                submitters.join(", "),
                opt(&v.reviewer_id),
                opt(&v.reason)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn results(list: &[SubmitResult]) -> String {
    list.iter()
        .map(|r| format!("{}:{}:{}", r.chart_id, r.accepted, opt(&r.reason)))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn load(dir: &std::path::Path) -> Submissions {
    Submissions::load(&CatalogConfig::new(dir)).await
}

#[tokio::test]
async fn operations_match_java() {
    let dir = tempfile::tempdir().unwrap();
    let s = load(dir.path()).await;
    let java = java_results("submissions_ops");
    let check = |key: &str, value: String| assert_eq!(value, java[key], "{key}");

    check(
        "submit1",
        results(&s.submit(0, &[1, 2, 3, 2], 7, Some("Alice")).await.unwrap()),
    );
    check(
        "submit2",
        results(&s.submit(0, &[2], 8, Some("Bob <&> \"q\"")).await.unwrap()),
    );
    check(
        "submit3",
        results(&s.submit(5, &[2], 8, None).await.unwrap()),
    );
    check(
        "approve",
        s.review(0, 1, true, 99, None).await.unwrap().to_string(),
    );
    check(
        "approveMissing",
        s.review(0, 42, true, 99, None).await.unwrap().to_string(),
    );
    check(
        "submitApproved",
        results(&s.submit(0, &[1], 8, Some("Bob")).await.unwrap()),
    );
    check(
        "reject",
        s.review(0, 3, false, 98, Some("太短"))
            .await
            .unwrap()
            .to_string(),
    );
    check(
        "submitRejected",
        results(&s.submit(0, &[3], 9, Some("Carol")).await.unwrap()),
    );
    check("withdraw", s.withdraw(0, 2, 7).await.unwrap().to_string());
    check(
        "withdrawApproved",
        s.withdraw(0, 1, 7).await.unwrap().to_string(),
    );
    check(
        "withdrawMissing",
        s.withdraw(5, 5, 7).await.unwrap().to_string(),
    );
    check(
        "withdrawNotMine",
        s.withdraw(0, 2, 7).await.unwrap().to_string(),
    );
    check(
        "withdrawLast",
        s.withdraw(5, 2, 8).await.unwrap().to_string(),
    );
    for u in 0..200 {
        s.submit(1, &[9], 1000 + u, Some(&format!("u{u}")))
            .await
            .unwrap();
    }
    check(
        "submitFull",
        results(&s.submit(1, &[9], 5000, Some("late")).await.unwrap()),
    );
    let n13 = |key: &str| {
        java[key].replace(
            "0/3/PENDING/[7=Alice, 9=Carol]/98/太短",
            "0/3/PENDING/[7=Alice, 9=Carol]/null/null",
        )
    };
    assert_eq!(views(&s.list_by_pool(0, None)), n13("byPool0"));
    assert_eq!(
        views(&s.list_by_pool(0, Some(SubmissionStatus::Pending))),
        n13("byPool0Pending")
    );
    assert_ne!(
        n13("byPool0"),
        java["byPool0"],
        "the fixture holds the Java behaviour"
    );
    check("pending", s.list_pending().len().to_string());
    check("bySubmitter8", views(&s.list_by_submitter(8)));
    assert_eq!(s.export_json().unwrap(), read(dir.path(), SUBMISSIONS_FILE));
    let reloaded = load(dir.path()).await;
    assert_eq!(reloaded.list_by_pool(0, None), s.list_by_pool(0, None));
}

#[tokio::test]
async fn a_java_file_loads_and_round_trips() {
    let dir = data_dir(
        &[(SUBMISSIONS_FILE, "submissions_ops/output/submissions.json")],
        false,
    );
    let s = load(dir.path()).await;
    let java = java_results("submissions_ops");
    assert_eq!(views(&s.list_by_pool(0, None)), java["byPool0"]);
    assert_eq!(views(&s.list_by_submitter(8)), java["bySubmitter8"]);
    assert_eq!(s.list_pending().len().to_string(), java["pending"]);
    assert!(same_json(
        &s.export_json().unwrap(),
        &read_fixture("submissions_ops/output/submissions.json")
    ));
}

#[tokio::test]
async fn odd_rows_load_like_java() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[(SUBMISSIONS_FILE, "submissions_odd/input/submissions.json")],
        false,
    );
    let s = load(dir.path()).await;
    let java = java_results("submissions_odd");
    assert_eq!(
        logs.lines(),
        [
            "WARN Submission 1/5 has an unknown status, treating it as PENDING",
            "WARN Submission 1/6 has an unknown status, treating it as PENDING",
        ]
    );
    assert_eq!(
        views(&s.list_by_pool(1, None)),
        java["byPool1"].replace("/null/[", "/PENDING/[")
    );
    assert_eq!(views(&s.list_by_pool(0, None)), java["byPool0"]);
    assert_eq!(
        views(&s.list_pending()),
        format!(
            "{} 1/6/PENDING/[]/null/null 1/5/PENDING/[3=X]/null/null",
            java["pending"]
        )
    );
    assert_eq!(views(&s.list_by_submitter(4)), java["bySubmitter4"]);
    assert_eq!(
        s.withdraw(1, 5, 99).await.unwrap().to_string(),
        java["withdraw"]
    );
    assert_eq!(
        read(dir.path(), SUBMISSIONS_FILE),
        read_fixture("submissions_odd/input/submissions.json")
    );
    assert_eq!(
        s.withdraw(1, 7, 4).await.unwrap().to_string(),
        java["withdrawReal"]
    );
    let java_file = read_fixture("submissions_odd/output/submissions.json");
    let mut expected = java_file.clone();
    for chart in [5, 6] {
        expected = expected.replace(
            &format!("\"chart_id\": {chart},\n      \"submitters\""),
            &format!(
                "\"chart_id\": {chart},\n      \"status\": \"PENDING\",\n      \"submitters\""
            ),
        );
    }
    assert_ne!(expected, java_file);
    assert!(same_json(&read(dir.path(), SUBMISSIONS_FILE), &expected));
}

#[tokio::test]
async fn an_unreadable_store_starts_empty_with_a_warning() {
    let (_guard, logs) = capture_logs();
    let dir = data_dir(
        &[(
            SUBMISSIONS_FILE,
            "submissions_null_row/input/submissions.json",
        )],
        false,
    );
    let s = load(dir.path()).await;
    assert_eq!(
        s.list_pending().len().to_string(),
        java_results("submissions_null_row")["pending"]
    );
    let lines = logs.lines();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("WARN Submission store is unreadable (invalid type: null")
            && lines[0].ends_with("), starting empty"),
        "{lines:?}"
    );

    logs.clear();
    write(
        dir.path(),
        SUBMISSIONS_FILE,
        "{\"submissions\":[{\"pool_id\":1,\"chart_id\":2,\"created_at\":\"soon\"}]}",
    );
    let s = load(dir.path()).await;
    assert!(s.list_by_pool(1, None).is_empty());
    let lines = logs.lines();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("WARN Submission store is unreadable (") && lines[0].contains("soon"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn pending_submissions_are_capped_per_account() {
    let dir = tempfile::tempdir().unwrap();
    let s = load(dir.path()).await;
    let first: Vec<i32> = (1..=30).collect();
    let second: Vec<i32> = (31..=60).collect();
    assert!(
        s.submit(0, &first, 7, Some("A"))
            .await
            .unwrap()
            .iter()
            .all(|r| r.accepted)
    );
    let answer = s.submit(3, &second, 7, Some("A")).await.unwrap();
    assert_eq!(answer.iter().filter(|r| r.accepted).count(), 20);
    assert!(
        answer[20..]
            .iter()
            .all(|r| r.reason.as_deref() == Some("待审投稿已达上限（50），请等待审核后再投"))
    );
    assert_eq!(answer[20].chart_id, 51);
    assert_eq!(s.pending_count(7), DEFAULT_MAX_PENDING_SUBMISSIONS as usize);
    assert!(!s.contains(3, 51), "a refused chart leaves no empty entry");

    let again = s.submit(0, &[1], 7, Some("A")).await.unwrap();
    assert_eq!(again[0].reason.as_deref(), Some("你已经投过这张谱面"));
    assert!(s.submit(3, &[51], 8, Some("B")).await.unwrap()[0].accepted);
    assert!(!s.submit(3, &[51], 7, Some("A")).await.unwrap()[0].accepted);

    assert!(s.review(0, 1, true, 1, None).await.unwrap());
    assert!(s.review(0, 2, false, 1, Some("no")).await.unwrap());
    assert!(s.withdraw(0, 3, 7).await.unwrap());
    let freed = s.submit(3, &[51, 52, 53, 54], 7, Some("A")).await.unwrap();
    let accepted: Vec<i32> = freed
        .iter()
        .filter(|r| r.accepted)
        .map(|r| r.chart_id)
        .collect();
    assert_eq!(accepted, [51, 52, 53]);
    assert_eq!(s.pending_count(7), 50);
}

#[tokio::test]
async fn approval_adds_the_chart_first() {
    let dir = data_dir(
        &[(CACHE_FILE, "pool_defaults/input/chart-info-cache.json")],
        true,
    );
    let upstream = FakeUpstream::new();
    let up: Arc<dyn Upstream> = upstream.clone();
    let cfg = config(dir.path());
    let pool = ChartPool::load(&cfg, up).await.unwrap();
    let s = Submissions::load(&cfg).await;
    s.submit(0, &[777, 52087], 7, Some("A")).await.unwrap();

    match s.approve(&pool, 0, 777, 99).await.unwrap() {
        ApproveOutcome::AddFailed(e) => {
            assert_eq!(e.message(), "HTTP 404: {\"detail\":\"Not found\"}")
        }
        other => panic!("{other:?}"),
    }
    let entry = &s.list_by_pool(0, None)[0];
    assert_eq!(
        (entry.chart_id, entry.status, entry.reviewer_id),
        (777, SubmissionStatus::Pending, None)
    );
    assert_eq!(s.list_pending().len(), 2, "still in the review queue");
    assert!(!pool.find_pool(0).unwrap().chart_ids.contains(&777));

    upstream.add_chart(chart(777, &["regular"], 0.9, 10, 17.0, None));
    assert!(matches!(
        s.approve(&pool, 0, 777, 99).await.unwrap(),
        ApproveOutcome::Approved
    ));
    assert!(pool.find_pool(0).unwrap().chart_ids.contains(&777));
    let entry = &s.list_by_pool(0, Some(SubmissionStatus::Approved))[0];
    assert_eq!(
        (entry.chart_id, entry.reviewer_id, entry.reason.as_deref()),
        (777, Some(99), None)
    );
    assert!(entry.reviewed_at.is_some());
    assert_eq!(
        s.submit(0, &[777], 8, Some("B")).await.unwrap()[0]
            .reason
            .as_deref(),
        Some("已经在该谱池里了")
    );

    assert!(matches!(
        s.approve(&pool, 0, 30474, 99).await.unwrap(),
        ApproveOutcome::NotFound
    ));
    s.submit(42, &[52087], 7, Some("A")).await.unwrap();
    match s.approve(&pool, 42, 52087, 99).await.unwrap() {
        ApproveOutcome::AddFailed(e) => assert_eq!(error_text(&e), "NotFound: Pool not found: 42"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        s.list_by_pool(42, None)[0].status,
        SubmissionStatus::Pending
    );
    assert!(read(dir.path(), POOL_FILE).contains("777"));
}

#[tokio::test]
async fn a_failed_write_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let files = FailingFiles::new();
    let s = Submissions::load(&CatalogConfig::new(dir.path()).with_files(files.clone())).await;
    s.submit(0, &[1, 2], 7, Some("A")).await.unwrap();
    let file = read(dir.path(), SUBMISSIONS_FILE);
    let before = s.list_by_pool(0, None);
    files.fail(SUBMISSIONS_FILE, true);

    let expect = "Storage: Failed to save submissions";
    assert_eq!(
        error_text(&s.submit(0, &[3], 8, Some("B")).await.unwrap_err()),
        expect
    );
    assert_eq!(error_text(&s.withdraw(0, 1, 7).await.unwrap_err()), expect);
    assert_eq!(
        error_text(&s.review(0, 2, true, 1, None).await.unwrap_err()),
        expect
    );
    assert_eq!(s.list_by_pool(0, None), before);
    assert_eq!(read(dir.path(), SUBMISSIONS_FILE), file);
    assert_eq!(s.export_json().unwrap(), file);
    assert!(!s.submit(0, &[1], 7, Some("A")).await.unwrap()[0].accepted);
}

#[tokio::test]
async fn export_without_a_file_is_the_java_empty_document() {
    let dir = tempfile::tempdir().unwrap();
    let s = load(dir.path()).await;
    assert_eq!(
        s.export_json().unwrap(),
        java_results("import_submissions")["exportEmpty"]
    );
    s.submit(0, &[1], 7, None).await.unwrap();
    assert!(s.withdraw(0, 1, 7).await.unwrap());
    assert_eq!(s.export_json().unwrap(), "{\n  \"submissions\": []\n}");
    assert_eq!(
        read(dir.path(), SUBMISSIONS_FILE),
        "{\n  \"submissions\": []\n}"
    );
}

#[tokio::test]
async fn a_reopened_submission_loses_its_old_review() {
    let dir = tempfile::tempdir().unwrap();
    let s = load(dir.path()).await;
    s.submit(0, &[5, 6], 7, Some("A")).await.unwrap();
    assert!(s.review(0, 5, false, 98, Some("太短")).await.unwrap());
    assert!(s.review(0, 6, true, 98, None).await.unwrap());
    let rejected = &s.list_by_pool(0, None)[0];
    assert_eq!(
        (rejected.status, rejected.reviewer_id),
        (SubmissionStatus::Rejected, Some(98))
    );
    assert!(rejected.reviewed_at.is_some());

    assert!(s.submit(0, &[5], 8, Some("B")).await.unwrap()[0].accepted);
    let reopened = &s.list_by_pool(0, None)[0];
    assert_eq!(reopened.status, SubmissionStatus::Pending);
    assert_eq!(
        (
            reopened.reviewer_id,
            reopened.reviewed_at,
            reopened.reason.as_deref()
        ),
        (None, None, None)
    );
    assert_eq!(reopened.submitters.len(), 2);
    assert!(!read(dir.path(), SUBMISSIONS_FILE).contains("太短"));
    assert!(s.submit(0, &[5], 9, Some("C")).await.unwrap()[0].accepted);
    assert_eq!(s.list_by_pool(0, None)[0].created_at, reopened.created_at);
    let approved = &s.list_by_pool(0, None)[1];
    assert_eq!(
        (approved.status, approved.reviewer_id),
        (SubmissionStatus::Approved, Some(98))
    );
}

#[tokio::test]
async fn n19_unknown_statuses_load_as_pending() {
    let (_guard, logs) = capture_logs();
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        SUBMISSIONS_FILE,
        r#"{"submissions":[
            {"pool_id":2,"chart_id":1,"status":"BOGUS","submitters":[{"user_id":7}]},
            {"pool_id":2,"chart_id":2,"status":null},
            {"pool_id":2,"chart_id":3,"status":"pending","created_at":"2025-01-01T00:00Z"},
            {"pool_id":2,"chart_id":4},
            {"pool_id":2,"chart_id":5,"status":"APPROVED"},
            {"pool_id":2,"chart_id":6,"status":7}
        ]}"#,
    );
    let s = load(dir.path()).await;
    assert_eq!(
        logs.lines(),
        [1, 2, 3, 6].map(|c| format!(
            "WARN Submission 2/{c} has an unknown status, treating it as PENDING"
        ))
    );
    let pending: Vec<i32> = s
        .list_by_pool(2, Some(SubmissionStatus::Pending))
        .iter()
        .map(|v| v.chart_id)
        .collect();
    assert_eq!(pending, [1, 2, 3, 4, 6]);
    assert_eq!(s.list_pending().len(), 5);
    assert_eq!(s.pending_count(7), 1);
    assert_eq!(
        s.list_by_pool(2, None)[4].status,
        SubmissionStatus::Approved
    );
    assert!(read(dir.path(), SUBMISSIONS_FILE).contains("BOGUS"));

    assert!(s.review(2, 1, true, 99, None).await.unwrap());
    assert!(s.review(2, 6, false, 99, Some("no")).await.unwrap());
    let file = read(dir.path(), SUBMISSIONS_FILE);
    assert!(!file.contains("BOGUS") && !file.contains("\"pending\""));
    assert_eq!(file.matches("\"status\": \"PENDING\"").count(), 3);
    logs.clear();
    let reloaded = load(dir.path()).await;
    assert!(logs.lines().is_empty(), "{:?}", logs.lines());
    assert_eq!(reloaded.list_by_pool(2, None), s.list_by_pool(2, None));

    let err = s
        .import_json(
            r#"{"submissions":[{"pool_id":2,"chart_id":1,"status":"BOGUS"}]}"#,
            zphira_catalog::ImportMode::Merge,
            |_: &std::path::Path| Ok(String::new()),
        )
        .await
        .unwrap_err();
    assert_eq!(err.message(), "submissions[0] 无效：未知状态 \"BOGUS\"");
}

#[tokio::test]
async fn the_pending_cap_is_configurable() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = CatalogConfig::new(dir.path()).with_max_pending_submissions(Some(3));
    let s = Submissions::load(&cfg).await;
    let answer = s.submit(0, &[1, 2, 3, 4, 5], 7, Some("A")).await.unwrap();
    let accepted: Vec<i32> = answer
        .iter()
        .filter(|r| r.accepted)
        .map(|r| r.chart_id)
        .collect();
    assert_eq!(accepted, [1, 2, 3]);
    assert_eq!(
        answer[3].reason.as_deref(),
        Some("待审投稿已达上限（3），请等待审核后再投")
    );
    assert_eq!(
        zphira_catalog::pending_limit_reason(3),
        "待审投稿已达上限（3），请等待审核后再投"
    );
    assert!(!s.contains(0, 4));
    assert!(s.submit(0, &[4], 8, Some("B")).await.unwrap()[0].accepted);
    assert!(s.review(0, 1, false, 1, None).await.unwrap());
    assert!(s.submit(1, &[9], 7, Some("A")).await.unwrap()[0].accepted);
    assert!(!s.submit(1, &[10], 7, Some("A")).await.unwrap()[0].accepted);

    let dir = tempfile::tempdir().unwrap();
    let zero =
        Submissions::load(&CatalogConfig::new(dir.path()).with_max_pending_submissions(Some(0)))
            .await;
    let answer = zero.submit(0, &[1], 7, None).await.unwrap();
    assert_eq!(
        answer[0].reason.as_deref(),
        Some("待审投稿已达上限（0），请等待审核后再投")
    );

    let dir = tempfile::tempdir().unwrap();
    let open =
        Submissions::load(&CatalogConfig::new(dir.path()).with_max_pending_submissions(None)).await;
    let ids: Vec<i32> = (1..=120).collect();
    assert!(
        open.submit(0, &ids, 7, Some("A"))
            .await
            .unwrap()
            .iter()
            .all(|r| r.accepted)
    );
    assert_eq!(open.pending_count(7), 120);
    assert_eq!(
        CatalogConfig::new(dir.path()).max_pending_submissions,
        Some(50),
        "default"
    );
}
