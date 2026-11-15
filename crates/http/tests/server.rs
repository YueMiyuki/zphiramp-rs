mod support;

use std::net::SocketAddr;
use std::time::Duration;

use support::Env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zphira_http::HttpServer;

async fn raw(addr: SocketAddr, request: &str) -> (String, Vec<(String, String)>, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let _ = stream.write_all(request.as_bytes()).await;
    let mut response = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset && !response.is_empty() => {
                break;
            }
            Err(e) => panic!("{e}"),
        }
    }
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no header end: {}", String::from_utf8_lossy(&response)));
    let head = String::from_utf8_lossy(&response[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().to_owned();
    let headers = lines
        .map(|l| {
            let (k, v) = l.split_once(':').unwrap();
            (k.trim().to_ascii_lowercase(), v.trim().to_owned())
        })
        .collect();
    (status, headers, response[split + 4..].to_vec())
}

fn get<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

async fn started(env: &Env) -> (HttpServer, SocketAddr) {
    let mut server = HttpServer::start(env.state.clone(), "127.0.0.1", 0);
    let addr = server.await_started().await.unwrap();
    (server, addr)
}

#[tokio::test(flavor = "current_thread")]
async fn bind_failure_reports_like_java() {
    let env = Env::new().await;
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let (captured, _guard) = support::replay::capture();
    let mut server = HttpServer::start(env.state.clone(), "127.0.0.1", port);
    let error = server.await_started().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "HTTP API server failed to bind 127.0.0.1:{port}; the game port is up but the web manager is unreachable. Free the port or pass a different --http-port."
        )
    );
    assert!(server.await_started().await.is_err());
    let logs: Vec<String> = captured
        .take()
        .into_iter()
        .filter(|l| l.contains("Server:"))
        .collect();
    assert_eq!(
        logs,
        vec![format!(
            "ERROR Server: HTTP API server failed to bind 127.0.0.1:{port}\n  Port already in use. Make sure no other process is using port {port} and try again."
        )]
    );
    server.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn start_logs_the_listening_line() {
    let env = Env::new().await;
    let (captured, _guard) = support::replay::capture();
    let (server, addr) = started(&env).await;
    assert_ne!(addr.port(), 0);
    let logs: Vec<String> = captured
        .take()
        .into_iter()
        .filter(|l| l.contains("Server:"))
        .collect();
    assert_eq!(
        logs,
        vec!["INFO Server: HTTP API server listening on 127.0.0.1:0".to_owned()]
    );
    server.stop().await;
}

#[tokio::test]
async fn wire_level_behaviour() {
    let env = Env::new().await;
    let (server, addr) = started(&env).await;
    let admin = env.admin.clone();

    let (status, headers, body) = raw(
        addr,
        "HEAD /api/v1/pool/list HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(status, "HTTP/1.1 401 Unauthorized");
    assert_eq!(get(&headers, "content-type"), Some("application/json"));
    assert_eq!(get(&headers, "content-length"), Some("36"));
    assert_eq!(get(&headers, "access-control-allow-origin"), Some("*"));
    assert!(body.is_empty());
    let (status, _, body) = raw(addr, &format!("HEAD /api/v1/pool/list HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n", env.player)).await;
    assert_eq!(status, "HTTP/1.1 403 Forbidden");
    assert!(body.is_empty());
    let (_, get_headers, get_body) = raw(addr, &format!("GET /api/v1/pool/list HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {admin}\r\nConnection: close\r\n\r\n")).await;
    let (status, headers, body) = raw(addr, &format!("HEAD /api/v1/pool/list HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {admin}\r\nConnection: close\r\n\r\n")).await;
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(get(&headers, "content-type"), Some("application/json"));
    assert_eq!(
        get(&headers, "content-length"),
        Some(get_body.len().to_string().as_str())
    );
    assert_eq!(
        get(&get_headers, "content-length"),
        get(&headers, "content-length")
    );
    assert!(body.is_empty());

    let (status, _, _) = raw(addr, &format!("GET /api/v1/pool/list HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {admin}   \r\nConnection: close\r\n\r\n")).await;
    assert_eq!(status, "HTTP/1.1 200 OK");

    for target in [
        "/api/v1/record/%zz",
        "/api/v1/record/abc%2",
        "/../secret.txt",
        "/room/../../secret.txt",
        "/%2e%2e/secret.txt",
    ] {
        let (status, headers, body) = raw(
            addr,
            &format!("GET {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"),
        )
        .await;
        assert_eq!(status, "HTTP/1.1 400 Bad Request", "{target}");
        assert_eq!(
            get(&headers, "content-type"),
            Some("text/html;charset=iso-8859-1"),
            "{target}"
        );
        assert_eq!(
            get(&headers, "cache-control"),
            Some("must-revalidate,no-cache,no-store")
        );
        assert_eq!(body.len(), 337, "{target}");
    }
    let (status, _, body) = raw(
        addr,
        "GET //etc/passwd HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(status, "HTTP/1.1 400 Bad Request");
    assert_eq!(body, b"Bad Request");
    let (status, headers, body) = raw(
        addr,
        "OPTIONS * HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(status, "HTTP/1.1 404 Not Found");
    assert_eq!(
        get(&headers, "cache-control"),
        Some("must-revalidate,no-cache,no-store")
    );
    assert!(body.is_empty());

    let (status, headers, body) = raw(addr, "OPTIONS /api/v1/pool/1 HTTP/1.1\r\nHost: x\r\nOrigin: http://localhost:3000\r\nAccess-Control-Request-Method: DELETE\r\nConnection: close\r\n\r\n").await;
    assert_eq!(status, "HTTP/1.1 204 No Content");
    assert_eq!(
        get(&headers, "access-control-allow-methods"),
        Some("GET, POST, PUT, DELETE, OPTIONS")
    );
    assert_eq!(
        get(&headers, "access-control-allow-headers"),
        Some("Content-Type, Authorization")
    );
    assert!(body.is_empty());

    let mut request = format!(
        "POST /api/v1/pool/generate HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {admin}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    );
    let chunk = " ".repeat(65536);
    for _ in 0..16 {
        request.push_str(&format!("{:x}\r\n{chunk}\r\n", chunk.len()));
    }
    request.push_str("0\r\n\r\n");
    let (status, _, body) = raw(addr, &request).await;
    assert_eq!(status, "HTTP/1.1 413 Payload Too Large");
    assert_eq!(body, b"Content Too Large");

    let (_, headers, _) = raw(addr, "GET /_next/static/chunks/app.js HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n").await;
    assert_eq!(get(&headers, "content-encoding"), Some("gzip"));
    assert_eq!(
        get(&headers, "content-type"),
        Some("application/javascript")
    );
    let (_, headers, _) = raw(
        addr,
        "GET /style.css HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(get(&headers, "content-encoding"), None);
    server.stop().await;
}

#[tokio::test]
async fn login_limit_uses_the_peer_address() {
    let env = Env::new().await;
    let (server, addr) = started(&env).await;
    let body = "{\"email\":\"user1@x\",\"password\":\"wrong\"}";
    let request = format!(
        "POST /api/v1/login HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Forwarded-For: 10.9.9.{{n}}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    for n in 0..30 {
        let (status, _, _) = raw(addr, &request.replace("{n}", &n.to_string())).await;
        assert_eq!(status, "HTTP/1.1 400 Bad Request", "attempt {}", n + 1);
    }
    let (status, _, body) = raw(addr, &request.replace("{n}", "99")).await;
    assert_eq!(status, "HTTP/1.1 429 Too Many Requests");
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "{\"ok\":false,\"reason\":\"登录尝试过于频繁，请稍后再试\"}"
    );
    server.stop().await;
}

#[tokio::test]
async fn graceful_stop_finishes_requests_in_flight() {
    let env = Env::new().await;
    env.phira.behaviour.lock().unwrap().user_delay = Some(Duration::from_millis(400));
    let (server, addr) = started(&env).await;
    let token = env.player.clone();
    let in_flight = tokio::spawn(async move {
        let body = "{\"chartIds\":[1004]}";
        raw(addr, &format!(
            "POST /api/v1/pool/2/submissions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    server.stop().await;
    let (status, _, body) = in_flight.await.unwrap();
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert!(String::from_utf8_lossy(&body).contains("\"accepted\":1"));
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "no longer accepting"
    );
}

#[tokio::test]
async fn slow_clients_are_dropped() {
    let config = zphira_http::HttpConfig {
        idle_timeout: Duration::from_millis(300),
        ..zphira_http::HttpConfig::default()
    };
    let env = Env::with_config(config).await;
    let (server, addr) = started(&env).await;

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /api/v1/pool/list HTTP/1.1\r\nHost: x\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf)).await;
    assert!(read.is_ok(), "the connection was not closed");

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST /api/v1/pool/generate HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {}\r\nContent-Length: 100\r\n\r\n{{\"category\"",
        env.admin
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf)).await;
    assert!(read.is_ok(), "the connection was not closed");
    let text = String::from_utf8_lossy(&buf);
    assert!(
        text.is_empty() || text.starts_with("HTTP/1.1 500"),
        "{text}"
    );

    let (status, _, _) = raw(
        addr,
        "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(status, "HTTP/1.1 200 OK");
    server.stop().await;
}

#[tokio::test]
async fn closed_reports_how_the_server_ended() {
    let env = Env::new().await;
    let (mut server, addr) = started(&env).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), server.closed())
            .await
            .is_err(),
        "a running server is not closed"
    );
    let (status, _, _) = raw(
        addr,
        "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(status, "HTTP/1.1 200 OK");
    server.stop().await;

    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let mut server = HttpServer::start(env.state.clone(), "127.0.0.1", port);
    assert_eq!(server.closed().await, zphira_http::ServerExit::BindFailed);
    assert_eq!(server.closed().await, zphira_http::ServerExit::BindFailed);
    assert!(server.await_started().await.is_err());
    server.stop().await;
}
