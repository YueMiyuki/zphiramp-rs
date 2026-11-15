mod support;

use std::net::SocketAddr;
use std::path::Path;

use axum::body::Body;
use axum::extract::ConnectInfo;
use http::Request;
use serde_json::{Value, json};
use support::{Env, error_body};
use zphira_http::HttpConfig;

fn ids(v: &Value) -> Vec<i64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect()
}

fn file(env: &Env, name: &str) -> String {
    std::fs::read_to_string(env.data.join(name)).unwrap_or_default()
}

fn backup_count(env: &Env) -> usize {
    std::fs::read_dir(env.data.join("backups"))
        .map(|d| d.count())
        .unwrap_or(0)
}

#[tokio::test]
async fn a_01_bad_imports_change_nothing() {
    let env = Env::new().await;
    let cases = [
        (
            "admins",
            "[1, {\"id\":2}]",
            "管理员名单第 2 项无效：必须是整数用户 ID",
        ),
        ("records", "[1,2]", "比赛记录第 1 项无效：必须是对象"),
        ("points", "{\"101\":{\"points\":-5}}", ""),
        (
            "pools",
            "{\"pools\":[{\"id\":1},{\"id\":1}]}",
            "pools[1] 无效：谱池 id 1 重复",
        ),
        (
            "submissions",
            "{\"submissions\":[null]}",
            "submissions[0] 无效：不是对象",
        ),
        ("pools", "{\"pools\":[]}", "pools 至少需要一个谱池"),
        ("admins", "[]", "至少保留一名管理员"),
    ];
    let files = [
        ("admins", "admins.json"),
        ("records", "round-records.json"),
        ("points", "player-points.json"),
        ("pools", "chart-pools.json"),
        ("submissions", "submissions.json"),
    ];
    for (kind, document, reason) in cases {
        let file_name = files.iter().find(|(k, _)| *k == kind).unwrap().1;
        let before_file = file(&env, file_name);
        let before_export = env.get(&format!("/api/v1/data/{kind}/export")).await.text();
        for mode in ["replace", "merge"] {
            let r = env
                .call(
                    "POST",
                    &format!("/api/v1/data/{kind}/import?mode={mode}"),
                    Some(&env.admin.clone()),
                    Some(document),
                )
                .await;
            if (document == "[]" || document == "{\"pools\":[]}") && mode == "merge" {
                assert_eq!(r.status, 200, "{}", r.text());
                continue;
            }
            assert_eq!(r.status, 400, "{kind} {mode}: {}", r.text());
            if !reason.is_empty() {
                assert_eq!(r.json(), error_body(reason), "{kind} {mode}");
            }
        }
        assert_eq!(file(&env, file_name), before_file, "{kind}: file changed");
        assert_eq!(
            env.get(&format!("/api/v1/data/{kind}/export")).await.text(),
            before_export,
            "{kind}: memory changed"
        );
    }
    assert_eq!(
        backup_count(&env),
        2,
        "only the two accepted no-op merges left a backup"
    );
    let reloaded = support::load_stores(&env.data, &env.phira.client()).await;
    assert_eq!(reloaded.records.len(), 3);
    assert_eq!(reloaded.admins.list(), vec![1]);

    std::fs::write(
        env.data.join("backups/records-20990101-000000-000.json"),
        "[1,2]",
    )
    .unwrap();
    let r = env
        .call(
            "POST",
            "/api/v1/data/records/restore",
            Some(&env.admin.clone()),
            Some("{\"backup\":\"records-20990101-000000-000.json\"}"),
        )
        .await;
    assert_eq!(r.json(), error_body("比赛记录第 1 项无效：必须是对象"));
    assert_eq!(
        env.get("/api/v1/record/list").await.json()["result"]["total"],
        json!(3)
    );
}

#[tokio::test]
async fn a_03_export_is_memory_and_import_goes_through_the_store() {
    let env = Env::new().await;
    env.stores.points.add_points(777, "Mallory", 42);
    assert!(!file(&env, "player-points.json").contains("777"));
    let export = env.get("/api/v1/data/points/export").await.json();
    assert_eq!(export["777"], json!({"name":"Mallory","points":42}));

    let r = env
        .call(
            "POST",
            "/api/v1/data/pools/import?mode=merge",
            Some(&env.admin.clone()),
            Some("{\"pools\":[{\"id\":30,\"chart_ids\":[1004]}]}"),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(file(&env, "chart-pools.json").contains("\"id\": 30"));
    let r = env
        .call(
            "POST",
            "/api/v1/pool/0/charts",
            Some(&env.admin.clone()),
            Some("{\"chartIds\":[1006]}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true,"added":1}));
    let pools = file(&env, "chart-pools.json");
    assert!(
        pools.contains("\"id\": 30") && pools.contains("1006"),
        "{pools}"
    );
    let list = env.get("/api/v1/pool/list").await.json();
    assert!(
        list["pools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == json!(30))
    );
}

fn login_request(address: SocketAddr, password: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/v1/login")
        .header("Content-Type", "application/json")
        .body(Body::from(format!(
            "{{\"email\":\"user1@x\",\"password\":\"{password}\"}}"
        )))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(address));
    request
}

#[tokio::test]
async fn a_06_login_rate_limit_per_address() {
    let env = Env::new().await;
    let a: SocketAddr = "10.1.1.1:50000".parse().unwrap();
    let a_other_port: SocketAddr = "10.1.1.1:50001".parse().unwrap();
    let b: SocketAddr = "10.2.2.2:50000".parse().unwrap();

    let r = env.send(login_request(a, "wrong")).await;
    assert_eq!(r.json(), error_body("登录失败，请检查邮箱与密码"));
    assert_eq!(
        env.phira
            .login_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    for i in 1..30 {
        let address = if i % 2 == 0 { a } else { a_other_port };
        let r = env.send(login_request(address, "right")).await;
        assert_eq!(r.status, 200, "attempt {}", i + 1);
    }
    let r = env.send(login_request(a, "right")).await;
    assert_eq!(r.status, 429);
    assert_eq!(r.json(), error_body("登录尝试过于频繁，请稍后再试"));
    assert_eq!(r.header("Access-Control-Allow-Origin"), Some("*"));
    assert_eq!(r.header("Content-Type"), Some("application/json"));
    assert_eq!(
        env.phira
            .login_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        30
    );
    assert_eq!(env.send(login_request(b, "right")).await.status, 200);
}

#[tokio::test]
async fn a_06_login_window_rolls() {
    let config = HttpConfig {
        login_rate_limit: Some(2),
        login_window: std::time::Duration::from_millis(300),
        ..HttpConfig::default()
    };
    let env = Env::with_config(config).await;
    let a: SocketAddr = "10.1.1.1:50000".parse().unwrap();
    assert_eq!(env.send(login_request(a, "right")).await.status, 200);
    assert_eq!(env.send(login_request(a, "right")).await.status, 200);
    assert_eq!(env.send(login_request(a, "right")).await.status, 429);
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    assert_eq!(env.send(login_request(a, "right")).await.status, 200);
}

#[tokio::test]
async fn a_06_submission_name_lookup_is_bounded() {
    let config = HttpConfig {
        name_lookup_timeout: std::time::Duration::from_millis(100),
        ..HttpConfig::default()
    };
    let env = Env::with_config(config).await;
    env.phira.behaviour.lock().unwrap().user_delay = Some(std::time::Duration::from_secs(5));
    let started = std::time::Instant::now();
    let r = env
        .call(
            "POST",
            "/api/v1/pool/2/submissions",
            Some(&env.user5.clone()),
            Some("{\"chartIds\":[1002]}"),
        )
        .await;
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(r.json()["accepted"], json!(1));
    let mine = env
        .call(
            "GET",
            "/api/v1/submission/mine",
            Some(&env.user5.clone()),
            None,
        )
        .await
        .json();
    assert_eq!(
        mine["submissions"][0]["submitters"][0]["name"],
        json!("用户 5")
    );
}

fn big_records_document() -> String {
    let mut rounds = Vec::new();
    for i in 0..900 {
        let results: Vec<Value> = (0..6)
            .map(|p| json!({"player_id":100 + p,"player_name":format!("player{p}"),"rank":p + 1,"score":900000,"accuracy":0.98,"std":0.01,"gained_points":3,"total_points":30}))
            .collect();
        rounds.push(json!({"id":format!("r1-{i}-1"),"room_id":"r1","chart_id":1000,"chart_name":"C1000","started_at":1,"finished_at":2 + i,"results":results}));
    }
    serde_json::to_string_pretty(&rounds).unwrap()
}

#[tokio::test]
async fn a_10_import_takes_64_mib_other_routes_1_mb() {
    let env = Env::new().await;
    let document = big_records_document();
    assert!(document.len() > 1_000_000, "{}", document.len());
    let r = env
        .call(
            "POST",
            "/api/v1/data/records/import?mode=replace",
            Some(&env.admin.clone()),
            Some(&document),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json()["imported"], json!(900));

    let mut padded = String::from("{\"category\":\"MANUAL\"}");
    padded.push_str(&" ".repeat(1_000_001 - padded.len()));
    let r = env
        .call(
            "POST",
            "/api/v1/pool/generate",
            Some(&env.admin.clone()),
            Some(&padded),
        )
        .await;
    assert_eq!(r.status, 413);
    assert_eq!(r.text(), "Content Too Large");
    padded.pop();
    let r = env
        .call(
            "POST",
            "/api/v1/pool/generate",
            Some(&env.admin.clone()),
            Some(&padded),
        )
        .await;
    assert_eq!(r.json(), error_body("该类别不支持规则生成"));

    let config = HttpConfig {
        max_import_size: 2_000_000,
        ..HttpConfig::default()
    };
    let env = Env::with_config(config).await;
    let mut huge = document.clone();
    huge.push_str(&" ".repeat(2_000_001 - huge.len()));
    let r = env
        .call(
            "POST",
            "/api/v1/data/records/import",
            Some(&env.admin.clone()),
            Some(&huge),
        )
        .await;
    assert_eq!(r.status, 413);
    assert_eq!(
        zphira_http::HttpConfig::default().max_import_size,
        64 * 1024 * 1024
    );
}

#[tokio::test]
async fn a_11_import_contract_kept() {
    let env = Env::new().await;
    let admin = env.admin.clone();
    let import = |q: &'static str, body: &'static str| {
        let admin = admin.clone();
        let env = &env;
        async move {
            env.call(
                "POST",
                &format!("/api/v1/data/admins/import{q}"),
                Some(&admin),
                Some(body),
            )
            .await
            .json()
        }
    };
    assert_eq!(import("", "[1,5]").await["imported"], json!(2));
    assert_eq!(
        import("", "[1]").await["imported"],
        json!(1),
        "no mode: replace"
    );
    assert_eq!(
        import("?mode=MERGE", "[6]").await["imported"],
        json!(2),
        "mode=merge"
    );
    assert_eq!(
        import("?merge=true", "[7]").await["imported"],
        json!(3),
        "merge=true"
    );
    assert_eq!(
        import("?mode=replace&merge=false", "[1]").await["imported"],
        json!(1)
    );
    let r = env
        .call(
            "POST",
            "/api/v1/data/points/import",
            Some(&admin),
            Some("{\"kind\":\"points\",\"content\":\"{\\\"8\\\":{\\\"points\\\":5}}\"}"),
        )
        .await;
    assert_eq!(r.status, 400, "{}", r.text());
    assert_eq!(
        env.get("/api/v1/point/ranking").await.json()["ranking"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_12_generate_probe_budget_capped_at_500() {
    let env = Env::new().await;
    let (captured, _guard) = support::replay::capture();
    let r = env
        .call(
            "POST",
            "/api/v1/pool/generate",
            Some(&env.admin.clone()),
            Some(
                "{\"category\":\"TB\",\"sizeLimit\":15,\"roundsPerStay\":1,\"probeDuration\":200}",
            ),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true,"poolIds":[3],"leftover":0}));
    let logs = captured.take();
    assert!(
        logs.iter()
            .any(|l| l == "INFO Server: Probed 6 durations for TB (budget 200)"),
        "{logs:?}"
    );
    let r = env
        .call(
            "POST",
            "/api/v1/pool/generate",
            Some(&env.admin.clone()),
            Some("{\"category\":\"TB\",\"probeDuration\":100000}"),
        )
        .await;
    assert_eq!(r.status, 200);
    let logs = captured.take();
    assert!(
        logs.iter()
            .any(|l| l == "INFO Server: Probed 0 durations for TB (budget 500)"),
        "{logs:?}"
    );
}

#[tokio::test]
async fn a_15_unindexed_charts_are_looked_up() {
    let env = Env::new().await;
    env.phira
        .behaviour
        .lock()
        .unwrap()
        .failing_charts
        .push(555_555);
    let before = env.get("/api/v1/chart/search").await.json()["result"]["indexed"].clone();
    let r = env
        .call(
            "POST",
            "/api/v1/pool/2/submissions",
            Some(&env.player.clone()),
            Some("{\"chartIds\":[424242,900005,1002,555555,424242]}"),
        )
        .await;
    assert_eq!(
        r.json(),
        json!({"ok":true,"accepted":2,"results":[
            {"chartId":424242,"accepted":true,"reason":null},
            {"chartId":900005,"accepted":false,"reason":"Phira 上找不到这张谱面"},
            {"chartId":1002,"accepted":true,"reason":null},
            {"chartId":555555,"accepted":false,"reason":"暂时无法从 Phira 获取这张谱面，请稍后再试"},
            {"chartId":424242,"accepted":false,"reason":"你已经投过这张谱面"}]})
    );
    let after = env.get("/api/v1/chart/search").await.json()["result"]["indexed"].clone();
    assert_eq!(after.as_i64().unwrap(), before.as_i64().unwrap() + 1);
    assert!(file(&env, "chart-index.json").contains("\"424242\""));
    let pending = env.get("/api/v1/submission/pending").await.json();
    let entry = pending["submissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["chartId"] == json!(424242))
        .unwrap()
        .clone();
    assert_eq!(entry["chart"]["name"], json!("C424242"));
}

#[tokio::test]
async fn a_19_failed_approval_stays_pending() {
    let env = Env::new().await;
    env.phira
        .behaviour
        .lock()
        .unwrap()
        .failing_charts
        .push(1004);
    let r = env
        .call(
            "POST",
            "/api/v1/pool/2/submissions",
            Some(&env.player.clone()),
            Some("{\"chartIds\":[1004]}"),
        )
        .await;
    assert_eq!(r.json()["accepted"], json!(1));
    let r = env
        .call(
            "POST",
            "/api/v1/submission/2/1004/approve",
            Some(&env.admin.clone()),
            None,
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json(), error_body("加入谱池失败：HTTP 500: boom"));
    let pending = env.get("/api/v1/submission/pending").await.json();
    assert!(
        pending["submissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["chartId"] == json!(1004) && s["status"] == json!("PENDING"))
    );
    let pool = env.get("/api/v1/pool/list").await.json()["pools"][2]["chartIds"].clone();
    assert_eq!(ids(&pool), vec![1003]);

    env.phira.behaviour.lock().unwrap().failing_charts.clear();
    let r = env
        .call(
            "POST",
            "/api/v1/submission/2/1004/approve",
            Some(&env.admin.clone()),
            None,
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    let pool = env.get("/api/v1/pool/list").await.json()["pools"][2]["chartIds"].clone();
    assert_eq!(ids(&pool), vec![1003, 1004]);
    let list = env
        .get("/api/v1/pool/2/submissions?status=APPROVED")
        .await
        .json();
    assert!(
        list["submissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["chartId"] == json!(1004))
    );
}

fn many_regular_charts(dir: &Path) {
    let path = dir.join("data/chart-index.json");
    let mut index: serde_json::Map<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for id in 5000..5205 {
        index.insert(id.to_string(), json!({"id":id,"name":format!("C{id}"),"level":"AT Lv.17","difficulty":17.5,"tags":["regular"],"rating":0.9,"ratingCount":10}));
    }
    std::fs::write(&path, serde_json::to_string_pretty(&index).unwrap()).unwrap();
}

#[tokio::test]
async fn a_24_generate_is_additive_and_reports_leftover() {
    let env = Env::build(HttpConfig::default(), many_regular_charts).await;
    let body = "{\"category\":\"REGULAR\",\"sizeLimit\":1}";
    let r = env
        .call(
            "POST",
            "/api/v1/pool/generate",
            Some(&env.admin.clone()),
            Some(body),
        )
        .await
        .json();
    assert_eq!(r["poolIds"].as_array().unwrap().len(), 200);
    assert_eq!(r["leftover"], json!(8));
    let r = env
        .call(
            "POST",
            "/api/v1/pool/generate",
            Some(&env.admin.clone()),
            Some(body),
        )
        .await
        .json();
    assert_eq!(r["poolIds"].as_array().unwrap().len(), 8);
    assert_eq!(r["leftover"], json!(0));
    let r = env
        .call(
            "POST",
            "/api/v1/pool/generate",
            Some(&env.admin.clone()),
            Some(body),
        )
        .await
        .json();
    assert_eq!(r, json!({"ok":true,"poolIds":[],"leftover":0}));
}

#[tokio::test]
async fn a_25_create_keeps_favorite_id() {
    let env = Env::new().await;
    let r = env
        .call("POST", "/api/v1/pool", Some(&env.admin.clone()),
            Some("{\"id\":40,\"chartIds\":[1004],\"favoriteId\":123,\"category\":\"MANUAL\",\"sizeLimit\":null,\"roundsPerStay\":null}"))
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    let pools = env.get("/api/v1/pool/list").await.json();
    let pool = pools["pools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == json!(40))
        .unwrap()
        .clone();
    assert_eq!(pool["favoriteId"], json!(123));
    assert!(file(&env, "chart-pools.json").contains("\"favorite_id\": 123"));
    let r = env
        .call(
            "POST",
            "/api/v1/pool",
            Some(&env.admin.clone()),
            Some("{\"id\":41,\"favoriteId\":null}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
}

#[tokio::test(flavor = "current_thread")]
async fn a_26_client_mistakes_are_400() {
    let env = Env::new().await;
    let admin = env.admin.clone();
    let (captured, _guard) = support::replay::capture();
    let cases: Vec<(&str, &str, Option<&str>, &str)> = vec![
        (
            "GET",
            "/api/v1/chart/search?tag=plain,plain",
            None,
            "参数格式错误：tag",
        ),
        (
            "PUT",
            "/api/v1/pool/999/favorite",
            Some("{\"favoriteId\":1}"),
            "Pool not found: 999",
        ),
        (
            "PUT",
            "/api/v1/pool/999/default",
            Some("{\"enabled\":true}"),
            "Pool not found: 999",
        ),
        (
            "POST",
            "/api/v1/pool/generate",
            Some("{\"category\":\"CONFIGURED\",\"sizeLimit\":0}"),
            "Pool size limit must be positive",
        ),
        (
            "POST",
            "/api/v1/pool/generate",
            Some("{\"category\":\"nope\"}"),
            "未知类别：nope",
        ),
        (
            "POST",
            "/api/v1/pool",
            Some("{\"id\":50,\"category\":\"Weird\"}"),
            "未知类别：Weird",
        ),
        (
            "PUT",
            "/api/v1/pool/0",
            Some("{\"category\":\"bogus\"}"),
            "未知类别：bogus",
        ),
        (
            "POST",
            "/api/v1/pool/batch",
            Some("{\"count\":1,\"category\":\"x\"}"),
            "未知类别：x",
        ),
        (
            "POST",
            "/api/v1/pool/0/charts",
            Some("{\"chartIds\":[1004,null]}"),
            "请检查是否传入了错误的格式",
        ),
        (
            "POST",
            "/api/v1/chart/duration",
            Some("{\"chartIds\":[null]}"),
            "请检查是否传入了错误的格式",
        ),
        (
            "POST",
            "/api/v1/pool/2/submissions",
            Some("{\"chartIds\":[null]}"),
            "请检查是否传入了错误的格式",
        ),
        (
            "POST",
            "/api/v1/data/admins/restore",
            Some("{\"backup\":\"/\"}"),
            "备份不存在",
        ),
        (
            "POST",
            "/api/v1/data/admins/restore",
            Some("{\"backup\":\"admins-/..\"}"),
            "备份不存在",
        ),
    ];
    for (method, target, body, reason) in cases {
        let r = env.call(method, target, Some(&admin), body).await;
        assert_eq!(r.status, 400, "{method} {target}: {}", r.text());
        assert_eq!(r.json(), error_body(reason), "{method} {target}");
    }
    assert!(
        !captured.take().iter().any(|l| l.starts_with("ERROR")),
        "no 500 was logged"
    );

    let r = env
        .call(
            "PUT",
            "/api/v1/pool/0",
            Some(&admin),
            Some("{\"category\":\" tb \"}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    let pools = env.get("/api/v1/pool/list").await.json();
    assert_eq!(pools["pools"][0]["category"], json!("TB"));
    let r = env
        .call(
            "POST",
            "/api/v1/pool/batch",
            Some(&admin),
            Some("{\"count\":1,\"category\":\"configured\"}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true,"poolIds":[3]}));
}

#[tokio::test]
async fn a_34_login_returns_phira_token() {
    let env = Env::new().await;
    let r = env
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some("{\"email\":\"user1@x\",\"password\":\"right\"}"),
        )
        .await
        .json();
    assert_eq!(r["phira_token"], json!("phira-token-1"));
    assert_eq!(r["isAdmin"], json!(true));
    assert_eq!(r["userId"], json!(1));
    let token = r["token"].as_str().unwrap();
    assert_eq!(env.state.jwt().parse_user_id(token), Some(1));
}

#[tokio::test]
async fn a_35_jwt_secret_permissions() {
    let dir = tempfile::tempdir().unwrap();
    zphira_http::JwtService::load(dir.path()).unwrap();
    let path = dir.path().join("jwt-secret");
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["jwt-secret".to_owned()]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = || std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(), 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        zphira_http::JwtService::load(dir.path()).unwrap();
        assert_eq!(mode(), 0o600);
    }
}

#[tokio::test]
async fn n_03_remove_non_admin_with_one_admin_left() {
    let env = Env::new().await;
    let r = env
        .call("DELETE", "/api/v1/admin/9", Some(&env.admin.clone()), None)
        .await;
    assert_eq!(r.json(), json!({"ok":true,"removed":false,"admins":[1]}));
    let r = env
        .call("DELETE", "/api/v1/admin/1", Some(&env.admin.clone()), None)
        .await;
    assert_eq!(r.json(), error_body("至少保留一名管理员"));
    assert_eq!(file(&env, "admins.json").trim(), "[\n  1\n]");
}

#[tokio::test]
async fn n_18_login_with_trailing_slash() {
    let env = Env::new().await;
    let body = Some("{\"email\":\"user2@x\",\"password\":\"right\"}");
    for path in ["/api/v1/login", "/api/v1/login/"] {
        let r = env.call("POST", path, None, body).await;
        assert_eq!(r.status, 200, "{path}: {}", r.text());
        assert_eq!(r.json()["userId"], json!(2));
        assert_eq!(r.header("Access-Control-Allow-Origin"), Some("*"));
    }
    for path in ["/api/v1/login//", "/api/v1/loginx", "/api/v1/login/x"] {
        let r = env.call("POST", path, None, body).await;
        assert_eq!(r.json(), error_body("需要登录"), "{path}");
    }
}

#[tokio::test]
async fn n_20_refresh_only_for_a_valid_search() {
    let env = Env::new().await;
    let sweeps = |env: &Env| {
        env.phira
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with("GET /chart") && !r.starts_with("GET /chart/"))
            .count()
    };
    for query in [
        "refresh&minRating=x",
        "refresh&category=nope",
        "refresh&limit=a",
        "refresh&probeDuration=a",
        "refresh&tag=a,a",
    ] {
        let r = env.get(&format!("/api/v1/chart/search?{query}")).await;
        assert_eq!(r.status, 400, "{query}: {}", r.text());
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(sweeps(&env), 0, "no sweep was started");
    let r = env.get("/api/v1/chart/search?limit=1").await;
    assert_eq!(r.json()["result"]["refreshing"], json!(false));

    let r = env
        .get("/api/v1/chart/search?refresh&division=plain&limit=1")
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["result"]["refreshing"], json!(true));
    for _ in 0..50 {
        if sweeps(&env) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(sweeps(&env) > 0, "the valid request started the sweep");
}

fn static_get(target: &str) -> Request<Body> {
    Request::builder().uri(target).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn n_21_encoded_static_paths_and_html_caching() {
    let env = Env::build(HttpConfig::default(), |dir| {
        let dist = dir.join("frontend/dist");
        let chunk = dist.join("_next/static/chunks/app/room/[id]");
        std::fs::create_dir_all(&chunk).unwrap();
        std::fs::write(chunk.join("page-abc.js"), "chunk").unwrap();
        std::fs::write(dist.join("a%20b.txt"), "raw name").unwrap();
    })
    .await;
    let immutable = Some("public, max-age=31536000, immutable");

    let r = env
        .send(static_get(
            "/_next/static/chunks/app/room/%5Bid%5D/page-abc.js",
        ))
        .await;
    assert_eq!((r.status.as_u16(), r.text().as_str()), (200, "chunk"));
    assert_eq!(r.header("Content-Type"), Some("application/javascript"));
    assert_eq!(r.header("Cache-Control"), immutable);
    let r = env
        .send(static_get("/_next/static/chunks/app/room/[id]/page-abc.js"))
        .await;
    assert_eq!(r.text(), "chunk", "the raw spelling works as before");
    assert_eq!(env.send(static_get("/a%20b.txt")).await.text(), "raw name");
    assert_eq!(
        env.send(static_get("/%E4%B8%AD%E6%96%87.txt")).await.text(),
        "chinese\n"
    );

    let r = env.send(static_get("/missing%C3%28.js")).await;
    assert_eq!((r.status.as_u16(), r.text().as_str()), (404, "Not Found"));
    let r = env.send(static_get("/page%C3%28")).await;
    assert_eq!(r.text(), "<!doctype html><title>root</title>\n");
    let r = env.send(static_get("/x%2Ey")).await;
    assert_eq!((r.status.as_u16(), r.text().as_str()), (404, "Not Found"));
    let r = env.send(static_get("/room%2F9")).await;
    assert_eq!(r.text(), "<!doctype html><title>room</title>\n");
    for target in [
        "/..%2fsecret.txt",
        "/docs%2f..%2f..%2fsecret.txt",
        "/..%5csecret.txt",
        "/a%00",
    ] {
        let r = env.send(static_get(target)).await;
        assert_eq!(
            (r.status.as_u16(), r.text().as_str()),
            (400, "Bad Request"),
            "{target}"
        );
    }

    for target in [
        "/index.html",
        "/room/__fallback__/index.html",
        "/pool/__fallback__/index.html",
        "/",
        "/room/7",
    ] {
        let r = env.send(static_get(target)).await;
        assert_eq!(r.status, 200, "{target}");
        assert_eq!(
            r.header("Content-Type"),
            Some("text/html;charset=utf-8"),
            "{target}"
        );
        assert_eq!(r.header("Cache-Control"), None, "{target}");
    }
    for target in ["/style.css", "/logo.jpg", "/index.txt"] {
        assert_eq!(
            env.send(static_get(target)).await.header("Cache-Control"),
            Some("public, max-age=300"),
            "{target}"
        );
    }
}

#[tokio::test]
async fn n_19_bad_status_row_is_listed_and_reviewable() {
    let env = Env::build(HttpConfig::default(), |dir| {
        std::fs::write(
            dir.join("data/submissions.json"),
            "{\"submissions\":[{\"pool_id\":2,\"chart_id\":1004,\"status\":\"BOGUS\",\"submitters\":[{\"user_id\":2,\"name\":\"player2\"}]},{\"pool_id\":2,\"chart_id\":1006,\"status\":null,\"submitters\":[]}]}",
        )
        .unwrap();
    })
    .await;
    for target in [
        "/api/v1/submission/pending",
        "/api/v1/pool/2/submissions",
        "/api/v1/pool/2/submissions?status=PENDING",
    ] {
        let r = env.get(target).await;
        assert_eq!(r.status, 200, "{target}: {}", r.text());
        let rows: Vec<(Value, Value)> = r.json()["submissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["chartId"].clone(), s["status"].clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                (json!(1004), json!("PENDING")),
                (json!(1006), json!("PENDING"))
            ],
            "{target}"
        );
    }
    let mine = env
        .call(
            "GET",
            "/api/v1/submission/mine",
            Some(&env.player.clone()),
            None,
        )
        .await;
    assert_eq!(mine.json()["submissions"][0]["status"], json!("PENDING"));
    let r = env
        .call(
            "POST",
            "/api/v1/submission/2/1004/reject",
            Some(&env.admin.clone()),
            Some("{\"reason\":\"no\"}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    let r = env
        .call(
            "POST",
            "/api/v1/submission/2/1006/approve",
            Some(&env.admin.clone()),
            None,
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    let list = env.get("/api/v1/pool/2/submissions").await.json();
    let statuses: Vec<&str> = list["submissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, vec!["REJECTED", "APPROVED"]);
    assert_eq!(
        env.get("/api/v1/submission/pending").await.json()["submissions"],
        json!([])
    );
}
