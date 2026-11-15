mod support;

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::StreamExt;
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio_util::codec::Framed;
use zphira_game::net::proxy;
use zphira_protocol::{ClientCodec, ServerBoundPacket as Sb};

async fn sleep(secs: f64) {
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
}

async fn is_closed(stream: &mut DuplexStream) -> bool {
    let mut buf = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_millis(1), stream.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => return true,
            Ok(Ok(_)) => continue,
            Err(_) => return false,
        }
    }
}

fn proxied(setup: &mut Setup) {
    setup.args.proxy_protocol = true;
}

#[tokio::test(start_paused = true)]
async fn connection_life_cycle_log_lines() {
    let w = World::new().await;
    let peer: SocketAddr = "127.0.0.1:5555".parse().unwrap();
    let c = w.connect_from(peer).await;
    settle().await;
    c.shutdown().await;
    settle().await;
    assert_eq!(
        w.logs.lines(),
        s(&[
            "INFO Establishing a connection from 127.0.0.1:5555",
            "INFO Receive client version 1 from 127.0.0.1:5555",
            "INFO Client disconnected: 127.0.0.1:5555 (ACTIVE)",
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn handshake_failures() {
    let w = World::new().await;
    let peer = w.peer();
    let mut s1 = w.raw(peer, 1024);
    s1.write_all(&[2]).await.unwrap();
    assert!(
        is_closed(&mut s1).await || {
            settle().await;
            is_closed(&mut s1).await
        }
    );
    assert!(w.logs.contains(&format!(
        "WARN Disconnecting 127.0.0.1:{} on Phira handshaking: Unsupported version: 2",
        peer.port()
    )));
    let peer = w.peer();
    let mut s2 = w.raw(peer, 1024);
    sleep(4.9).await;
    assert!(!is_closed(&mut s2).await);
    sleep(0.2).await;
    assert!(is_closed(&mut s2).await);
    assert!(w.logs.contains(&format!(
        "WARN Disconnecting 127.0.0.1:{} on Phira handshaking: Handshake timeout",
        peer.port()
    )));
    let peer = w.peer();
    let s3 = w.raw(peer, 1024);
    drop(s3);
    settle().await;
    assert!(w.logs.contains(&format!(
        "WARN Disconnecting 127.0.0.1:{} on Phira handshaking: Connection closed before handshake",
        peer.port()
    )));
    assert!(!w.logs.contains("Client disconnected"));
}

#[tokio::test(start_paused = true)]
async fn g07_pre_auth_deadline_of_twenty_seconds() {
    let w = World::new().await;
    let mut c = w.connect().await;
    for _ in 0..19 {
        sleep(1.0).await;
        c.send(Sb::Ping).await;
    }
    assert_eq!(c.drain().await.len(), 19);
    sleep(0.95).await;
    assert!(!c.closed_now().await);
    sleep(0.1).await;
    assert!(c.closed().await);
    settle().await;
    let port = c.peer.port();
    assert!(
        w.logs
            .contains(&format!("ERROR 127.0.0.1:{port}: read timed out"))
    );
    assert!(w.logs.contains(&format!(
        "INFO Client disconnected: 127.0.0.1:{port} (TIMEOUT)"
    )));
}

#[tokio::test(start_paused = true)]
async fn g07_upstream_check_is_not_counted_and_login_lifts_the_deadline() {
    let w = World::new().await;
    w.upstream.user("tok", 1, "A");
    let gate = std::sync::Arc::new(Gate::default());
    *w.upstream.token_gate.lock().unwrap() = Some(gate.clone());
    let mut c = w.connect().await;
    sleep(15.0).await;
    c.send(Sb::Authenticate {
        token: "tok".into(),
    })
    .await;
    gate.wait_reached().await;
    sleep(30.0).await;
    gate.open();
    let lines = c.lines().await;
    assert!(lines[0].starts_with("Authenticate ok"), "{lines:?}");
}

#[tokio::test(start_paused = true)]
async fn g07_idle_timeout_after_login_is_read_timeout_with_a_floor_of_600() {
    let w = World::new().await;
    let mut c = w.login(1, "A").await;
    sleep(3599.0).await;
    assert!(!c.closed_now().await);
    sleep(1.1).await;
    assert!(c.closed().await);
    settle().await;
    assert!(w.logs.contains("(TIMEOUT)"));

    let mut setup = Setup::default();
    setup.args.read_timeout_seconds = 100;
    setup.options.pre_auth_deadline = Duration::from_secs(1000);
    let w = World::with(setup).await;
    let mut idle = w.connect().await;
    sleep(99.0).await;
    assert!(!idle.closed_now().await);
    sleep(1.1).await;
    assert!(idle.closed().await);
    let mut c = w.login(1, "A").await;
    sleep(599.0).await;
    assert!(!c.closed_now().await);
    c.send(Sb::Ping).await;
    sleep(599.0).await;
    assert!(!c.closed_now().await);
    sleep(1.1).await;
    assert!(c.closed().await);
}

#[tokio::test(start_paused = true)]
async fn proxy_v1_and_v2_give_the_real_address() {
    let mut setup = Setup::default();
    proxied(&mut setup);
    let w = World::with(setup).await;
    w.upstream.user("tok", 1, "A");

    let mut s1 = w.raw(w.peer(), 64 * 1024);
    let mut bytes = b"PROXY TCP4 203.0.113.9 10.0.0.1 51000 12346\r\n".to_vec();
    bytes.push(1);
    s1.write_all(&bytes).await.unwrap();
    let mut c1 = Client::new(s1, w.peer());
    c1.send(Sb::Authenticate {
        token: "tok".into(),
    })
    .await;
    assert!(c1.lines().await[0].starts_with("Authenticate ok"));
    assert!(
        w.logs
            .contains("INFO Establishing a connection from 203.0.113.9:51000")
    );
    assert!(
        w.logs
            .contains("INFO 203.0.113.9:51000 has logged in as [1] A")
    );

    let src: SocketAddr = "[2001:db8::7]:4000".parse().unwrap();
    let dst: SocketAddr = "[::1]:12346".parse().unwrap();
    let mut s2 = w.raw(w.peer(), 64 * 1024);
    let mut header = proxy::encode_v2(src, dst);
    header.push(1);
    s2.write_all(&header[..10]).await.unwrap();
    settle().await;
    s2.write_all(&header[10..]).await.unwrap();
    settle().await;
    assert!(
        w.logs
            .contains("INFO Receive client version 1 from [2001:db8::7]:4000")
    );
    drop(s2);
    drop(c1);
}

#[tokio::test(start_paused = true)]
async fn proxy_header_failures() {
    let mut setup = Setup::default();
    proxied(&mut setup);
    let w = World::with(setup).await;
    let peer: SocketAddr = "127.0.0.1:6001".parse().unwrap();
    let mut s1 = w.raw(peer, 1024);
    s1.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    settle().await;
    assert!(is_closed(&mut s1).await);
    assert!(w.logs.contains(
        "WARN Disconnecting 127.0.0.1:6001 on HAProxy handshaking: unknown identifier: GET"
    ));

    let peer: SocketAddr = "127.0.0.1:6002".parse().unwrap();
    let mut s2 = w.raw(peer, 1024);
    s2.write_all(b"PROXY UNKNOWN\r\n").await.unwrap();
    settle().await;
    assert!(is_closed(&mut s2).await);
    assert!(w.logs.contains(
        "WARN Disconnecting 127.0.0.1:6002 on HAProxy handshaking: HAProxyMessage missing source address"
    ));

    let peer: SocketAddr = "127.0.0.1:6003".parse().unwrap();
    let mut s3 = w.raw(peer, 1024);
    s3.write_all(&[1]).await.unwrap();
    sleep(4.9).await;
    assert!(!is_closed(&mut s3).await);
    sleep(0.2).await;
    assert!(is_closed(&mut s3).await);
    assert!(w.logs.contains(
        "WARN Disconnecting 127.0.0.1:6003 on HAProxy handshaking: Proxy protocol handshake timeout"
    ));
}

#[tokio::test(start_paused = true)]
async fn g07_proxy_header_then_silence_is_closed_by_the_deadline() {
    let mut setup = Setup::default();
    proxied(&mut setup);
    let w = World::with(setup).await;
    let mut s1 = w.raw(w.peer(), 1024);
    s1.write_all(b"PROXY TCP4 198.51.100.1 10.0.0.1 7000 12346\r\n")
        .await
        .unwrap();
    sleep(19.9).await;
    assert!(!is_closed(&mut s1).await);
    sleep(0.2).await;
    assert!(is_closed(&mut s1).await);
    assert!(
        w.logs.contains(
            "WARN Disconnecting 198.51.100.1:7000 on Phira handshaking: Handshake timeout"
        )
    );
}

#[tokio::test(start_paused = true)]
async fn connection_limits_total_and_per_address() {
    let mut setup = Setup::default();
    setup.args.max_connections = 2;
    let w = World::with(setup).await;
    let c1 = w.connect().await;
    let _c2 = w.connect().await;
    settle().await;
    let peer = w.peer();
    let mut s3 = w.raw(peer, 1024);
    settle().await;
    assert!(is_closed(&mut s3).await);
    assert!(w.logs.contains(&format!(
        "WARN Refusing connection from 127.0.0.1:{}: connection limit reached (2)",
        peer.port()
    )));
    c1.shutdown().await;
    settle().await;
    let mut c4 = w.connect().await;
    c4.send(Sb::Ping).await;
    assert_eq!(c4.lines().await, s(&["Pong"]));

    let w = World::new().await;
    let mut clients = Vec::new();
    for i in 0..64 {
        clients.push(
            w.connect_from(format!("10.9.9.9:{}", 1000 + i).parse().unwrap())
                .await,
        );
    }
    settle().await;
    let mut extra = w.raw("10.9.9.9:2000".parse().unwrap(), 1024);
    settle().await;
    assert!(is_closed(&mut extra).await);
    assert!(w.logs.contains(
        "WARN Refusing connection from 10.9.9.9:2000: too many connections from 10.9.9.9"
    ));
    let mut other = w.connect_from("10.9.9.8:1".parse().unwrap()).await;
    other.send(Sb::Ping).await;
    assert_eq!(other.lines().await, s(&["Pong"]));
}

#[tokio::test(start_paused = true)]
async fn rate_guard_closes_above_200_packets_per_second() {
    let w = World::new().await;
    let mut c = w.login(1, "A").await;
    for _ in 0..199 {
        c.send(Sb::Ping).await;
    }
    assert_eq!(c.drain().await.len(), 199);
    sleep(1.0).await;
    for _ in 0..200 {
        c.send(Sb::Ping).await;
    }
    assert_eq!(c.drain().await.len(), 200);
    c.send(Sb::Ping).await;
    assert!(c.closed().await);
    settle().await;
    let port = c.peer.port();
    assert!(w.logs.contains(&format!(
        "WARN Closing 127.0.0.1:{port}, packet rate exceeded 200 per second"
    )));
    assert!(
        w.logs
            .contains(&format!("Client disconnected: 127.0.0.1:{port} (ACTIVE)"))
    );
}

fn padded_ping(len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut n = len as u32;
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
    out.push(0x00);
    out.resize(out.len() + len - 1, 0xAA);
    out
}

#[tokio::test(start_paused = true)]
async fn g01_pre_auth_frame_cap() {
    let w = World::new().await;
    let mut c = w.connect().await;
    c.framed
        .get_mut()
        .write_all(&padded_ping(1024))
        .await
        .unwrap();
    assert_eq!(c.lines().await, s(&["Pong"]));
    c.framed
        .get_mut()
        .write_all(&padded_ping(1025))
        .await
        .unwrap();
    assert!(c.closed().await);
    settle().await;
    let port = c.peer.port();
    assert!(w.logs.contains(&format!(
        "WARN Closing 127.0.0.1:{port}, frame of 1025 bytes exceeds the limit of 1024"
    )));

    let mut c = w.login(1, "A").await;
    c.framed
        .get_mut()
        .write_all(&padded_ping(100_000))
        .await
        .unwrap();
    assert_eq!(c.lines().await, s(&["Pong"]));
}

#[tokio::test(start_paused = true)]
async fn g01_list_count_from_the_wire_is_not_trusted() {
    let w = World::new().await;
    let mut c = w.connect().await;
    c.framed
        .get_mut()
        .write_all(&[0x05, 0x03, 0x80, 0x84, 0xAF, 0x5F])
        .await
        .unwrap();
    assert!(c.closed().await);
    settle().await;
    let port = c.peer.port();
    assert!(w.logs.contains(&format!(
        "INFO Client disconnected: 127.0.0.1:{port} (ACTIVE)"
    )));
    assert!(
        !w.logs
            .lines()
            .iter()
            .any(|l| l.starts_with("ERROR") || l.starts_with("WARN"))
    );
}

#[tokio::test(start_paused = true)]
async fn malformed_frames_close_silently() {
    let w = World::new().await;
    let mut c = w.login(1, "A").await;
    c.framed.get_mut().write_all(&[0x01, 0x7f]).await.unwrap();
    assert!(c.closed().await);
    settle().await;
    let port = c.peer.port();
    assert!(w.logs.contains(&format!(
        "INFO Client disconnected: 127.0.0.1:{port} (ACTIVE)"
    )));
    let c = w.login(1, "A").await;
    drop(c);
}

#[tokio::test(start_paused = true)]
async fn outbound_backpressure_drops_and_recovers() {
    let w = World::new().await;
    w.upstream.user("tok", 1, "A");
    let peer = w.peer();
    let mut stream = w.raw(peer, 64);
    stream.write_all(&[1]).await.unwrap();
    let mut framed = Framed::new(stream, ClientCodec::new());
    use futures_util::SinkExt;
    framed
        .send(Sb::Authenticate {
            token: "tok".into(),
        })
        .await
        .unwrap();
    settle().await;
    let big = "x".repeat(100_000);
    for _ in 0..30 {
        w.ctx.say_to_all(&big);
    }
    settle().await;
    let port = peer.port();
    let dropped = w.logs.count(&format!(
        "WARN Dropping outbound packet to 127.0.0.1:{port}, buffer is backed up"
    ));
    assert!((5..=10).contains(&dropped), "{dropped}");
    let mut received = 0;
    while let Ok(Some(Ok(_))) = tokio::time::timeout(Duration::from_millis(5), framed.next()).await
    {
        received += 1;
    }
    assert_eq!(
        received,
        7 + 30 - dropped,
        "login packets + delivered lines"
    );
    w.ctx.say_to_all("again");
    let next = tokio::time::timeout(Duration::from_millis(5), framed.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(render(&next), "> again");
}

async fn served_tcp(w: &World) -> tokio::net::TcpStream {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, peer) = listener.accept().await.unwrap();
    let ctx = w.ctx.clone();
    tokio::spawn(async move {
        // The client's bytes are queued in the socket before the server reads anything.
        tokio::time::sleep(Duration::from_millis(50)).await;
        ctx.serve_connection(server, peer).await;
    });
    client
}

async fn end_of_stream(client: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    let mut buf = vec![0u8; 4096];
    tokio::time::timeout(MUST_ARRIVE, async {
        loop {
            if client.read(&mut buf).await? == 0 {
                return Ok(());
            }
        }
    })
    .await
    .expect("the server closes the connection")
}

#[tokio::test]
async fn a_refused_connection_ends_with_end_of_stream_not_a_reset() {
    let mut setup = Setup::default();
    setup.args.max_connections = 1;
    let w = World::with(setup).await;
    let mut first = w.connect().await;
    first.send(Sb::Ping).await;
    assert_eq!(first.expect_next().await, "Pong");
    let mut refused = served_tcp(&w).await;
    refused.write_all(&[1; 4096]).await.unwrap();
    end_of_stream(&mut refused).await.unwrap();
    assert!(w.logs.contains("connection limit reached (1)"));
}

#[tokio::test]
async fn a_failed_handshake_ends_with_end_of_stream_not_a_reset() {
    let w = World::new().await;
    let mut client = served_tcp(&w).await;
    client.write_all(&[2; 4096]).await.unwrap();
    end_of_stream(&mut client).await.unwrap();
    assert!(
        w.logs
            .contains("on Phira handshaking: Unsupported version: 2")
    );
}

#[tokio::test]
async fn closing_an_open_connection_ends_with_end_of_stream_not_a_reset() {
    let w = World::new().await;
    let mut client = served_tcp(&w).await;
    let mut bytes = vec![1];
    bytes.extend_from_slice(&padded_ping(1025));
    bytes.resize(bytes.len() + 32 * 1024, 0xAA);
    client.write_all(&bytes).await.unwrap();
    end_of_stream(&mut client).await.unwrap();
    assert!(
        w.logs
            .contains("frame of 1025 bytes exceeds the limit of 1024")
    );
}
