mod support;

use std::time::Duration;

use axum::body::Body;
use http::Request;
use serde_json::{Value, json};
use support::replay::{Captured, capture, capture_everywhere};
use support::{Env, Reply, error_body};
use zphira_http::HttpConfig;

const LOGIN_FAILED: &str = "登录失败，请检查邮箱与密码";
const NO_TOKEN_WARN: &str = "WARN Server: HTTP login: upstream answer had no usable token";

fn login(email: &str) -> String {
    json!({"email": email, "password": "right"}).to_string()
}

async fn wait_for(captured: &Captured, seen: &mut Vec<String>, wanted: &str) -> bool {
    for _ in 0..200 {
        seen.extend(captured.take());
        if seen.iter().any(|l| l.contains(wanted)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[tokio::test]
async fn login_rate_limit_defaults_to_30_and_none_disables_it() {
    assert_eq!(HttpConfig::default().login_rate_limit, Some(30));
    let env = Env::with_config(HttpConfig {
        login_rate_limit: None,
        ..HttpConfig::default()
    })
    .await;
    for attempt in 0..45 {
        let r = env
            .call("POST", "/api/v1/login", None, Some(&login("user1@x")))
            .await;
        assert_eq!(r.status, 200, "attempt {attempt}: {}", r.text());
    }
}

async fn config(env: &Env) -> (Value, Value, Value) {
    let v = env.get("/api/v1/room/r1").await.json()["info"]["config"].clone();
    (
        v["cancelReadyInVote"].clone(),
        v["startWhenAllReady"].clone(),
        v["allReadyDelay"].clone(),
    )
}

#[tokio::test]
async fn room_update_sets_the_ready_settings_and_bounds_the_delay() {
    let env = Env::new().await;
    let admin = env.admin.clone();
    let r = env
        .call("POST", "/api/v1/room/r1/create", Some(&admin), Some("{}"))
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    assert_eq!(config(&env).await, (json!(true), json!(true), json!(5)));

    let update = |body: &'static str| {
        env.call(
            "PUT",
            "/api/v1/room/r1/update",
            Some(admin.as_str()),
            Some(body),
        )
    };
    let r =
        update(r#"{"cancelReadyInVote":false,"startWhenAllReady":false,"allReadyDelay":0}"#).await;
    assert_eq!(r.json(), json!({"ok":true}));
    assert_eq!(config(&env).await, (json!(false), json!(false), json!(0)));

    for body in [
        r#"{"allReadyDelay":301,"cancelReadyInVote":true}"#,
        r#"{"allReadyDelay":-1,"startWhenAllReady":true}"#,
    ] {
        let r = update(body).await;
        assert_eq!(r.status, 400, "{body}");
        assert_eq!(r.json(), error_body("allReadyDelay 必须在 0 到 300 之间"));
    }
    assert_eq!(config(&env).await, (json!(false), json!(false), json!(0)));

    let r = update(r#"{"allReadyDelay":300,"cancelReadyInVote":true}"#).await;
    assert_eq!(r.json(), json!({"ok":true}));
    assert_eq!(config(&env).await, (json!(true), json!(false), json!(300)));
}

fn request(method: &str, target: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(target);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

fn answer(r: &Reply) -> (u16, Option<&str>, Option<&str>, Option<&str>) {
    (
        r.status.as_u16(),
        r.header("Content-Type"),
        r.header("Cache-Control"),
        r.header("Access-Control-Allow-Origin"),
    )
}

#[tokio::test]
async fn head_answers_what_get_answers_without_a_body() {
    let env = Env::new().await;
    let admin = env.admin.clone();
    let player = env.player.clone();
    let cases = [
        ("/api/v1/pool/list", None),
        ("/api/v1/pool/list", Some(player.as_str())),
        ("/api/v1/pool/list", Some(admin.as_str())),
        ("/api/v1/room/list/", Some(admin.as_str())),
        ("/api/v1/admin/5", Some(admin.as_str())),
        ("/api/v1/login", None),
        ("/api/v1/nope", Some(admin.as_str())),
        ("/", None),
        ("/room/7", None),
        ("/style.css", None),
        ("/_next/static/chunks/app.js", None),
        ("/missing.png", None),
    ];
    for (target, token) in cases {
        let get = env.send(request("GET", target, token)).await;
        let head = env.send(request("HEAD", target, token)).await;
        assert_eq!(answer(&head), answer(&get), "{target} {token:?}");
        assert_eq!(
            head.header("Content-Length"),
            Some(get.body.len().to_string().as_str()),
            "{target}"
        );
        assert!(head.body.is_empty(), "{target}");
    }
    assert_eq!(
        env.send(request("HEAD", "/api/v1/pool/list", Some(&player)))
            .await
            .status,
        403
    );
}

#[tokio::test(flavor = "current_thread")]
async fn login_without_usable_token_is_a_plain_login_failure() {
    let env = Env::new().await;
    let (captured, _guard) = capture();
    for email in ["notoken@x", "user0@x", "empty@x", "html@x"] {
        captured.take();
        let r = env
            .call("POST", "/api/v1/login", None, Some(&login(email)))
            .await;
        assert_eq!(r.status, 400, "{email}");
        assert_eq!(r.json(), error_body(LOGIN_FAILED), "{email}");
        let logs: Vec<String> = captured
            .take()
            .into_iter()
            .filter(|l| l.contains("Server:"))
            .collect();
        assert_eq!(logs, [NO_TOKEN_WARN], "{email}");
    }
    let r = env
        .call("POST", "/api/v1/login", None, Some(&login("user7@x")))
        .await;
    assert_eq!(r.json()["userId"], json!(7));
    assert_eq!(
        captured.take(),
        ["INFO Server: HTTP login: user 7 isAdmin=false"]
    );
}

fn listing(r: &Reply) -> Vec<String> {
    r.json()["backups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn backups_keep_the_newest_twenty_per_kind() {
    let env = Env::build(HttpConfig::default(), |dir| {
        let backups = dir.join("data/backups");
        std::fs::create_dir_all(&backups).unwrap();
        for i in 0..25 {
            let name = format!("admins-20200101-000000-{i:03}.json");
            std::fs::write(backups.join(name), "[1]").unwrap();
        }
        std::fs::create_dir(backups.join("admins-20190101-000000-000.json")).unwrap();
        for i in 0..3 {
            let name = format!("points-20200101-000000-{i:03}.json");
            std::fs::write(backups.join(name), "{}").unwrap();
        }
    })
    .await;
    let (captured, _guard) = capture_everywhere();
    let r = env
        .call(
            "POST",
            "/api/v1/data/admins/import?mode=merge",
            Some(&env.admin.clone()),
            Some("[5]"),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let written = r.json()["backup"].as_str().unwrap().to_owned();

    let admins = listing(&env.get("/api/v1/data/admins/backups").await);
    let mut expected = vec![written.rsplit('/').next().unwrap().to_owned()];
    expected.extend(
        (6..25)
            .rev()
            .map(|i| format!("admins-20200101-000000-{i:03}.json")),
    );
    expected.push("admins-20190101-000000-000.json".to_owned());
    assert_eq!(admins, expected);
    assert_eq!(
        listing(&env.get("/api/v1/data/points/backups").await).len(),
        3
    );
    let warnings: Vec<String> = captured
        .take()
        .into_iter()
        .filter(|l| l.starts_with("WARN") && l.contains("admins-20190101-000000-000.json"))
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].starts_with("WARN Server: Could not delete old backup "));
}

#[tokio::test]
async fn method_override_header_is_ignored() {
    let env = Env::new().await;
    let with_override = |method: &str, target: &str, value: &str| {
        Request::builder()
            .method(method)
            .uri(target)
            .header("Authorization", format!("Bearer {}", env.admin))
            .header("X-HTTP-Method-Override", value)
            .body(Body::empty())
            .unwrap()
    };
    let r = env
        .send(with_override("POST", "/api/v1/pool/list", "GET"))
        .await;
    assert_eq!(r.status, 404);
    assert_eq!(r.text(), "Endpoint POST /api/v1/pool/list not found");
    let r = env
        .send(with_override("GET", "/api/v1/pool/list", "DELETE"))
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["ok"], json!(true));
    let r = env
        .send(with_override("DELETE", "/api/v1/chart/search", "GET"))
        .await;
    assert_eq!(r.text(), "Endpoint DELETE /api/v1/chart/search not found");
}

async fn search(env: &Env, query: &str) -> Value {
    let r = env.get(&format!("/api/v1/chart/search?{query}")).await;
    r.json()
}

#[tokio::test(flavor = "current_thread")]
async fn refresh_full_starts_a_full_sweep_whatever_the_division() {
    const FULL: &str = "Running a full chart index sweep on request";
    let env = Env::new().await;
    let (captured, _guard) = capture();
    let refused = env.get("/api/v1/chart/search?refresh=full&limit=a").await;
    assert_eq!(refused.status, 400);
    let plain = search(&env, "limit=1").await;

    let v = search(&env, "refresh=full&division=plain&limit=1").await;
    assert_eq!(v["result"]["refreshing"], json!(true));
    let keys = |v: &Value| {
        v["result"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&v), keys(&plain));
    let mut seen = Vec::new();
    assert!(wait_for(&captured, &mut seen, FULL).await, "{seen:?}");
    assert_eq!(seen.iter().filter(|l| l.contains(FULL)).count(), 1);

    let other = Env::new().await;
    let mut seen = Vec::new();
    search(&other, "refresh=1&division=plain&limit=1").await;
    assert!(
        wait_for(&captured, &mut seen, "(division=plain)").await,
        "{seen:?}"
    );
    assert!(!seen.iter().any(|l| l.contains(FULL)), "{seen:?}");
}

#[tokio::test]
async fn only_next_static_files_are_cached_for_a_year() {
    let env = Env::build(HttpConfig::default(), |dir| {
        let dist = dir.join("frontend/dist");
        std::fs::create_dir_all(dist.join("_next/staticx")).unwrap();
        std::fs::write(dist.join("_next/staticx/a.js"), "x").unwrap();
        std::fs::write(dist.join("_next/build.json"), "{}").unwrap();
    })
    .await;
    let cache = |target: &'static str| {
        let r = env.send(request("GET", target, None));
        async move {
            let r = r.await;
            (
                r.status.as_u16(),
                r.header("Cache-Control").map(str::to_owned),
            )
        }
    };
    let year = Some("public, max-age=31536000, immutable".to_owned());
    let short = Some("public, max-age=300".to_owned());
    assert_eq!(cache("/_next/static/chunks/app.js").await, (200, year));
    for target in [
        "/_next/staticx/a.js",
        "/_next/build.json",
        "/style.css",
        "/logo.jpg",
        "/LICENSE",
        "/docs/readme.txt",
        "/manifest.webmanifest",
    ] {
        assert_eq!(cache(target).await, (200, short.clone()), "{target}");
    }
    for target in [
        "/",
        "/index.html",
        "/room/7",
        "/pool/__fallback__/index.html",
    ] {
        assert_eq!(cache(target).await, (200, None), "{target}");
    }
    assert_eq!(cache("/_next/static/missing.js").await, (404, None));
}

async fn restore(env: &Env, kind: &str, name: &str) -> Reply {
    env.call(
        "POST",
        &format!("/api/v1/data/{kind}/restore"),
        Some(&env.admin.clone()),
        Some(&json!({"backup": name}).to_string()),
    )
    .await
}

#[tokio::test]
async fn a_backup_restores_only_as_its_own_kind() {
    let env = Env::new().await;
    let admin = env.admin.clone();
    let r = env
        .call(
            "POST",
            "/api/v1/data/admins/import?mode=merge",
            Some(&admin),
            Some("[5]"),
        )
        .await;
    assert_eq!(r.status, 200);
    let r = env
        .call(
            "POST",
            "/api/v1/data/points/import",
            Some(&admin),
            Some("{\"5\":{\"points\":10}}"),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let admins_backup = env.backups("admins")[0].clone();
    let points_backup = env.backups("points")[0].clone();
    for name in [
        points_backup.clone(),
        format!("admins-x/../{points_backup}"),
        format!("admins-/../{points_backup}"),
        format!("admins-x\\..\\{points_backup}"),
        format!("{admins_backup}/"),
        format!("./{admins_backup}"),
        format!("admins-x/{admins_backup}"),
    ] {
        let r = restore(&env, "admins", &name).await;
        assert_eq!(r.json(), error_body("备份不存在"), "{name}");
        let r = restore(
            &env,
            "points",
            &name.replace(&points_backup, &admins_backup),
        )
        .await;
        assert_eq!(r.json(), error_body("备份不存在"), "{name}");
    }
    assert_eq!(
        env.get("/api/v1/admin").await.json()["admins"],
        json!([1, 5])
    );
    let r = restore(&env, "admins", &admins_backup).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(env.get("/api/v1/admin").await.json()["admins"], json!([1]));
}
