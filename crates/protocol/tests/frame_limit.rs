mod common;

use bytes::{Bytes, BytesMut};
use common::NO_CAP;
use futures_util::StreamExt;
use tokio_util::codec::{Decoder, Encoder, FramedRead};
use zphira_protocol::*;

fn prefixed(len: usize, body_len: usize) -> BytesMut {
    let mut b = BytesMut::new();
    zphira_protocol::wire::encode_varint(&mut b, len as i32);
    b.extend(std::iter::repeat_n(0u8, body_len));
    b
}

#[test]
fn constants() {
    assert_eq!(DEFAULT_MAX_FRAME_LENGTH, 2 * 1024 * 1024);
    assert_eq!(
        ServerCodec::new().max_frame_length(),
        DEFAULT_MAX_FRAME_LENGTH
    );
    assert_eq!(
        ClientCodec::new().max_frame_length(),
        DEFAULT_MAX_FRAME_LENGTH
    );
    assert_eq!(
        ServerCodec::default().max_frame_length(),
        DEFAULT_MAX_FRAME_LENGTH
    );
    assert_eq!(
        FrameDecoder::new().max_frame_length(),
        DEFAULT_MAX_FRAME_LENGTH
    );
    assert_eq!(FrameDecoder::with_max_frame_length(7).max_frame_length(), 7);
}

#[test]
fn frame_exactly_at_the_limit_decodes() {
    let mut buf = prefixed(100, 100);
    let got = decode_frame(&mut buf, 100).unwrap().unwrap();
    assert_eq!(got.len(), 100);
    assert!(buf.is_empty());

    let mut buf = prefixed(DEFAULT_MAX_FRAME_LENGTH, DEFAULT_MAX_FRAME_LENGTH);
    assert_eq!(
        ServerCodec::new().decode(&mut buf).unwrap(),
        Some(ServerBoundPacket::Ping)
    );
    let mut buf = prefixed(DEFAULT_MAX_FRAME_LENGTH, DEFAULT_MAX_FRAME_LENGTH);
    assert_eq!(
        ClientCodec::new().decode(&mut buf).unwrap(),
        Some(ClientBoundPacket::Pong)
    );

    let mut buf = prefixed(DEFAULT_MAX_FRAME_LENGTH, DEFAULT_MAX_FRAME_LENGTH);
    let got = FrameDecoder::new().decode(&mut buf).unwrap().unwrap();
    assert_eq!(got.len(), DEFAULT_MAX_FRAME_LENGTH);
}

#[test]
fn one_byte_over_fails_without_waiting_for_the_body() {
    let mut buf = prefixed(101, 0);
    assert_eq!(
        decode_frame(&mut buf, 100),
        Err(ProtocolError::FrameTooLarge {
            length: 101,
            max: 100
        })
    );

    let mut buf = prefixed(DEFAULT_MAX_FRAME_LENGTH + 1, 0);
    let err = match ServerCodec::new().decode(&mut buf) {
        Err(CodecError::Protocol(e)) => e,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        err,
        ProtocolError::FrameTooLarge {
            length: DEFAULT_MAX_FRAME_LENGTH + 1,
            max: DEFAULT_MAX_FRAME_LENGTH
        }
    );
    assert_eq!(err.pipeline_action(), PipelineAction::WarnAndClose);
    assert_eq!(
        err.to_string(),
        "frame of 2097153 bytes exceeds the limit of 2097152"
    );
    assert!(
        ClientCodec::new()
            .decode(&mut prefixed(DEFAULT_MAX_FRAME_LENGTH + 1, 0))
            .is_err()
    );
    assert!(
        FrameDecoder::new()
            .decode(&mut prefixed(DEFAULT_MAX_FRAME_LENGTH + 1, 0))
            .is_err()
    );

    let mut buf = prefixed(11, 11);
    assert!(matches!(
        ServerCodec::with_max_frame_length(10).decode(&mut buf),
        Err(CodecError::Protocol(ProtocolError::FrameTooLarge {
            length: 11,
            max: 10
        }))
    ));

    let mut buf = BytesMut::from(&[0x80u8][..]);
    assert_eq!(decode_frame(&mut buf, 100), Ok(None));
    buf.extend_from_slice(&[0x02]);
    assert_eq!(
        decode_frame(&mut buf, 100),
        Err(ProtocolError::FrameTooLarge {
            length: 256,
            max: 100
        })
    );
}

#[test]
fn nul_padding_does_not_bypass_the_limit() {
    let mut buf = BytesMut::from(&vec![0u8; 10_000][..]);
    buf.extend_from_slice(&prefixed(101, 0));
    assert_eq!(
        decode_frame(&mut buf, 100),
        Err(ProtocolError::FrameTooLarge {
            length: 101,
            max: 100
        })
    );

    let mut codec = ServerCodec::with_max_frame_length(100);
    let mut buf = BytesMut::from(&vec![0u8; 4096][..]);
    assert!(matches!(codec.decode(&mut buf), Ok(None)));
    assert!(buf.is_empty(), "padding is consumed, not accumulated");
    buf.extend_from_slice(&prefixed(101, 0));
    assert!(matches!(
        codec.decode(&mut buf),
        Err(CodecError::Protocol(ProtocolError::FrameTooLarge { .. }))
    ));

    let mut buf = BytesMut::from(&vec![0u8; 500][..]);
    buf.extend_from_slice(&prefixed(100, 100));
    assert_eq!(decode_frame(&mut buf, 100).unwrap().unwrap().len(), 100);

    let mut buf = prefixed(1, 1);
    buf.extend_from_slice(&[0, 0, 0]);
    buf.extend_from_slice(&[0xe5, 0x80, 0x00]);
    assert_eq!(decode_frame(&mut buf, 100).unwrap().unwrap().len(), 1);
    assert_eq!(
        decode_frame(&mut buf, 100),
        Err(ProtocolError::FrameTooLarge {
            length: 101,
            max: 100
        })
    );
}

#[test]
fn existing_prefix_errors_come_first_and_zero_length_frames_still_pass() {
    let mut buf = BytesMut::from(&[0xff, 0xff, 0xff, 0xff, 0x0f][..]);
    assert_eq!(
        decode_frame(&mut buf, 100),
        Err(ProtocolError::Malformed("negative frame length"))
    );
    let mut buf = BytesMut::from(&[0xff; 5][..]);
    assert_eq!(
        decode_frame(&mut buf, 100),
        Err(ProtocolError::Malformed("bad varint"))
    );

    let mut buf = BytesMut::from(&[0x80, 0x00][..]);
    assert_eq!(decode_frame(&mut buf, 0).unwrap(), Some(Bytes::new()));
}

#[test]
fn a_huge_cap_waits_for_the_body() {
    let mut buf = BytesMut::from(&[0x80, 0x80, 0x80, 0x80, 0x01, 0x00][..]);
    assert_eq!(decode_frame(&mut buf, NO_CAP), Ok(None));
    assert_eq!(
        FrameDecoder::with_max_frame_length(NO_CAP)
            .decode(&mut buf)
            .unwrap(),
        None
    );
    assert_eq!(
        ServerCodec::with_max_frame_length(NO_CAP)
            .decode(&mut buf)
            .unwrap(),
        None
    );

    assert!(matches!(
        decode_frame(&mut buf, DEFAULT_MAX_FRAME_LENGTH),
        Err(ProtocolError::FrameTooLarge {
            length: 268_435_456,
            ..
        })
    ));
}

#[tokio::test]
async fn framed_stream_yields_earlier_packets_then_the_error_then_ends() {
    let mut bytes = vec![0x01, 0x00];
    bytes.extend_from_slice(&prefixed(DEFAULT_MAX_FRAME_LENGTH + 1, 0));
    bytes.extend_from_slice(&[0x01, 0x00]);
    let mut framed = FramedRead::new(&bytes[..], ServerCodec::new());
    assert_eq!(
        framed.next().await.unwrap().unwrap(),
        ServerBoundPacket::Ping
    );
    match framed.next().await.unwrap() {
        Err(CodecError::Protocol(e @ ProtocolError::FrameTooLarge { .. })) => {
            assert_eq!(e.pipeline_action(), PipelineAction::WarnAndClose);
        }
        other => panic!("{other:?}"),
    }
    assert!(framed.next().await.is_none());
}

#[test]
fn pre_auth_cap_covers_the_largest_legal_pre_auth_frame() {
    assert_eq!(PRE_AUTH_MAX_FRAME_LENGTH, 1024);

    let max_token = ServerBoundPacket::Authenticate {
        token: "t".repeat(MAX_TOKEN_LENGTH as usize),
    };
    let payload = max_token.to_bytes().len();
    assert_eq!(payload, 1 + 1 + 32);
    assert_eq!(ServerBoundPacket::Ping.to_bytes().len(), 1);

    assert_eq!(1 + 5 + 32, 38);
    const { assert!(PRE_AUTH_MAX_FRAME_LENGTH >= 20 * 38) };

    let mut codec = ServerCodec::new();
    codec.set_max_frame_length(PRE_AUTH_MAX_FRAME_LENGTH);
    let mut buf = BytesMut::new();
    ClientCodec::new().encode(&max_token, &mut buf).unwrap();
    ClientCodec::new()
        .encode(ServerBoundPacket::Ping, &mut buf)
        .unwrap();
    assert_eq!(codec.decode(&mut buf).unwrap(), Some(max_token));
    assert_eq!(
        codec.decode(&mut buf).unwrap(),
        Some(ServerBoundPacket::Ping)
    );
}

#[test]
fn changing_the_cap_applies_to_the_next_frame_even_with_a_partial_frame_buffered() {
    let mut codec = ServerCodec::new();
    codec.set_max_frame_length(PRE_AUTH_MAX_FRAME_LENGTH);
    assert_eq!(codec.max_frame_length(), PRE_AUTH_MAX_FRAME_LENGTH);

    let mut buf = BytesMut::new();
    ClientCodec::new()
        .encode(
            ServerBoundPacket::Authenticate {
                token: "tok".into(),
            },
            &mut buf,
        )
        .unwrap();
    let big_start = prefixed(5000, 10);
    buf.extend_from_slice(&big_start);

    let first = codec.decode(&mut buf).unwrap();
    assert!(matches!(
        first,
        Some(ServerBoundPacket::Authenticate { .. })
    ));

    codec.set_max_frame_length(DEFAULT_MAX_FRAME_LENGTH);
    assert_eq!(
        codec.decode(&mut buf).unwrap(),
        None,
        "5000 bytes are fine now; wait for the body"
    );

    buf.extend(std::iter::repeat_n(0u8, 4990));
    assert_eq!(
        codec.decode(&mut buf).unwrap(),
        Some(ServerBoundPacket::Ping)
    );
    assert!(buf.is_empty());

    let mut pre = ServerCodec::new();
    pre.set_max_frame_length(PRE_AUTH_MAX_FRAME_LENGTH);
    let mut buf = prefixed(5000, 10);
    assert!(matches!(
        pre.decode(&mut buf),
        Err(CodecError::Protocol(ProtocolError::FrameTooLarge {
            length: 5000,
            max: 1024
        }))
    ));
}

#[test]
fn lowering_the_cap_and_a_partly_buffered_prefix() {
    let mut dec = FrameDecoder::with_max_frame_length(100);
    let mut buf = prefixed(50, 10);
    assert_eq!(dec.decode(&mut buf).unwrap(), None);

    dec.set_max_frame_length(40);
    assert!(matches!(
        dec.decode(&mut buf),
        Err(CodecError::Protocol(ProtocolError::FrameTooLarge {
            length: 50,
            max: 40
        }))
    ));

    let mut dec = FrameDecoder::with_max_frame_length(100);
    let mut buf = BytesMut::from(&[0x80u8][..]);
    assert_eq!(dec.decode(&mut buf).unwrap(), None);
    dec.set_max_frame_length(1000);
    buf.extend_from_slice(&[0x02]);
    assert_eq!(dec.decode(&mut buf).unwrap(), None);
    buf.extend(std::iter::repeat_n(7u8, 256));
    assert_eq!(dec.decode(&mut buf).unwrap().unwrap().len(), 256);

    let mut c = ClientCodec::new();
    c.set_max_frame_length(1);
    assert_eq!(c.max_frame_length(), 1);
    assert!(matches!(
        c.decode(&mut prefixed(2, 0)),
        Err(CodecError::Protocol(_))
    ));
}

#[tokio::test]
async fn cap_can_be_changed_through_framed() {
    use tokio::io::AsyncWriteExt;
    let (a, mut b) = tokio::io::duplex(64 * 1024);
    let mut codec = ServerCodec::new();
    codec.set_max_frame_length(PRE_AUTH_MAX_FRAME_LENGTH);
    let mut server = tokio_util::codec::Framed::new(a, codec);

    let mut bytes = BytesMut::new();
    ClientCodec::new()
        .encode(
            ServerBoundPacket::Authenticate {
                token: "tok".into(),
            },
            &mut bytes,
        )
        .unwrap();
    bytes.extend_from_slice(&prefixed(5000, 4));
    b.write_all(&bytes).await.unwrap();

    assert!(matches!(
        server.next().await.unwrap().unwrap(),
        ServerBoundPacket::Authenticate { .. }
    ));
    server
        .codec_mut()
        .set_max_frame_length(DEFAULT_MAX_FRAME_LENGTH);
    b.write_all(&vec![0u8; 4996]).await.unwrap();
    assert_eq!(
        server.next().await.unwrap().unwrap(),
        ServerBoundPacket::Ping
    );
}
