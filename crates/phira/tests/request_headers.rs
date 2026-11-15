mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use support::read_tsv;
use support::server::{Reply, Request, Server};
use zphira_phira::{Options, PhiraClient};

const JDK_CLEARTEXT_UPGRADE: [&str; 3] = ["connection", "upgrade", "http2-settings"];

type Headers = BTreeMap<String, String>;

fn java_rows() -> Vec<(String, String, Headers)> {
    read_tsv("request_headers.tsv")
        .into_iter()
        .map(|row| {
            let [method, target, headers] = <[String; 3]>::try_from(row).expect("3 columns");
            let headers: Headers = headers
                .lines()
                .map(|l| {
                    let (name, value) = l.split_once(": ").expect("name: value");
                    (name.to_owned(), value.to_owned())
                })
                .filter(|(name, _)| !JDK_CLEARTEXT_UPGRADE.contains(&name.as_str()))
                .map(|(name, value)| {
                    if name == "host" {
                        (name, "<host>".to_owned())
                    } else {
                        (name, value)
                    }
                })
                .collect();
            (method, target, headers)
        })
        .collect()
}

fn rust_headers(request: &Request) -> Headers {
    request
        .headers
        .iter()
        .map(|(name, value)| {
            if name == "host" {
                (name.clone(), "<host>".to_owned())
            } else {
                (name.clone(), value.clone())
            }
        })
        .collect()
}

#[tokio::test]
async fn requests_carry_exactly_the_headers_java_sends() {
    let own = std::sync::Arc::new(std::sync::OnceLock::<String>::new());
    let base = own.clone();
    let server = Server::start(move |request, _| {
        let target = request.target.as_str();
        if target.starts_with("/chart/") {
            Reply::json(&format!(
                r#"{{"id":42,"file":"{}files/archive"}}"#,
                base.get().unwrap()
            ))
        } else if target.starts_with("/files/") {
            Reply::ranged(request, &[0u8; 64])
        } else if target.starts_with("/record?") {
            Reply::json("[]")
        } else if target.starts_with("/chart?") {
            Reply::json(r#"{"count":0,"results":[]}"#)
        } else {
            Reply::json(r#"{"id":1,"token":"t"}"#)
        }
    })
    .await;
    own.set(server.base_url()).unwrap();
    let phira = PhiraClient::with_options(
        server.base_url(),
        Options {
            probe_timeout: Duration::from_secs(10),
            ..Options::default()
        },
    )
    .unwrap();

    tokio::time::timeout(Duration::from_secs(30), async {
        phira.user_by_token("secret-token").await.unwrap();
        phira.user_by_id(77).await.unwrap();
        phira.chart(42).await.unwrap();
        phira.record(9).await.unwrap();
        phira.recent_records(7).await.unwrap();
        phira.login("player@example.com", "pw").await.unwrap();
        phira.chart_page(3, None).await.unwrap();
        phira.chart_page(4, Some("plain")).await.unwrap();
        let _ = phira.probe_duration(42).await;
    })
    .await
    .expect("calls finished");

    let java = java_rows();
    let rust = server.requests();
    assert_eq!(java.len(), 9);
    assert_eq!(
        rust.len(),
        9,
        "one request per call: {:?}",
        rust.iter().map(|r| &r.target).collect::<Vec<_>>()
    );
    for ((method, target, java_headers), request) in java.iter().zip(&rust) {
        assert_eq!(&request.method, method, "{target}");
        let java_target = if target.starts_with("/files/") {
            "/files/archive"
        } else {
            target
        };
        assert_eq!(request.target, java_target);
        assert_eq!(&rust_headers(request), java_headers, "{method} {target}");
    }

    let probe = rust.last().unwrap();
    assert_eq!(probe.header("accept"), None, "the probe sends no Accept");
    assert_eq!(probe.header("user-agent"), Some("JPhira/1"));
    assert_eq!(probe.header("range"), Some("bytes=0-32767"));
    let mut names: Vec<&str> = probe.headers.iter().map(|(n, _)| n.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["host", "range", "user-agent"]);
    assert_eq!(rust[0].header("authorization"), Some("Bearer secret-token"));
    assert_eq!(rust[5].header("content-type"), Some("application/json"));
}
