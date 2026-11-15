mod support;

use axum::body::Body;
use http::Request;
use serde_json::json;
use support::{Env, error_body};

#[tokio::test]
async fn admin_status_is_checked_live() {
    let env = Env::new().await;
    let claim_admin = env.state.jwt().issue_token(2, true);
    assert_eq!(
        env.call("GET", "/api/v1/pool/list", Some(&claim_admin), None)
            .await
            .json(),
        error_body("需要权限")
    );
    let user5 = env.user5.clone();
    assert_eq!(
        env.call("GET", "/api/v1/admin", Some(&user5), None)
            .await
            .status,
        403
    );
    env.call(
        "POST",
        "/api/v1/admin",
        Some(&env.admin.clone()),
        Some("{\"userId\":5}"),
    )
    .await;
    assert_eq!(
        env.call("GET", "/api/v1/admin", Some(&user5), None)
            .await
            .status,
        200
    );
    env.call("DELETE", "/api/v1/admin/5", Some(&env.admin.clone()), None)
        .await;
    assert_eq!(
        env.call("GET", "/api/v1/admin", Some(&user5), None)
            .await
            .status,
        403
    );

    std::fs::write(env.data.join("admins.json"), "[1, 2]").unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(
        env.call("GET", "/api/v1/admin", Some(&claim_admin), None)
            .await
            .json(),
        json!({"ok":true,"admins":[1,2]})
    );
}

#[tokio::test]
async fn tokens_from_another_secret_or_expired_are_refused() {
    let env = Env::new().await;
    let expired = env.state.jwt().issue_token_at(1, true, 1_600_000_000_000);
    let foreign = zphira_http::JwtService::from_secret(vec![7; 32])
        .unwrap()
        .issue_token(1, true);
    for token in [expired, foreign, format!("{}x", env.admin)] {
        let r = env
            .call("GET", "/api/v1/pool/list", Some(&token), None)
            .await;
        assert_eq!(r.status, 401);
        assert_eq!(r.json(), error_body("需要登录"));
    }
}

#[tokio::test]
async fn login_failures() {
    let env = Env::new().await;
    let r = env
        .call(
            "POST",
            "/api/v1/login",
            None,
            Some("{\"email\":\"down@x\",\"password\":\"right\"}"),
        )
        .await;
    assert_eq!(r.json(), error_body("登录失败，请检查邮箱与密码"));
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/login")
        .header("Content-Type", "application/json; charset=ISO-8859-1")
        .body(Body::from(
            b"{\"email\":\"user3@x\",\"password\":\"r\xE9ght\"}".to_vec(),
        ))
        .unwrap();
    assert_eq!(
        env.send(request).await.json(),
        error_body("登录失败，请检查邮箱与密码")
    );
    let calls = env.phira.requests.lock().unwrap().clone();
    assert!(calls.iter().filter(|c| c.as_str() == "POST /login").count() >= 2);
}

#[tokio::test]
async fn static_traversal_attempts() {
    let env = Env::new().await;
    let cases: &[(&str, u16, &str)] = &[
        ("/../secret.txt", 400, "jetty"),
        ("/%2e%2e/secret.txt", 400, "jetty"),
        ("/%2E%2E/secret.txt", 400, "jetty"),
        ("/room/../../secret.txt", 400, "jetty"),
        ("/docs/./../../secret.txt", 400, "jetty"),
        ("/..%2fsecret.txt", 400, "Bad Request"),
        ("/docs%2f..%2f..%2fsecret.txt", 400, "Bad Request"),
        ("/%252e%252e/secret.txt", 404, "Not Found"),
        ("/..%5csecret.txt", 400, "Bad Request"),
        ("/..\\secret.txt", 404, "Not Found"),
        ("//etc/passwd", 400, "Bad Request"),
        ("///secret.txt", 400, "Bad Request"),
        ("/%00", 400, "Bad Request"),
        ("/a%00.js", 400, "Bad Request"),
        ("/docs/../style.css", 200, "body{color:red}\n"),
    ];
    for (target, status, body) in cases {
        let request = Request::builder().uri(*target).body(Body::empty()).unwrap();
        let r = env.send(request).await;
        assert_eq!(r.status.as_u16(), *status, "{target}");
        if *body != "jetty" {
            assert_eq!(r.text(), *body, "{target}");
        }
        assert!(!r.text().contains("outside dist"), "{target}");
    }
}

#[tokio::test]
async fn missing_frontend_is_404() {
    let env = Env::build(Default::default(), |dir| {
        std::fs::remove_dir_all(dir.join("frontend/dist")).unwrap();
    })
    .await;
    for (target, content_type) in [
        ("/", "text/html;charset=utf-8"),
        ("/room/1", "text/html;charset=utf-8"),
        ("/pool/1", "text/html;charset=utf-8"),
        ("/x.js", "text/plain"),
        ("/index.html", "text/plain"),
        ("/api/x", "text/plain"),
    ] {
        let r = env
            .send(Request::builder().uri(target).body(Body::empty()).unwrap())
            .await;
        assert_eq!(r.status, 404, "{target}");
        assert_eq!(r.text(), "Not Found", "{target}");
        assert_eq!(r.header("Content-Type"), Some(content_type), "{target}");
        assert_eq!(r.header("Cache-Control"), None, "{target}");
    }
}

#[tokio::test]
async fn json_is_compact_gson() {
    let env = Env::new().await;
    let r = env.get("/api/v1/record/r1-1735689600000-1").await;
    assert_eq!(
        r.text(),
        "{\"ok\":true,\"record\":{\"id\":\"r1-1735689600000-1\",\"roomId\":\"r1\",\"chartId\":1000,\"chartName\":\"C1000\",\"startedAt\":1735689500000,\"finishedAt\":1735689600000,\"results\":[{\"playerId\":101,\"playerName\":\"Alice\",\"rank\":1,\"score\":998765,\"accuracy\":0.9876543,\"std\":0.012,\"gainedPoints\":100,\"totalPoints\":100},{\"playerId\":102,\"playerName\":\"Bob\\u003c\\u0026\\u003e\",\"rank\":2,\"score\":900000,\"accuracy\":0.95,\"std\":0.0,\"gainedPoints\":20,\"totalPoints\":20}]}}"
    );
    let r = env.get("/api/v1/chart/search?minRating=0.94&limit=1").await;
    assert_eq!(
        r.text(),
        "{\"ok\":true,\"result\":{\"indexed\":13,\"remoteTotal\":-1,\"refreshing\":false,\"matched\":3,\"pendingDuration\":false,\"charts\":[{\"id\":1008,\"name\":\"C1008\",\"level\":\"AT Lv.17\",\"difficulty\":16.0,\"charter\":\"ch1008\",\"composer\":\"comp1008\",\"illustrator\":null,\"description\":null,\"ranked\":true,\"reviewed\":true,\"stable\":true,\"stableRequest\":false,\"illustration\":\"https://img.example/1008.png\",\"preview\":null,\"file\":\"https://files.example/1008\",\"uploader\":0,\"tags\":[\"regular\"],\"rating\":0.99,\"ratingCount\":10,\"created\":\"2025-01-02T03:04:05.123456+08:00\",\"updated\":\"2025-03-04T05:06:07Z\",\"chartUpdated\":\"2025-03-04T05:06Z\",\"durationSeconds\":null}]}}"
    );
}
