use tokio_util::codec::Decoder;
use zphira_protocol::data::{FullUserProfile, JudgeEvent, RoomInfo, TouchFrame, TouchPoint};
use zphira_protocol::wire::{Encode, MAX_INITIAL_CAPACITY, initial_capacity};
use zphira_protocol::*;

fn encoded<T: Encode>(v: &T) -> usize {
    let mut b = Vec::new();
    v.encode(&mut b);
    b.len()
}

#[test]
fn claimed_counts_reserve_at_most_the_constant() {
    assert_eq!(MAX_INITIAL_CAPACITY, 1024);

    for min in [
        TouchFrame::MIN_WIRE_SIZE,
        JudgeEvent::WIRE_SIZE,
        FullUserProfile::MIN_WIRE_SIZE,
        TouchPoint::WIRE_SIZE,
        1,
    ] {
        let reserved = initial_capacity(2_000_000, DEFAULT_MAX_FRAME_LENGTH, min);
        assert!(
            reserved <= MAX_INITIAL_CAPACITY,
            "min size {min}: {reserved}"
        );
    }

    assert_eq!(
        initial_capacity(0x0fff_ffff, 4, TouchFrame::MIN_WIRE_SIZE),
        0
    );
}

#[test]
fn element_size_constants_are_true_lower_bounds() {
    assert_eq!(
        encoded(&TouchFrame {
            time: 0.0,
            points: vec![]
        }),
        TouchFrame::MIN_WIRE_SIZE
    );
    assert_eq!(
        encoded(&TouchPoint {
            id: 0,
            pos: CompactPos { x: 0.0, y: 0.0 }
        }),
        TouchPoint::WIRE_SIZE
    );
    for j in [Judgement::Perfect, Judgement::HoldGood] {
        assert_eq!(
            encoded(&JudgeEvent {
                time: 0.0,
                line_id: 0,
                note_id: 0,
                judgement: j
            }),
            JudgeEvent::WIRE_SIZE
        );
    }
    assert_eq!(
        encoded(&FullUserProfile::new(0, "", false)),
        FullUserProfile::MIN_WIRE_SIZE
    );

    let empty = RoomInfo::from_lists(
        "",
        GameState::Playing,
        false,
        false,
        false,
        false,
        false,
        &[],
        &[],
    );
    let one = RoomInfo::from_lists(
        "",
        GameState::Playing,
        false,
        false,
        false,
        false,
        false,
        &[UserProfile::new(0, "")],
        &[],
    );
    assert_eq!(
        encoded(&one) - encoded(&empty),
        4 + FullUserProfile::MIN_WIRE_SIZE
    );
}

fn refused(packet: &[u8]) -> ProtocolError {
    let mut framed = bytes::BytesMut::new();
    encode_frame(packet, &mut framed).unwrap();
    assert!(framed.len() <= 16, "the frame is tiny");
    match ServerCodec::new().decode(&mut framed) {
        Err(CodecError::Protocol(e)) => e,
        other => panic!("a tiny frame with a huge count must fail, got {other:?}"),
    }
}

#[test]
fn tiny_frames_claiming_huge_counts_fail_fast() {
    for count in [
        &[0xff, 0xff, 0xff, 0x7f][..],
        &[0xff, 0xff, 0xff, 0xff, 0x07][..],
    ] {
        for id in [serverbound_id::TOUCHES, serverbound_id::JUDGES] {
            let err = refused(&[&[id][..], count].concat());
            assert!(
                matches!(err, ProtocolError::Malformed(_)),
                "id {id:#x}: {err}"
            );
        }
    }

    for (packet, count) in [
        (
            &[clientbound_id::TOUCHES, 1, 0, 0, 0][..],
            &[0xff, 0xff, 0xff, 0x7f][..],
        ),
        (
            &[clientbound_id::JUDGES, 1, 0, 0, 0][..],
            &[0xff, 0xff, 0xff, 0x7f][..],
        ),
        (
            &[clientbound_id::JOIN_ROOM, 1, 1][..],
            &[0xff, 0xff, 0xff, 0x7f][..],
        ),
        (
            &[
                clientbound_id::AUTHENTICATE,
                1,
                1,
                0,
                0,
                0,
                1,
                b'a',
                0,
                1,
                1,
                b'r',
                0,
                0,
                0,
                0,
                0,
                0,
            ][..],
            &[0xff, 0xff, 0xff, 0x7f][..],
        ),
    ] {
        let mut bytes = packet.to_vec();
        bytes.extend_from_slice(count);
        let err = ClientBoundPacket::decode(&mut &bytes[..]).expect_err("must fail");
        assert!(matches!(err, ProtocolError::Malformed(_)), "{err}");
    }
}
