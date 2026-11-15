mod common;

use bytes::{Bytes, BytesMut};
use common::*;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::codec::{Decoder, Encoder, Framed, FramedParts, FramedRead};
use zphira_protocol::handshake::decode_handshake;
use zphira_protocol::*;

struct Outcome {
    events: Vec<String>,
    closed: bool,
    packets: Vec<Vec<u8>>,
    eof_events: usize,
}

impl Outcome {
    fn render(&self) -> String {
        format!(
            "{};{}",
            self.events.join(";"),
            if self.closed { "CLOSED" } else { "OPEN" }
        )
    }

    fn summary(&self) -> String {
        let all: Vec<u8> = self.packets.concat();
        format!(
            "{}.{}.{}.{}",
            self.packets.len(),
            if self.closed { "C" } else { "O" },
            crc32(&all),
            self.eof_events
        )
    }
}

enum Side {
    Server,
    Client,
}

fn decode_one(
    side: &Side,
    server: &mut ServerCodec,
    client: &mut ClientCodec,
    buf: &mut BytesMut,
) -> Result<Option<Vec<u8>>, CodecError> {
    match side {
        Side::Server => Ok(server.decode(buf)?.map(|p| p.to_bytes().to_vec())),
        Side::Client => Ok(client.decode(buf)?.map(|p| p.to_bytes().to_vec())),
    }
}

fn run(side: Side, handshake: bool, chunks: &[Vec<u8>]) -> Outcome {
    let mut out = Outcome {
        events: vec![],
        closed: false,
        packets: vec![],
        eof_events: 0,
    };

    // Recorded Java vectors include 2 GB length prefixes that must only wait: replay uncapped.
    let (mut server, mut client) = (
        ServerCodec::with_max_frame_length(NO_CAP),
        ClientCodec::with_max_frame_length(NO_CAP),
    );
    let mut buf = BytesMut::new();
    let mut awaiting_version = handshake;
    for chunk in chunks {
        if out.closed {
            break;
        }
        buf.extend_from_slice(chunk);
        if awaiting_version {
            match decode_handshake(&mut buf) {
                Ok(None) => continue,
                Ok(Some(v)) => {
                    out.events.push(format!("V:{v}"));
                    awaiting_version = false;
                }
                Err(e) => {
                    out.events.push(format!("H:{e}"));
                    out.closed = true;
                    continue;
                }
            }
        }
        loop {
            match decode_one(&side, &mut server, &mut client, &mut buf) {
                Ok(Some(bytes)) => {
                    out.events.push(format!("P:{}", hex(&bytes)));
                    out.packets.push(bytes);
                }
                Ok(None) => break,
                Err(CodecError::Protocol(_)) => {
                    out.closed = true;
                    break;
                }
                Err(other) => panic!("unexpected {other}"),
            }
        }
    }
    if !out.closed {
        if awaiting_version {
            let e = HandshakeError::ConnectionClosed;
            out.events.push(format!("EOF:H:{e}"));
            out.eof_events += 1;
        } else {
            loop {
                match decode_eof_one(&side, &mut server, &mut client, &mut buf) {
                    Ok(Some(bytes)) => {
                        out.events.push(format!("EOF:P:{}", hex(&bytes)));
                        out.eof_events += 1;
                    }
                    Ok(None) => break,
                    Err(e) => panic!("unexpected error at EOF: {e}"),
                }
            }
            assert!(buf.is_empty(), "leftover bytes must be dropped at EOF");
        }
    }
    out
}

fn decode_eof_one(
    side: &Side,
    server: &mut ServerCodec,
    client: &mut ClientCodec,
    buf: &mut BytesMut,
) -> Result<Option<Vec<u8>>, CodecError> {
    match side {
        Side::Server => Ok(server.decode_eof(buf)?.map(|p| p.to_bytes().to_vec())),
        Side::Client => Ok(client.decode_eof(buf)?.map(|p| p.to_bytes().to_vec())),
    }
}

fn gen_chunks(spec: &str) -> Vec<Vec<u8>> {
    match spec {
        "gen:big_frame" => {
            let mut v = Vec::new();
            encode_frame(&vec![0u8; 300_000], &mut v).unwrap();
            vec![v]
        }
        "gen:big_frame_chunked" => {
            let mut v = Vec::new();
            encode_frame(&vec![0u8; 300_000], &mut v).unwrap();
            v.chunks(4096).map(<[u8]>::to_vec).collect()
        }
        hexes => vec![unhex(hexes)],
    }
}

#[test]
fn crafted_streams() {
    let rows = rows("streams.tsv");
    let mut n = 0;
    for row in &rows {
        let (name, side, mode) = (&row[0], &row[1], &row[2]);
        let want = &row[4];
        let side = if side == "server" {
            Side::Server
        } else {
            Side::Client
        };
        let got = match mode.as_str() {
            "timeout" => format!("H:{};CLOSED", HandshakeError::Timeout),
            "eof" => format!("H:{};CLOSED", HandshakeError::ConnectionClosed),
            mode => {
                let chunks: Vec<Vec<u8>> = if row[3].is_empty() {
                    vec![]
                } else {
                    row[3].split('|').flat_map(gen_chunks_one).collect()
                };
                run(side, mode == "1", &chunks).render()
            }
        };
        assert_eq!(&got, want, "stream {name}");
        n += 1;
    }
    eprintln!("{n} crafted streams");
}

fn gen_chunks_one(item: &str) -> Vec<Vec<u8>> {
    gen_chunks(item)
}

fn stream_fuzz(name: &str, seed: u64, framed_prefix: &str, side: fn() -> Side) {
    let frames: Vec<Vec<u8>> = rows("packets.tsv")
        .into_iter()
        .filter(|r| r[0].starts_with(framed_prefix))
        .map(|r| unhex(&r[3]))
        .collect();
    let expected = lines(name);
    assert_eq!(expected.len(), 1500);
    let mut r = Rng(seed);
    let mut closed = 0;
    for (i, want) in expected.iter().enumerate() {
        let k = 1 + r.below(4);
        let mut stream = Vec::new();
        for _ in 0..k {
            stream.extend_from_slice(&frames[r.below(frames.len())]);
        }
        let ops = r.below(4);
        let stream = mutate(&mut r, &stream, ops);
        let mut chunks = Vec::new();
        let mut pos = 0;
        while pos < stream.len() {
            let remaining = stream.len() - pos;
            let m = 1 + r.below(64);
            let n = 1 + r.below(remaining.min(m));
            chunks.push(stream[pos..pos + n].to_vec());
            pos += n;
        }
        let got = run(side(), false, &chunks);
        let got_summary = got.summary();
        assert_eq!(
            &got_summary,
            want,
            "{name} case {i}: stream {} in chunks {:?}",
            hex(&stream),
            chunks.iter().map(|c| c.len()).collect::<Vec<_>>()
        );
        closed += usize::from(got.closed);
    }
    eprintln!("{name}: 1500 streams, {closed} closed");
}

#[test]
fn stream_fuzz_server() {
    stream_fuzz("stream_fuzz_sb.txt", 0x057A_EA01, "sb.", || Side::Server);
}

#[test]
fn stream_fuzz_client() {
    stream_fuzz("stream_fuzz_cb.txt", 0x057A_EA02, "cb.", || Side::Client);
}

#[tokio::test]
async fn framed_roundtrip_over_a_socket() {
    let (a, b) = tokio::io::duplex(64 * 1024);
    let mut server = Framed::new(a, ServerCodec::new());
    let mut client = Framed::new(b, ClientCodec::new());

    client
        .send(ServerBoundPacket::Chat {
            message: "hi 你好".into(),
        })
        .await
        .unwrap();
    client.send(ServerBoundPacket::Ping).await.unwrap();
    assert_eq!(
        server.next().await.unwrap().unwrap(),
        ServerBoundPacket::Chat {
            message: "hi 你好".into()
        }
    );
    assert_eq!(
        server.next().await.unwrap().unwrap(),
        ServerBoundPacket::Ping
    );

    let reply = ClientBoundPacket::Chat(PacketResult::failed("没有权限"));
    server.send(&reply).await.unwrap();
    server.send(ClientBoundPacket::Pong).await.unwrap();
    assert_eq!(client.next().await.unwrap().unwrap(), reply);
    assert_eq!(
        client.next().await.unwrap().unwrap(),
        ClientBoundPacket::Pong
    );

    drop(client);
    assert!(server.next().await.is_none());
}

#[tokio::test]
async fn bad_frame_yields_earlier_packets_then_the_error_then_ends() {
    let bytes: &[u8] = &[
        0x01, 0x00, 0x04, 0x02, 0x02, b'h', b'i', 0x01, 0x20, 0x01, 0x00,
    ];
    let mut framed = FramedRead::new(bytes, ServerCodec::new());
    assert_eq!(
        framed.next().await.unwrap().unwrap(),
        ServerBoundPacket::Ping
    );
    assert_eq!(
        framed.next().await.unwrap().unwrap(),
        ServerBoundPacket::Chat {
            message: "hi".into()
        }
    );
    match framed.next().await.unwrap() {
        Err(CodecError::Protocol(e)) => {
            assert_eq!(
                e,
                ProtocolError::UnknownId {
                    kind: IdKind::ServerBoundPacket,
                    id: 0x20
                }
            );
            assert_eq!(e.pipeline_action(), PipelineAction::Close);
            assert_eq!(e.to_string(), "unknown serverbound packet id 32");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        framed.next().await.is_none(),
        "the stream ends after an error"
    );
}

#[tokio::test]
async fn incomplete_frame_at_eof_is_dropped_silently() {
    let bytes: &[u8] = &[0x01, 0x00, 0x05, 0x02, 0x03];
    let mut framed = FramedRead::new(bytes, ServerCodec::new());
    assert_eq!(
        framed.next().await.unwrap().unwrap(),
        ServerBoundPacket::Ping
    );
    assert!(framed.next().await.is_none());

    let mut framed = FramedRead::new(&[0u8, 0, 0][..], ServerCodec::new());
    assert!(framed.next().await.is_none());
}

#[test]
fn frame_functions_on_plain_buffers() {
    let mut buf = BytesMut::new();
    encode_frame(b"abc", &mut buf).unwrap();
    encode_frame(b"", &mut buf).unwrap();
    encode_frame(&[7; 200], &mut buf).unwrap();
    assert_eq!(&buf[..5], &[3, b'a', b'b', b'c', 0]);

    assert_eq!(
        decode_frame(&mut buf, NO_CAP).unwrap(),
        Some(Bytes::from_static(b"abc"))
    );
    assert_eq!(decode_frame(&mut buf, NO_CAP).unwrap().unwrap().len(), 200);
    assert!(buf.is_empty());

    let mut buf = BytesMut::from(&[0u8, 0, 4, 1][..]);
    assert_eq!(decode_frame(&mut buf, NO_CAP).unwrap(), None);
    assert_eq!(&buf[..], &[4, 1]);

    let mut buf = BytesMut::from(&[0xff, 0xff, 0xff, 0xff, 0xff][..]);
    assert_eq!(
        decode_frame(&mut buf, NO_CAP),
        Err(ProtocolError::Malformed("bad varint"))
    );
    let mut buf = BytesMut::from(&[0xff, 0xff, 0xff, 0xff, 0x0f][..]);
    assert_eq!(
        decode_frame(&mut buf, NO_CAP),
        Err(ProtocolError::Malformed("negative frame length"))
    );
}

#[test]
fn handshake_functions() {
    assert_eq!(decode_handshake(&mut &[][..]), Ok(None));
    assert_eq!(decode_handshake(&mut &[1u8, 9][..]), Ok(Some(1)));
    assert_eq!(
        decode_handshake(&mut &[0xffu8][..]),
        Err(HandshakeError::UnsupportedVersion(-1))
    );
    assert_eq!(
        decode_handshake(&mut &[2u8][..]).unwrap_err().to_string(),
        "Unsupported version: 2"
    );
    assert_eq!(
        HandshakeEncoder::new(2),
        Err(HandshakeError::UnsupportedVersion(2))
    );
    assert_eq!(
        HandshakeEncoder::new(-1),
        Err(HandshakeError::UnsupportedVersion(-1))
    );
    let mut out = BytesMut::new();
    HandshakeEncoder::new(1).unwrap().encode(&mut out);
    assert_eq!(&out[..], &[1]);
    let mut out = BytesMut::new();
    Encoder::<()>::encode(&mut HandshakeEncoder::default(), (), &mut out).unwrap();
    assert_eq!(&out[..], &[1]);
    assert_eq!(HANDSHAKE_TIMEOUT.as_millis(), 5000);

    let mut d = HandshakeDecoder;
    let mut buf = BytesMut::from(&[1u8, 0, 0][..]);
    assert_eq!(d.decode(&mut buf).unwrap(), Some(1));
    assert_eq!(&buf[..], &[0, 0]);
    let mut buf = BytesMut::new();
    assert!(matches!(
        d.decode_eof(&mut buf),
        Err(CodecError::Handshake(HandshakeError::ConnectionClosed))
    ));
}

#[test]
fn simple_handler_default() {
    assert_eq!(
        ServerBoundPacket::Ping.default_response(),
        Some(ClientBoundPacket::Pong)
    );
    for p in [
        ServerBoundPacket::LeaveRoom,
        ServerBoundPacket::Ready,
        ServerBoundPacket::Chat {
            message: String::new(),
        },
        ServerBoundPacket::Authenticate {
            token: String::new(),
        },
    ] {
        assert_eq!(p.default_response(), None, "{}", p.name());
    }
}

#[tokio::test]
async fn handshake_then_packets_through_framed_parts() {
    let (a, mut b) = tokio::io::duplex(1024);

    b.write_all(&[1, 1, 0, 3, 2, 1, b'x']).await.unwrap();

    let mut handshake = Framed::new(a, HandshakeDecoder);
    assert_eq!(handshake.next().await.unwrap().unwrap(), 1);
    let parts = handshake.into_parts();
    let mut next = FramedParts::new::<ClientBoundPacket>(parts.io, ServerCodec::new());
    next.read_buf = parts.read_buf;
    let mut server = Framed::from_parts(next);

    assert_eq!(
        server.next().await.unwrap().unwrap(),
        ServerBoundPacket::Ping
    );
    assert_eq!(
        server.next().await.unwrap().unwrap(),
        ServerBoundPacket::Chat {
            message: "x".into()
        }
    );
}

#[tokio::test]
async fn handshake_byte_read_off_the_raw_socket() {
    let (mut a, mut b) = tokio::io::duplex(1024);
    b.write_all(&[1, 1, 0]).await.unwrap();
    let version = a.read_u8().await.unwrap();
    assert_eq!(decode_handshake(&mut &[version][..]), Ok(Some(1)));
    let mut server = Framed::new(a, ServerCodec::new());
    assert_eq!(
        server.next().await.unwrap().unwrap(),
        ServerBoundPacket::Ping
    );
}

#[tokio::test]
async fn handshake_failures_through_framed() {
    let mut bad = Framed::new(&[2u8, 1, 0][..], HandshakeDecoder);
    match bad.next().await.unwrap() {
        Err(CodecError::Handshake(e)) => assert_eq!(e, HandshakeError::UnsupportedVersion(2)),
        other => panic!("{other:?}"),
    }
    let mut closed = Framed::new(&[][..], HandshakeDecoder);
    match closed.next().await.unwrap() {
        Err(CodecError::Handshake(e)) => assert_eq!(e, HandshakeError::ConnectionClosed),
        other => panic!("{other:?}"),
    }
}
