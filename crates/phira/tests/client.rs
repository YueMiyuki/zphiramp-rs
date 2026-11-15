mod support;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use support::server::{Reply, Request, Server};
use zphira_common::gson::JsonElement;
use zphira_phira::{ChartInfo, Options, PhiraClient, PhiraError};

async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .expect("test step exceeded its 30 s bound")
}

fn fast() -> Options {
    Options {
        retry_delay: Duration::from_millis(20),
        ..Options::default()
    }
}

fn client(server: &Server, options: Options) -> PhiraClient {
    PhiraClient::with_options(server.base_url(), options).unwrap()
}

fn user_json(id: i32) -> String {
    format!(r#"{{"id":{id},"name":"user{id}","rks":12.5,"lastLogin":"2026-10-01T12:00:00Z"}}"#)
}

fn chart_json(id: i32, file: &str) -> String {
    format!(
        r#"{{"id":{id},"name":"chart{id}","level":"AT 17","difficulty":16.5,"file":"{file}","tags":["regular"]}}"#
    )
}

fn record_json(id: i32) -> String {
    format!(
        r#"{{"id":{id},"player":7,"chart":42,"score":999000,"accuracy":0.99,"max_combo":100,"time":"2026-10-01T12:00:00Z"}}"#
    )
}

fn api(request: &Request, _index: usize) -> Reply {
    let target = request.target.as_str();
    let id = |prefix: &str| {
        target
            .strip_prefix(prefix)
            .and_then(|n| n.parse::<i32>().ok())
    };
    if target == "/me" {
        Reply::json(&user_json(1))
    } else if let Some(n) = id("/user/") {
        Reply::json(&user_json(n))
    } else if let Some(n) = id("/chart/") {
        Reply::json(&chart_json(n, "https://example.invalid/file"))
    } else if let Some(n) = id("/record/") {
        Reply::json(&record_json(n))
    } else if target.starts_with("/record?player=") {
        Reply::json(&format!("[{},{}]", record_json(1), record_json(2)))
    } else if target.starts_with("/chart?page=") {
        Reply::json(r#"{"count":61,"results":[{"id":5,"name":"a"},{"id":6,"name":"b"}]}"#)
    } else if target == "/login" {
        Reply::json(
            r#"{"id":9,"token":"tok","refresh_token":"ref","expire_at":"2026-12-01T00:00:00Z"}"#,
        )
    } else {
        Reply::status(404, format!("no route {target}"))
    }
}

#[tokio::test]
async fn request_shapes_match_java() {
    let server = Server::start(api).await;
    let phira = client(&server, fast());
    bounded(async {
        let me = phira.user_by_token("secret-token").await.unwrap();
        assert_eq!((me.id, me.name.as_deref()), (1, Some("user1")));
        assert_eq!(me.rks, 12.5);
        assert_eq!(
            me.last_login.as_ref().map(|t| t.to_string()),
            Some("2026-10-01T12:00Z".into())
        );
        phira.user_by_id(77).await.unwrap();
        phira.chart(42).await.unwrap();
        phira.record(9).await.unwrap();
        phira.recent_records(7).await.unwrap();
        phira.chart_page(3, None).await.unwrap();
        phira.chart_page(4, Some("plain")).await.unwrap();
        phira.chart_page(5, Some("   ")).await.unwrap();
        phira.chart_page(6, Some("")).await.unwrap();
    })
    .await;

    let requests = server.requests();
    let shapes: Vec<(&str, &str)> = requests
        .iter()
        .map(|r| (r.method.as_str(), r.target.as_str()))
        .collect();
    assert_eq!(
        shapes,
        [
            ("GET", "/me"),
            ("GET", "/user/77"),
            ("GET", "/chart/42"),
            ("GET", "/record/9"),
            ("GET", "/record?player=7&order=-time&pageNum=10"),
            ("GET", "/chart?page=3"),
            ("GET", "/chart?page=4&division=plain"),
            ("GET", "/chart?page=5"),
            ("GET", "/chart?page=6"),
        ]
    );
    for r in &requests {
        assert_eq!(r.header("user-agent"), Some("JPhira/1"), "{}", r.target);
        assert_eq!(r.header("accept"), Some("application/json"), "{}", r.target);
        assert_eq!(r.header("accept-encoding"), None, "{}", r.target);
        assert!(r.body.is_empty());
        if r.target == "/me" {
            assert_eq!(r.header("authorization"), Some("Bearer secret-token"));
        } else {
            assert_eq!(r.header("authorization"), None, "{}", r.target);
        }
    }
}

#[tokio::test]
async fn login_posts_a_json_body() {
    let server = Server::start(api).await;
    let phira = client(&server, fast());
    let result = bounded(phira.login("player@example.com", "pw<&>'=\"\\ü"))
        .await
        .unwrap();
    assert_eq!(result.id, 9);
    assert_eq!(result.token.as_deref(), Some("tok"));
    assert_eq!(result.refresh_token.as_deref(), Some("ref"));
    assert_eq!(result.expire_at.as_deref(), Some("2026-12-01T00:00:00Z"));

    let [request] = <[Request; 1]>::try_from(server.requests()).unwrap();
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("POST", "/login")
    );
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("accept"), Some("application/json"));
    assert_eq!(request.header("user-agent"), Some("JPhira/1"));
    assert_eq!(request.header("authorization"), None);
    let body = request.body_text();
    assert_eq!(
        request.header("content-length"),
        Some(body.len().to_string().as_str())
    );
    assert!(body.starts_with("{\n  \""), "{body:?}");
    assert!(
        body.contains("\\u003c")
            && body.contains("\\u0026")
            && body.contains("\\u003d")
            && body.contains("\\u0027"),
        "{body}"
    );
    let parsed = JsonElement::parse(&body).unwrap();
    let object = parsed.as_object().unwrap();
    assert_eq!(object.len(), 2);
    assert_eq!(
        object.get("email").unwrap().get_as_string().as_deref(),
        Some("player@example.com")
    );
    assert_eq!(
        object.get("password").unwrap().get_as_string().as_deref(),
        Some("pw<&>'=\"\\ü")
    );
}

#[tokio::test]
async fn login_and_chart_pages_are_not_cached() {
    let server = Server::start(api).await;
    let phira = client(&server, fast());
    bounded(async {
        for _ in 0..2 {
            phira.login("a", "b").await.unwrap();
            let page = phira.chart_page(1, Some("plain")).await.unwrap();
            assert_eq!(page.count, 61);
            let results = page.results.unwrap();
            assert_eq!(results.iter().map(|c| c.id).collect::<Vec<_>>(), [5, 6]);
        }
    })
    .await;
    assert_eq!(server.request_count(), 4);
}

#[tokio::test]
async fn cached_values_are_served_without_a_second_request() {
    let server = Server::start(api).await;
    let phira = client(&server, fast());
    bounded(async {
        let first = phira.user_by_token("tok-a").await.unwrap();
        let second = phira.user_by_token("tok-a").await.unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "the same Arc comes out of the cache"
        );
        assert_eq!(server.request_count(), 1);
        phira.user_by_token("tok-b").await.unwrap();
        assert_eq!(server.request_count(), 2);

        phira.user_by_id(5).await.unwrap();
        phira.user_by_id(5).await.unwrap();
        phira.chart(6).await.unwrap();
        phira.chart(6).await.unwrap();
        phira.record(7).await.unwrap();
        phira.record(7).await.unwrap();
        phira.recent_records(8).await.unwrap();
        phira.recent_records(8).await.unwrap();
        assert_eq!(server.request_count(), 6);

        phira.user_by_id(1).await.unwrap();
        assert_eq!(server.request_count(), 7);
        let clone = phira.clone();
        clone.chart(6).await.unwrap();
        clone.chart(99).await.unwrap();
        phira.chart(99).await.unwrap();
        assert_eq!(server.request_count(), 8);
    })
    .await;
}

#[tokio::test]
async fn only_successful_results_are_cached() {
    let outcome = Arc::new(AtomicUsize::new(0));
    let o = outcome.clone();
    let server = Server::start(move |request, _| match o.load(Ordering::SeqCst) {
        0 => Reply::status(404, "not yet"),
        1 => Reply::ok("this is not json"),
        2 => Reply::ok(""),
        _ => api(request, 0),
    })
    .await;
    let phira = client(&server, fast());
    bounded(async {
        for stage in 0..3 {
            outcome.store(stage, Ordering::SeqCst);
            assert!(phira.user_by_id(3).await.is_err(), "stage {stage}");
            assert!(phira.chart(3).await.is_err());
            assert!(phira.record(3).await.is_err());
            assert!(phira.user_by_token("t").await.is_err());
        }
        let before = server.request_count();
        outcome.store(3, Ordering::SeqCst);
        assert_eq!(phira.user_by_id(3).await.unwrap().id, 3);
        assert_eq!(phira.chart(3).await.unwrap().id, 3);
        assert_eq!(phira.record(3).await.unwrap().id, 3);
        assert_eq!(phira.user_by_token("t").await.unwrap().id, 1);
        assert_eq!(server.request_count(), before + 4);
        phira.user_by_id(3).await.unwrap();
        phira.chart(3).await.unwrap();
        assert_eq!(server.request_count(), before + 4);
    })
    .await;
}

#[tokio::test]
async fn recent_records_expire_after_ten_seconds_scaled_down_here() {
    let server = Server::start(api).await;
    let phira = client(
        &server,
        Options {
            recent_record_ttl: Duration::from_millis(600),
            ..fast()
        },
    );
    bounded(async {
        let first = phira.recent_records(7).await.unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!((first[0].id, first[0].max_combo), (1, 100));
        tokio::time::sleep(Duration::from_millis(100)).await;
        phira.recent_records(7).await.unwrap();
        assert_eq!(server.request_count(), 1, "still fresh");
        phira.recent_records(8).await.unwrap();
        assert_eq!(server.request_count(), 2);
        tokio::time::sleep(Duration::from_millis(800)).await;
        phira.recent_records(7).await.unwrap();
        assert_eq!(server.request_count(), 3, "expired, fetched again");
        phira.chart(1).await.unwrap();
        tokio::time::sleep(Duration::from_millis(350)).await;
        phira.chart(1).await.unwrap();
        assert_eq!(server.request_count(), 4);
    })
    .await;
}

#[tokio::test]
async fn an_empty_or_null_recent_records_answer_is_an_empty_list() {
    for body in ["", "null", "[]", "  "] {
        let server = Server::start(move |_, _| Reply::ok(body)).await;
        let phira = client(&server, fast());
        let records = bounded(phira.recent_records(1)).await.unwrap();
        assert!(records.is_empty(), "{body:?}");
        bounded(phira.recent_records(1)).await.unwrap();
        assert_eq!(server.request_count(), 1, "{body:?}");
    }
}

#[tokio::test]
async fn non_2xx_display_is_http_status_and_body() {
    let server = Server::start(|_, _| Reply::status(404, "chart gone: 丢失 <&>")).await;
    let phira = client(&server, fast());
    let error = bounded(phira.chart(1)).await.unwrap_err();
    assert_eq!(error.to_string(), "HTTP 404: chart gone: 丢失 <&>");
    assert_eq!(error.status(), Some(404));
    assert_eq!(error.body(), Some("chart gone: 丢失 <&>"));
    assert!(!error.is_transport());

    let server = Server::start(|_, _| Reply::status(401, "")).await;
    let phira = client(&server, fast());
    let error = bounded(phira.user_by_token("bad")).await.unwrap_err();
    assert_eq!(error.to_string(), "HTTP 401: ");

    let server = Server::start(|_, _| Reply::status(400, vec![b'o', b'k', 0xFF, 0xFE])).await;
    let phira = client(&server, fast());
    let error = bounded(phira.chart(1)).await.unwrap_err();
    assert_eq!(error.to_string(), "HTTP 400: ok\u{fffd}\u{fffd}");
}

#[tokio::test]
async fn client_errors_are_not_retried() {
    for status in [400u16, 401, 403, 404, 405, 408, 409, 410, 418, 422, 451] {
        let server =
            Server::start(move |_, _| Reply::status(status, format!("body {status}"))).await;
        let phira = client(
            &server,
            Options {
                retry_delay: Duration::from_secs(5),
                ..Options::default()
            },
        );
        let started = Instant::now();
        let error = bounded(phira.user_by_token("t")).await.unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{status} did not wait for a backoff"
        );
        assert_eq!(error.to_string(), format!("HTTP {status}: body {status}"));
        assert_eq!(server.request_count(), 1, "status {status} is not retried");
    }
    let server = Server::start(|_, _| Reply::status(401, "no")).await;
    let phira = client(
        &server,
        Options {
            retry_delay: Duration::from_secs(5),
            ..Options::default()
        },
    );
    bounded(async {
        assert!(phira.user_by_id(1).await.is_err());
        assert!(phira.chart(1).await.is_err());
        assert!(phira.record(1).await.is_err());
        assert!(phira.recent_records(1).await.is_err());
        assert!(phira.login("a", "b").await.is_err());
        assert!(phira.chart_page(1, None).await.is_err());
    })
    .await;
    assert_eq!(server.request_count(), 6);
}

#[tokio::test]
async fn server_errors_are_retried_eight_times() {
    for status in [500u16, 502, 503, 504, 429, 599] {
        let server =
            Server::start(move |_, index| Reply::status(status, format!("attempt {}", index + 1)))
                .await;
        let phira = client(&server, fast());
        let error = bounded(phira.chart(1)).await.unwrap_err();
        assert_eq!(server.request_count(), 8, "status {status}");
        assert_eq!(
            error.to_string(),
            format!("HTTP {status}: attempt 8"),
            "the last answer is reported"
        );
        assert_eq!(error.status(), Some(status));
    }
}

#[tokio::test]
async fn a_503_can_succeed_on_a_later_attempt() {
    for failures in [1usize, 2, 7] {
        let server = Server::start(move |request, index| {
            if index < failures {
                Reply::status(503, "busy")
            } else {
                api(request, index)
            }
        })
        .await;
        let phira = client(&server, fast());
        let chart = bounded(phira.chart(42)).await.unwrap();
        assert_eq!(chart.id, 42);
        assert_eq!(
            server.request_count(),
            failures + 1,
            "{failures} failures then success"
        );
        bounded(phira.chart(42)).await.unwrap();
        assert_eq!(server.request_count(), failures + 1);
    }
    let server = Server::start(|request, index| {
        if index < 8 {
            Reply::status(503, "busy")
        } else {
            api(request, index)
        }
    })
    .await;
    let phira = client(&server, fast());
    assert!(bounded(phira.chart(42)).await.is_err());
    assert_eq!(server.request_count(), 8);
}

#[tokio::test]
async fn backoff_waits_n_times_the_unit() {
    let unit = Duration::from_millis(25);
    let server = Server::start(|_, _| Reply::status(503, "busy")).await;
    let phira = client(
        &server,
        Options {
            retry_delay: unit,
            ..Options::default()
        },
    );
    let started = Instant::now();
    bounded(phira.record(1)).await.unwrap_err();
    let total = started.elapsed();

    let requests = server.requests();
    assert_eq!(requests.len(), 8);
    for (n, pair) in requests.windows(2).enumerate() {
        let gap = pair[1].at - pair[0].at;
        let wait = unit * (n as u32 + 1);
        assert!(
            gap >= wait,
            "gap {} was {gap:?}, expected at least {wait:?}",
            n + 1
        );
    }
    assert!(total >= Duration::from_millis(700), "{total:?}");
    assert!(total < Duration::from_millis(700 + 2500), "{total:?}");
}

#[tokio::test]
async fn attempts_and_unit_are_options() {
    let server = Server::start(|_, _| Reply::status(500, "x")).await;
    let phira = client(
        &server,
        Options {
            max_attempts: 3,
            retry_delay: Duration::from_millis(1),
            ..Options::default()
        },
    );
    bounded(phira.chart(1)).await.unwrap_err();
    assert_eq!(server.request_count(), 3);
    let server = Server::start(|_, _| Reply::status(500, "x")).await;
    let phira = client(
        &server,
        Options {
            max_attempts: 0,
            ..fast()
        },
    );
    bounded(phira.chart(1)).await.unwrap_err();
    assert_eq!(server.request_count(), 1, "at least one attempt");
}

#[tokio::test]
async fn a_transport_failure_is_retried() {
    let server = Server::start(|request, index| {
        if index < 3 {
            Reply::Drop
        } else {
            api(request, index)
        }
    })
    .await;
    let phira = client(&server, fast());
    let user = bounded(phira.user_by_id(12)).await.unwrap();
    assert_eq!(user.id, 12);
    assert_eq!(server.request_count(), 4);

    let server = Server::start(|_, _| Reply::Drop).await;
    let phira = client(&server, fast());
    let error: PhiraError = bounded(phira.user_by_id(12)).await.unwrap_err();
    assert_eq!(server.request_count(), 8);
    assert!(error.is_transport() && error.status().is_none(), "{error}");
    assert!(!error.to_string().is_empty());
}

#[tokio::test]
async fn connection_refused_is_retried_and_reported() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let phira = PhiraClient::with_options(
        format!("http://127.0.0.1:{port}/"),
        Options {
            max_attempts: 4,
            retry_delay: Duration::from_millis(20),
            ..Options::default()
        },
    )
    .unwrap();
    let started = Instant::now();
    let error = bounded(phira.chart(1)).await.unwrap_err();
    assert!(error.is_transport(), "{error}");
    assert!(error.to_string().contains("connect"), "{error}");
    assert!(
        started.elapsed() >= Duration::from_millis(120),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn an_answer_that_never_comes_times_out_and_is_retried() {
    let server = Server::start(|_, _| Reply::Hang).await;
    let phira = client(
        &server,
        Options {
            max_attempts: 2,
            retry_delay: Duration::from_millis(10),
            request_timeout: Duration::from_millis(200),
            ..Options::default()
        },
    );
    let started = Instant::now();
    let error = bounded(phira.chart(1)).await.unwrap_err();
    assert_eq!(server.request_count(), 2);
    assert!(
        error.is_transport() && error.to_string().contains("timed out"),
        "{error}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(410),
        "{:?}",
        started.elapsed()
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn an_answer_that_stalls_mid_body_times_out() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nabc")
                    .await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            });
        }
    });
    let phira = PhiraClient::with_options(
        format!("http://{addr}/"),
        Options {
            max_attempts: 1,
            request_timeout: Duration::from_millis(300),
            ..Options::default()
        },
    )
    .unwrap();
    let error = bounded(phira.chart(1)).await.unwrap_err();
    assert!(error.is_transport(), "{error}");
    task.abort();
}

#[tokio::test]
async fn undecodable_answers_are_not_retried() {
    let server = Server::start(|_, _| Reply::ok("{\"id\": \"not a number\"}")).await;
    let phira = client(&server, fast());
    let error = bounded(phira.user_by_id(1)).await.unwrap_err();
    assert_eq!(server.request_count(), 1, "a 2xx answer is never retried");
    assert!(error.status().is_none() && !error.is_transport(), "{error}");
    assert!(!error.to_string().is_empty());

    for body in ["", "null", "   "] {
        let server = Server::start(move |_, _| Reply::ok(body)).await;
        let phira = client(&server, fast());
        let error = bounded(phira.chart(1)).await.unwrap_err();
        assert_eq!(server.request_count(), 1);
        assert!(error.status().is_none() && !error.is_transport(), "{error}");
        assert_eq!(error.to_string(), "Empty response from Phira");
    }
}

#[tokio::test]
async fn gson_reading_rules_apply_to_upstream_bodies() {
    let server = Server::start(|_, _| {
        Reply::ok("{id: '5', name: 12, rks: \"3.5\", banned: \"TRUE\", extra: [1,{}], joined: \"2026-01-02T03:04:05.5+08:00\"}")
    })
    .await;
    let phira = client(&server, fast());
    let user = bounded(phira.user_by_id(5)).await.unwrap();
    assert_eq!(
        (user.id, user.name.as_deref(), user.rks, user.banned),
        (5, Some("12"), 3.5, true)
    );
    assert_eq!(
        user.joined.as_ref().map(|t| t.to_string()),
        Some("2026-01-02T03:04:05.500+08:00".to_owned())
    );
}

#[tokio::test]
async fn redirects_are_followed_with_the_same_headers() {
    let target = Server::start(api).await;
    let target_base = target.base_url();
    let origin = Server::start(move |request, _| {
        Reply::redirect(
            302,
            &format!("{target_base}{}", request.target.trim_start_matches('/')),
        )
    })
    .await;
    let phira = client(&origin, fast());
    let chart = bounded(phira.chart(42)).await.unwrap();
    assert_eq!(chart.id, 42);
    assert_eq!(origin.request_count(), 1);
    let [followed] = <[Request; 1]>::try_from(target.requests()).unwrap();
    assert_eq!(followed.target, "/chart/42");
    assert_eq!(followed.header("user-agent"), Some("JPhira/1"));
    assert_eq!(followed.header("accept"), Some("application/json"));
}

#[tokio::test]
async fn authorization_is_not_forwarded_to_another_origin() {
    let target = Server::start(api).await;
    let target_base = target.base_url();
    let origin = Server::start(move |request, _| {
        Reply::redirect(
            302,
            &format!("{target_base}{}", request.target.trim_start_matches('/')),
        )
    })
    .await;
    let phira = client(&origin, fast());
    assert_eq!(bounded(phira.user_by_token("secret")).await.unwrap().id, 1);
    assert_eq!(
        origin.requests()[0].header("authorization"),
        Some("Bearer secret")
    );
    let [followed] = <[Request; 1]>::try_from(target.requests()).unwrap();
    assert_eq!(followed.header("authorization"), None);
    assert_eq!(followed.header("user-agent"), Some("JPhira/1"));
    assert_eq!(followed.header("accept"), Some("application/json"));
}

#[tokio::test]
async fn at_most_four_redirects_are_followed() {
    for (hops, ok) in [(0usize, true), (1, true), (4, true), (5, false), (6, false)] {
        let base = Arc::new(std::sync::OnceLock::<String>::new());
        let b = base.clone();
        let origin = Server::start(move |request, _| {
            let remaining = match request.target.strip_prefix("/hop/") {
                Some(n) => n.parse::<usize>().unwrap(),
                None => hops,
            };
            if remaining == 0 {
                Reply::json(&chart_json(7, "x"))
            } else {
                Reply::redirect(302, &format!("{}hop/{}", b.get().unwrap(), remaining - 1))
            }
        })
        .await;
        base.set(origin.base_url()).unwrap();
        let phira = client(&origin, fast());
        let result = bounded(phira.chart(7)).await;
        if ok {
            assert_eq!(result.unwrap().id, 7, "{hops} redirects");
            assert_eq!(origin.request_count(), hops + 1);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.status(), Some(302), "{hops} redirects");
            assert!(error.to_string().starts_with("HTTP 302: "), "{error}");
            assert_eq!(origin.request_count(), 5, "{hops} redirects");
        }
    }
}

#[tokio::test]
async fn a_redirect_without_location_is_an_error_and_not_retried() {
    let server = Server::start(|_, _| Reply::status(302, "moved")).await;
    let phira = client(&server, fast());
    let error = bounded(phira.chart(1)).await.unwrap_err();
    assert_eq!(error.to_string(), "HTTP 302: moved");
    assert_eq!(server.request_count(), 1);
    let server = Server::start(|_, _| Reply::status(304, "")).await;
    let phira = client(&server, fast());
    assert_eq!(
        bounded(phira.chart(1)).await.unwrap_err().status(),
        Some(304)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_callers_do_not_block_each_other() {
    let server = Server::start(|request, index| {
        if request.target == "/chart/1" {
            Reply::after(Duration::from_millis(600), api(request, index))
        } else {
            Reply::after(Duration::from_millis(150), api(request, index))
        }
    })
    .await;
    let phira = client(&server, fast());
    bounded(async {
        phira.chart(2).await.unwrap();

        let slow = {
            let phira = phira.clone();
            tokio::spawn(async move { phira.chart(1).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;

        let started = Instant::now();
        phira.chart(2).await.unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "{:?}",
            started.elapsed()
        );

        let started = Instant::now();
        let calls: Vec<_> = (10..26)
            .map(|id| {
                let phira = phira.clone();
                tokio::spawn(async move { phira.user_by_id(id).await.map(|u| u.id) })
            })
            .collect();
        for (id, call) in (10..26).zip(calls) {
            assert_eq!(call.await.unwrap().unwrap(), id);
        }
        assert!(
            started.elapsed() < Duration::from_millis(1000),
            "16 x 150 ms ran in {:?}",
            started.elapsed()
        );

        assert_eq!(slow.await.unwrap().unwrap().id, 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_misses_both_load() {
    let server = Server::start(|request, index| {
        Reply::after(Duration::from_millis(200), api(request, index))
    })
    .await;
    let phira = client(&server, fast());
    let (a, b) = bounded(async { tokio::join!(phira.chart(5), phira.chart(5)) }).await;
    assert_eq!((a.unwrap().id, b.unwrap().id), (5, 5));
    assert_eq!(server.request_count(), 2);
    bounded(phira.chart(5)).await.unwrap();
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn charts_are_shared_arcs_with_a_mutable_duration() {
    let server = Server::start(api).await;
    let phira = client(&server, fast());
    let held_by_a_room: Arc<ChartInfo> = bounded(phira.chart(42)).await.unwrap();
    assert_eq!(held_by_a_room.duration_seconds(), None);
    let from_cache = bounded(phira.chart(42)).await.unwrap();
    from_cache.set_duration_seconds(Some(188));
    assert_eq!(held_by_a_room.duration_seconds(), Some(188));
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn https_is_supported_and_fails_cleanly_against_a_plain_server() {
    let server = Server::start(|_, _| Reply::Hang).await;
    let phira = PhiraClient::with_options(
        format!("https://{}/", server.addr),
        Options {
            max_attempts: 1,
            request_timeout: Duration::from_millis(500),
            ..Options::default()
        },
    )
    .unwrap();
    let error = bounded(phira.chart(1)).await.unwrap_err();
    assert!(error.is_transport(), "{error}");
}

#[tokio::test]
async fn bad_tokens_and_urls_are_errors_not_panics() {
    let server = Server::start(api).await;
    let phira = client(&server, fast());
    let error = bounded(phira.user_by_token("bad\ntoken"))
        .await
        .unwrap_err();
    assert!(error.to_string().starts_with("Invalid request"), "{error}");
    assert_eq!(server.request_count(), 0);
    let phira = PhiraClient::new("not a url").unwrap();
    let error = bounded(phira.chart(1)).await.unwrap_err();
    assert!(error.to_string().starts_with("Invalid URL"), "{error}");
    let server = Server::start(api).await;
    let phira = client(&server, fast());
    bounded(phira.chart_page(1, Some("plain&order=x")))
        .await
        .unwrap();
    assert_eq!(
        server.requests()[0].target,
        "/chart?page=1&division=plain&order=x"
    );
}
