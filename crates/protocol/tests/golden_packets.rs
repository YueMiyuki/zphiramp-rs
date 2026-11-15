#![allow(clippy::excessive_precision, clippy::approx_constant)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use bytes::BytesMut;
use common::*;
use tokio_util::codec::{Decoder, Encoder};
use zphira_protocol::float16::{float_to_half, half_to_float};
use zphira_protocol::*;

const ZH: &str = "你好，世界";
const EMOJI: &str = "🎮🎵🏆";
const MIX: &str = "Zenith 音游战队 🎶 é";

const NAN: f32 = f32::NAN;
const INF: f32 = f32::INFINITY;
const NINF: f32 = f32::NEG_INFINITY;

fn nan_payload() -> f32 {
    f32::from_bits(0x7fc0_1234)
}
fn snan() -> f32 {
    f32::from_bits(0x7f80_0001)
}
fn neg_nan() -> f32 {
    f32::from_bits(0xffc0_0000)
}
fn min_value() -> f32 {
    f32::from_bits(1)
}

#[rustfmt::skip]
fn f16s() -> Vec<f32> {
    vec![
        0.0, -0.0, 1.0, -1.0, 0.5, -2.0, 0.1, 1.5, 3.14159265, 100.25,
        65504.0,
        65519.0, 65520.0,
        1e9, INF, NINF, NAN, nan_payload(), neg_nan(),
        6.1035156e-5,
        6.097555e-5,
        5.9604645e-8,
        2.9802322e-8,
        4.4703484e-8,
        1e-8, -1e-8,
        min_value(), f32::MAX, -f32::MAX, f32::MIN_POSITIVE,
        0.99951171875,
        1.00048828125,
        2047.5, 2048.5,
    ]
}

#[rustfmt::skip]
fn times() -> Vec<f32> {
    vec![
        0.0, -0.0, 1.5, 123.456, -7.25, NAN, nan_payload(), snan(), INF, NINF,
        min_value(), f32::MAX, f32::MIN_POSITIVE,
    ]
}

fn rep(s: &str, n: usize) -> String {
    s.repeat(n)
}

fn up(id: i32, n: &str) -> UserProfile {
    UserProfile::new(id, n)
}

fn fup(id: i32, n: &str, m: bool) -> FullUserProfile {
    FullUserProfile::new(id, n, m)
}

fn tp(id: i32, x: f32, y: f32) -> TouchPoint {
    TouchPoint {
        id: id as i8,
        pos: CompactPos { x, y },
    }
}

fn tf(time: f32, points: Vec<TouchPoint>) -> TouchFrame {
    TouchFrame { time, points }
}

fn je(time: f32, line_id: i32, note_id: i32, judgement: Judgement) -> JudgeEvent {
    JudgeEvent {
        time,
        line_id,
        note_id,
        judgement,
    }
}

const JUDGEMENTS: [Judgement; 6] = [
    Judgement::Perfect,
    Judgement::Good,
    Judgement::Bad,
    Judgement::Miss,
    Judgement::HoldPerfect,
    Judgement::HoldGood,
];

fn frames() -> Vec<TouchFrame> {
    vec![
        tf(0.0, vec![]),
        tf(1.5, vec![tp(0, 0.5, -0.5)]),
        tf(
            2.25,
            vec![
                tp(1, 0.0, 0.0),
                tp(-1, 1.0, 1.0),
                tp(127, 65504.0, -65504.0),
                tp(-128, 0.099975586, 3.0),
            ],
        ),
    ]
}

fn judges() -> Vec<JudgeEvent> {
    vec![
        je(0.0, 0, 0, Judgement::Perfect),
        je(1.25, 3, 17, Judgement::Good),
        je(-0.5, -1, -2, Judgement::Bad),
        je(99.5, i32::MAX, i32::MIN, Judgement::Miss),
        je(NAN, 0x0102_0304, 5, Judgement::HoldPerfect),
        je(INF, 6, 7, Judgement::HoldGood),
    ]
}

fn rooms() -> Vec<RoomInfo> {
    let many: Vec<UserProfile> = (0..40).map(|i| up(1000 + i, &format!("user{i}"))).collect();
    vec![
        RoomInfo::from_lists(
            "room1",
            GameState::SelectChart(None),
            false,
            false,
            false,
            false,
            false,
            &[],
            &[],
        ),
        RoomInfo::from_lists(
            "Room-中文",
            GameState::WaitForReady,
            true,
            true,
            true,
            true,
            true,
            &[up(1, "alice"), up(2, "鲍勃")],
            &[up(-5, "watcher🎥")],
        ),
        RoomInfo::from_lists(
            "p",
            GameState::Playing,
            false,
            true,
            false,
            true,
            false,
            &[up(i32::MAX, "max")],
            &[],
        ),
        RoomInfo::from_lists(
            "",
            GameState::SelectChart(Some(42)),
            true,
            false,
            true,
            false,
            true,
            &[],
            &[up(7, ""), up(8, "m")],
        ),
        RoomInfo::from_lists(
            "abcdefghij0123456789",
            GameState::SelectChart(Some(-1)),
            false,
            false,
            false,
            true,
            true,
            &[up(0, "zero"), up(i32::MIN, "min")],
            &[],
        ),
        RoomInfo::from_lists(
            "abcdefghij01234567890",
            GameState::Playing,
            false,
            false,
            false,
            false,
            false,
            &[up(3, "x")],
            &[],
        ),
        RoomInfo::from_lists(
            "big",
            GameState::WaitForReady,
            true,
            false,
            true,
            false,
            false,
            &many,
            &[up(9999, "obs")],
        ),
    ]
}

fn states() -> Vec<GameState> {
    vec![
        GameState::SelectChart(None),
        GameState::SelectChart(Some(0)),
        GameState::SelectChart(Some(42)),
        GameState::SelectChart(Some(-1)),
        GameState::SelectChart(Some(i32::MAX)),
        GameState::SelectChart(Some(i32::MIN)),
        GameState::WaitForReady,
        GameState::Playing,
    ]
}

fn s(x: &str) -> String {
    x.to_owned()
}

fn messages() -> Vec<(&'static str, Message)> {
    vec![
        (
            "Chat.empty",
            Message::Chat {
                user: 0,
                content: s(""),
            },
        ),
        (
            "Chat.system",
            Message::Chat {
                user: -1,
                content: s("欢迎加入 Zenith 音游战队 服务器！"),
            },
        ),
        (
            "Chat.user",
            Message::Chat {
                user: 42,
                content: s("hello"),
            },
        ),
        (
            "Chat.emoji",
            Message::Chat {
                user: i32::MAX,
                content: s(EMOJI),
            },
        ),
        (
            "Chat.long",
            Message::Chat {
                user: i32::MIN,
                content: rep("长", 400),
            },
        ),
        ("CreateRoom.0", Message::CreateRoom { user: 0 }),
        ("CreateRoom.1", Message::CreateRoom { user: 1 }),
        ("CreateRoom.neg", Message::CreateRoom { user: -123456 }),
        (
            "JoinRoom.a",
            Message::JoinRoom {
                user: 1,
                name: s("alice"),
            },
        ),
        (
            "JoinRoom.zh",
            Message::JoinRoom {
                user: 2,
                name: s("鲍勃"),
            },
        ),
        (
            "JoinRoom.empty",
            Message::JoinRoom {
                user: i32::MIN,
                name: s(""),
            },
        ),
        (
            "LeaveRoom.a",
            Message::LeaveRoom {
                user: 1,
                name: s("alice"),
            },
        ),
        (
            "LeaveRoom.emoji",
            Message::LeaveRoom {
                user: -2,
                name: s("🎮"),
            },
        ),
        ("NewHost.a", Message::NewHost { user: 5 }),
        ("NewHost.neg", Message::NewHost { user: -5 }),
        (
            "SelectChart.a",
            Message::SelectChart {
                user: 1,
                name: s("Chart A"),
                id: 1234,
            },
        ),
        (
            "SelectChart.zh",
            Message::SelectChart {
                user: 2,
                name: s("谱面 ✨"),
                id: -7,
            },
        ),
        (
            "SelectChart.empty",
            Message::SelectChart {
                user: 0,
                name: s(""),
                id: 0,
            },
        ),
        ("GameStart.a", Message::GameStart { user: 3 }),
        ("Ready.a", Message::Ready { user: 4 }),
        ("CancelReady.a", Message::CancelReady { user: 5 }),
        ("CancelGame.a", Message::CancelGame { user: 6 }),
        ("StartPlaying", Message::StartPlaying),
        (
            "Played.a",
            Message::Played {
                user: 1,
                score: 1_000_000,
                accuracy: 99.5,
                full_combo: true,
            },
        ),
        (
            "Played.zero",
            Message::Played {
                user: 0,
                score: 0,
                accuracy: 0.0,
                full_combo: false,
            },
        ),
        (
            "Played.neg",
            Message::Played {
                user: -1,
                score: -1,
                accuracy: -0.0,
                full_combo: true,
            },
        ),
        (
            "Played.nan",
            Message::Played {
                user: 2,
                score: i32::MAX,
                accuracy: NAN,
                full_combo: false,
            },
        ),
        (
            "Played.nan_payload",
            Message::Played {
                user: 2,
                score: i32::MIN,
                accuracy: nan_payload(),
                full_combo: false,
            },
        ),
        (
            "Played.snan",
            Message::Played {
                user: 2,
                score: 5,
                accuracy: snan(),
                full_combo: true,
            },
        ),
        (
            "Played.inf",
            Message::Played {
                user: 3,
                score: 5,
                accuracy: INF,
                full_combo: true,
            },
        ),
        (
            "Played.ninf",
            Message::Played {
                user: 3,
                score: 5,
                accuracy: NINF,
                full_combo: false,
            },
        ),
        (
            "Played.denormal",
            Message::Played {
                user: 3,
                score: 5,
                accuracy: min_value(),
                full_combo: false,
            },
        ),
        (
            "Played.max",
            Message::Played {
                user: 3,
                score: 5,
                accuracy: f32::MAX,
                full_combo: true,
            },
        ),
        ("GameEnd", Message::GameEnd),
        ("Abort.a", Message::Abort { user: 7 }),
        ("LockRoom.true", Message::LockRoom { lock: true }),
        ("LockRoom.false", Message::LockRoom { lock: false }),
        ("CycleRoom.true", Message::CycleRoom { cycle: true }),
        ("CycleRoom.false", Message::CycleRoom { cycle: false }),
    ]
}

fn serverbound() -> Vec<(String, ServerBoundPacket)> {
    use ServerBoundPacket as P;
    let mut v: Vec<(String, ServerBoundPacket)> = Vec::new();
    let mut add = |n: &str, p: ServerBoundPacket| v.push((n.to_owned(), p));
    let auth = |t: String| P::Authenticate { token: t };
    let chat = |t: String| P::Chat { message: t };
    let create = |t: &str| P::CreateRoom {
        room_id: t.to_owned(),
    };

    add("Ping", P::Ping);
    add("Authenticate.empty", auth(s("")));
    add("Authenticate.ascii", auth(s("abcDEF0123456789")));
    add("Authenticate.max32", auth(rep("t", 32)));
    add("Authenticate.max32_cjk", auth(rep("令", 10) + "ab"));
    add("Authenticate.zh", auth(s("令牌")));
    add("Authenticate.emoji", auth(s("🎮🎮")));
    add("Authenticate.nul", auth(s("a\0b")));
    add("Authenticate.too_long33", auth(rep("t", 33)));
    add("Authenticate.too_long_cjk", auth(rep("令", 11)));

    add("Chat.empty", chat(s("")));
    add("Chat.hello", chat(s("hello")));
    add("Chat.zh", chat(s(ZH)));
    add("Chat.emoji", chat(s(EMOJI)));
    add("Chat.mix", chat(s(MIX)));
    add("Chat.max200", chat(rep("x", 200)));
    add("Chat.max200_cjk", chat(rep("字", 66) + "yz"));
    add("Chat.too_long201", chat(rep("x", 201)));
    add("Chat.too_long_cjk", chat(rep("字", 67)));
    add("Chat.long_varint2", chat(rep("y", 128)));

    add("Touches.empty", P::Touches { frames: vec![] });
    add(
        "Touches.one_empty_frame",
        P::Touches {
            frames: vec![tf(0.0, vec![])],
        },
    );
    add("Touches.frames", P::Touches { frames: frames() });
    let (f16, times) = (f16s(), times());
    for i in 0..f16.len() {
        add(
            &format!("Touches.f16_{i}"),
            P::Touches {
                frames: vec![tf(
                    times[i % times.len()],
                    vec![tp(i as i32, f16[i], f16[f16.len() - 1 - i])],
                )],
            },
        );
    }
    let many: Vec<TouchFrame> = (0..130)
        .map(|i| {
            tf(
                i as f32 * 0.016,
                vec![tp(i, i as f32 * 0.5, (-i) as f32 * 0.25)],
            )
        })
        .collect();
    add("Touches.many130", P::Touches { frames: many });

    add("Judges.empty", P::Judges { judges: vec![] });
    add(
        "Judges.one",
        P::Judges {
            judges: vec![je(12.5, 1, 2, Judgement::Perfect)],
        },
    );
    add("Judges.all", P::Judges { judges: judges() });
    for i in 0..times.len() {
        add(
            &format!("Judges.time_{i}"),
            P::Judges {
                judges: vec![je(times[i], i as i32, -(i as i32), JUDGEMENTS[i % 6])],
            },
        );
    }

    add("CreateRoom.empty", create(""));
    add("CreateRoom.simple", create("room1"));
    add("CreateRoom.max20", create("abcdefghij0123456789"));
    add("CreateRoom.zh", create("房间"));
    add("CreateRoom.emoji", create("🎮ab"));
    add("CreateRoom.max20_cjk", create(&(rep("房", 6) + "ab")));
    add("CreateRoom.too_long21", create("abcdefghij01234567890"));
    add("CreateRoom.too_long_cjk", create(&rep("房", 7)));

    add(
        "JoinRoom.player",
        P::JoinRoom {
            room_id: s("room1"),
            monitor: false,
        },
    );
    add(
        "JoinRoom.monitor",
        P::JoinRoom {
            room_id: s("room1"),
            monitor: true,
        },
    );
    add(
        "JoinRoom.empty",
        P::JoinRoom {
            room_id: s(""),
            monitor: false,
        },
    );
    add(
        "JoinRoom.zh",
        P::JoinRoom {
            room_id: s("测试房"),
            monitor: true,
        },
    );
    add(
        "JoinRoom.max20",
        P::JoinRoom {
            room_id: rep("r", 20),
            monitor: false,
        },
    );
    add(
        "JoinRoom.too_long",
        P::JoinRoom {
            room_id: rep("r", 21),
            monitor: true,
        },
    );

    add("LeaveRoom", P::LeaveRoom);
    add("LockRoom.true", P::LockRoom { lock: true });
    add("LockRoom.false", P::LockRoom { lock: false });
    add("CycleRoom.true", P::CycleRoom { cycle: true });
    add("CycleRoom.false", P::CycleRoom { cycle: false });

    for i in [
        0,
        1,
        -1,
        42,
        255,
        256,
        65535,
        0x0102_0304,
        i32::MAX,
        i32::MIN,
        -2,
    ] {
        add(&format!("SelectChart.{i}"), P::SelectChart { id: i });
        add(&format!("Played.{i}"), P::Played { id: i });
    }
    add("RequestStart", P::RequestStart);
    add("Ready", P::Ready);
    add("CancelReady", P::CancelReady);
    add("Abort", P::Abort);
    v
}

fn void_variants(
    v: &mut Vec<(String, ClientBoundPacket)>,
    name: &str,
    make: fn(PacketResult<()>) -> ClientBoundPacket,
) {
    let mut add = |suffix: &str, r: PacketResult<()>| v.push((format!("{name}.{suffix}"), make(r)));
    add("success", PacketResult::success_void());
    add("failed_empty", PacketResult::failed(""));
    add("failed_msg", PacketResult::failed("operation failed"));
    add("failed_zh", PacketResult::failed("没有权限"));
    add("failed_emoji", PacketResult::failed("🚫 nope"));
    add("failed_long", PacketResult::failed(rep("long ", 60)));
}

fn clientbound() -> Vec<(String, ClientBoundPacket)> {
    use ClientBoundPacket as P;
    let mut v: Vec<(String, ClientBoundPacket)> = Vec::new();
    v.push(("Pong".into(), P::Pong));

    let auth_failed = |m: String| P::Authenticate(PacketResult::failed(m));
    let auth_ok = |u: FullUserProfile, r: Option<RoomInfo>| {
        P::Authenticate(PacketResult::success(AuthenticateOk {
            user_profile: u,
            room_info: r,
        }))
    };
    v.push(("Authenticate.failed_empty".into(), auth_failed(s(""))));
    v.push((
        "Authenticate.failed_msg".into(),
        auth_failed(s("bad token")),
    ));
    v.push((
        "Authenticate.failed_zh".into(),
        auth_failed(s("认证失败 🙅")),
    ));
    v.push((
        "Authenticate.failed_long".into(),
        auth_failed(rep("e", 300)),
    ));
    v.push((
        "Authenticate.success".into(),
        auth_ok(fup(1, "alice", false), None),
    ));
    v.push((
        "Authenticate.success_monitor".into(),
        auth_ok(fup(-1, "观察者", true), None),
    ));
    v.push((
        "Authenticate.success_empty_name".into(),
        auth_ok(fup(0, "", false), None),
    ));
    for (i, room) in rooms().into_iter().enumerate() {
        v.push((
            format!("Authenticate.success_room{i}"),
            auth_ok(
                fup(100 + i as i32, &format!("player{i}"), i % 2 == 0),
                Some(room),
            ),
        ));
    }

    void_variants(&mut v, "Chat", P::Chat);
    void_variants(&mut v, "CreateRoom", P::CreateRoom);
    void_variants(&mut v, "LeaveRoom", P::LeaveRoom);
    void_variants(&mut v, "LockRoom", P::LockRoom);
    void_variants(&mut v, "CycleRoom", P::CycleRoom);
    void_variants(&mut v, "SelectChart", P::SelectChart);
    void_variants(&mut v, "RequestStart", P::RequestStart);
    void_variants(&mut v, "Ready", P::Ready);
    void_variants(&mut v, "CancelReady", P::CancelReady);
    void_variants(&mut v, "Played", P::Played);
    void_variants(&mut v, "Abort", P::Abort);

    v.push((
        "Touches.empty".into(),
        P::Touches {
            id: 0,
            frames: vec![],
        },
    ));
    v.push((
        "Touches.frames".into(),
        P::Touches {
            id: 7,
            frames: frames(),
        },
    ));
    v.push((
        "Touches.neg_id".into(),
        P::Touches {
            id: -1,
            frames: vec![tf(1.0, vec![tp(3, 0.25, 0.75)])],
        },
    ));
    v.push((
        "Touches.max_id".into(),
        P::Touches {
            id: i32::MAX,
            frames: vec![tf(NAN, vec![])],
        },
    ));
    v.push((
        "Touches.min_id".into(),
        P::Touches {
            id: i32::MIN,
            frames: frames(),
        },
    ));
    let (f16, times) = (f16s(), times());
    for i in (0..f16.len()).step_by(3) {
        v.push((
            format!("Touches.f16_{i}"),
            P::Touches {
                id: i as i32,
                frames: vec![tf(
                    times[i % times.len()],
                    vec![tp(i as i32, f16[i], f16[(i * 7) % f16.len()])],
                )],
            },
        ));
    }

    v.push((
        "Judges.empty".into(),
        P::Judges {
            id: 0,
            judges: vec![],
        },
    ));
    v.push((
        "Judges.all".into(),
        P::Judges {
            id: 5,
            judges: judges(),
        },
    ));
    v.push((
        "Judges.neg_id".into(),
        P::Judges {
            id: -9,
            judges: vec![je(0.5, 1, 1, Judgement::Good)],
        },
    ));

    for (name, m) in messages() {
        v.push((format!("Message.{name}"), P::Message(m)));
    }
    for (i, st) in states().into_iter().enumerate() {
        v.push((format!("ChangeState.{i}"), P::ChangeState(st)));
    }
    v.push(("ChangeHost.true".into(), P::ChangeHost { host: true }));
    v.push(("ChangeHost.false".into(), P::ChangeHost { host: false }));

    let join_failed = |m: &str| P::JoinRoom(PacketResult::failed(m));
    let join_ok = |st: GameState, users: &[UserProfile], monitors: &[UserProfile], live: bool| {
        P::JoinRoom(PacketResult::success(JoinRoomOk::from_lists(
            st, users, monitors, live,
        )))
    };
    v.push(("JoinRoom.failed_empty".into(), join_failed("")));
    v.push(("JoinRoom.failed_msg".into(), join_failed("room not found")));
    v.push(("JoinRoom.failed_zh".into(), join_failed("房间不存在")));
    v.push((
        "JoinRoom.success_empty".into(),
        join_ok(GameState::SelectChart(None), &[], &[], false),
    ));
    v.push((
        "JoinRoom.success_users".into(),
        join_ok(
            GameState::WaitForReady,
            &[up(1, "alice"), up(2, "鲍勃")],
            &[up(3, "obs")],
            true,
        ),
    ));
    v.push((
        "JoinRoom.success_playing".into(),
        join_ok(GameState::Playing, &[up(i32::MIN, "n")], &[], true),
    ));
    v.push((
        "JoinRoom.success_chart".into(),
        join_ok(
            GameState::SelectChart(Some(99)),
            &[up(5, "e")],
            &[up(6, "🎥"), up(7, "g")],
            false,
        ),
    ));
    let many: Vec<UserProfile> = (0..130).map(|i| up(i, &format!("u{i}"))).collect();
    v.push((
        "JoinRoom.success_many130".into(),
        join_ok(GameState::SelectChart(Some(1)), &many, &[], false),
    ));

    v.push((
        "OnJoinRoom.player".into(),
        P::OnJoinRoom(fup(1, "alice", false)),
    ));
    v.push((
        "OnJoinRoom.monitor".into(),
        P::OnJoinRoom(fup(2, "观察者", true)),
    ));
    v.push((
        "OnJoinRoom.extremes".into(),
        P::OnJoinRoom(fup(i32::MIN, "", true)),
    ));
    v.push((
        "OnJoinRoom.max".into(),
        P::OnJoinRoom(fup(i32::MAX, &rep("n", 200), false)),
    ));
    v
}

// Positions are rounded to half precision on the wire.
trait Quantize: Sized {
    fn quantized(&self) -> Self;
}

fn q(x: f32) -> f32 {
    half_to_float(float_to_half(x))
}

impl Quantize for CompactPos {
    fn quantized(&self) -> Self {
        CompactPos {
            x: q(self.x),
            y: q(self.y),
        }
    }
}
impl Quantize for TouchPoint {
    fn quantized(&self) -> Self {
        TouchPoint {
            id: self.id,
            pos: self.pos.quantized(),
        }
    }
}
impl Quantize for TouchFrame {
    fn quantized(&self) -> Self {
        TouchFrame {
            time: self.time,
            points: self.points.iter().map(Quantize::quantized).collect(),
        }
    }
}
impl Quantize for ServerBoundPacket {
    fn quantized(&self) -> Self {
        match self {
            ServerBoundPacket::Touches { frames } => ServerBoundPacket::Touches {
                frames: frames.iter().map(Quantize::quantized).collect(),
            },
            other => other.clone(),
        }
    }
}
impl Quantize for ClientBoundPacket {
    fn quantized(&self) -> Self {
        match self {
            ClientBoundPacket::Touches { id, frames } => ClientBoundPacket::Touches {
                id: *id,
                frames: frames.iter().map(Quantize::quantized).collect(),
            },
            other => other.clone(),
        }
    }
}
macro_rules! identity_quantize {
    ($($t:ty),*) => {$(impl Quantize for $t { fn quantized(&self) -> Self { self.clone() } })*};
}
identity_quantize!(
    UserProfile,
    FullUserProfile,
    RoomInfo,
    GameState,
    Message,
    Judgement,
    JudgeEvent,
    PacketResult<()>,
    PacketResult<UserProfile>
);

fn check_refused(what: &str, rt: &str, err: &ProtocolError) {
    let mut parts = rt.splitn(3, ':');
    assert_eq!(parts.next(), Some("ERR"), "{what}");
    let class = parts.next().unwrap();
    let message = parts.next().unwrap();
    assert_eq!(err.java_exception(), Some(class), "{what}: exception class");
    assert_eq!(err.to_string(), message, "{what}: message");
}

fn check_packet_sb(name: &str, value: &ServerBoundPacket, row: &[String]) {
    let (hex_in, rt, framed) = (unhex(&row[1]), &row[2], unhex(&row[3]));
    assert_bytes(&value.to_bytes(), &hex_in, &format!("{name}: encode"));
    let decoded = ServerBoundPacket::decode(&mut &hex_in[..]);
    if rt.starts_with("ERR:") {
        check_refused(name, rt, &decoded.expect_err(name));
    } else {
        let decoded = decoded.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_same(&decoded, &value.quantized(), name);
        assert_eq!(hex(&decoded.to_bytes()), *rt, "{name}: decode-then-encode");
    }

    let mut out = BytesMut::new();
    ClientCodec::new().encode(value, &mut out).unwrap();
    assert_bytes(&out, &framed, &format!("{name}: framed encode"));
    if !rt.starts_with("ERR:") {
        let mut input = BytesMut::from(&framed[..]);
        let got = ServerCodec::new().decode(&mut input).unwrap().expect(name);
        assert_same(&got, &value.quantized(), name);
        assert!(input.is_empty());
    }
}

fn check_packet_cb(name: &str, value: &ClientBoundPacket, row: &[String]) {
    let (hex_in, rt, framed) = (unhex(&row[1]), &row[2], unhex(&row[3]));
    assert_bytes(&value.to_bytes(), &hex_in, &format!("{name}: encode"));
    let decoded = ClientBoundPacket::decode(&mut &hex_in[..]);
    if rt.starts_with("ERR:") {
        check_refused(name, rt, &decoded.expect_err(name));
    } else {
        let decoded = decoded.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_same(&decoded, &value.quantized(), name);
        assert_eq!(hex(&decoded.to_bytes()), *rt, "{name}: decode-then-encode");
    }
    let mut out = BytesMut::new();
    ServerCodec::new().encode(value, &mut out).unwrap();
    assert_bytes(&out, &framed, &format!("{name}: framed encode"));
    if !rt.starts_with("ERR:") {
        let mut input = BytesMut::from(&framed[..]);
        let got = ClientCodec::new().decode(&mut input).unwrap().expect(name);
        assert_same(&got, &value.quantized(), name);
        assert!(input.is_empty());
    }
}

fn check_type<T>(name: &str, value: &T, hex_in: &[u8], rt: &str)
where
    T: Encode + Decode + Debug + PartialEq + Quantize,
{
    let mut enc = Vec::new();
    value.encode(&mut enc);
    assert_bytes(&enc, hex_in, &format!("{name}: encode"));
    let decoded = T::decode(&mut &hex_in[..]);
    if let Some(expected) = rt.strip_prefix("ERR ") {
        let err = decoded.expect_err(name);
        let mut parts = expected.split(' ');
        assert_eq!(err.java_exception(), parts.next(), "{name}");
        return;
    }
    let decoded = decoded.unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_same(&decoded, &value.quantized(), name);
    let mut again = Vec::new();
    decoded.encode(&mut again);
    assert_eq!(hex(&again), rt, "{name}: decode-then-encode");
}

type TypeCheck = Box<dyn Fn(&str, &[u8], &str)>;

fn tcase<T>(name: &str, value: T) -> (String, TypeCheck)
where
    T: Encode + Decode + Debug + PartialEq + Quantize + 'static,
{
    let owned = name.to_owned();
    (
        name.to_owned(),
        Box::new(move |_, hex_in, rt| check_type(&owned, &value, hex_in, rt)),
    )
}

#[allow(clippy::vec_init_then_push)]
fn types() -> Vec<(String, TypeCheck)> {
    let mut t: Vec<(String, TypeCheck)> = Vec::new();
    t.push(tcase("UserProfile.simple", up(1, "alice")));
    t.push(tcase("UserProfile.empty", up(0, "")));
    t.push(tcase("UserProfile.zh", up(-1, ZH)));
    t.push(tcase("UserProfile.emoji", up(i32::MAX, EMOJI)));
    t.push(tcase("UserProfile.min", up(i32::MIN, MIX)));
    t.push(tcase("UserProfile.long", up(5, &rep("n", 1000))));
    t.push(tcase("FullUserProfile.player", fup(1, "alice", false)));
    t.push(tcase("FullUserProfile.monitor", fup(2, "观察者", true)));
    t.push(tcase("FullUserProfile.empty", fup(0, "", false)));
    for (i, r) in rooms().into_iter().enumerate() {
        t.push(tcase(&format!("RoomInfo.{i}"), r));
    }
    for (i, st) in states().into_iter().enumerate() {
        t.push(tcase(&format!("GameState.{i}"), st));
    }
    for (name, m) in messages() {
        t.push(tcase(&format!("Message.{name}"), m));
    }
    let names = ["Perfect", "Good", "Bad", "Miss", "HoldPerfect", "HoldGood"];
    for (j, n) in JUDGEMENTS.iter().zip(names) {
        t.push(tcase(&format!("Judgement.{n}"), *j));
    }
    let times = times();
    for i in 0..6 {
        t.push(tcase(
            &format!("JudgeEvent.{i}"),
            je(times[i], i as i32 - 3, i as i32 * 100, JUDGEMENTS[i]),
        ));
    }
    let f16 = f16s();
    for i in 0..f16.len() {
        t.push(tcase(
            &format!("CompactPos.{i}"),
            CompactPos {
                x: f16[i],
                y: f16[(i * 5 + 3) % f16.len()],
            },
        ));
    }
    t.push(tcase("TouchPoint.a", tp(0, 0.5, -0.5)));
    t.push(tcase("TouchPoint.neg", tp(-128, -1.0, 1.0)));
    t.push(tcase("TouchPoint.max", tp(127, 65504.0, 0.0)));
    t.push(tcase("TouchFrame.empty", tf(0.0, vec![])));
    t.push(tcase(
        "TouchFrame.points",
        tf(2.5, vec![tp(1, 0.25, 0.75), tp(2, -0.25, -0.75)]),
    ));
    t.push(tcase("TouchFrame.nan_time", tf(NAN, vec![tp(1, NAN, INF)])));
    t.push(tcase(
        "PacketResult.void_success",
        PacketResult::success_void(),
    ));
    t.push(tcase(
        "PacketResult.void_failed",
        PacketResult::<()>::failed("no"),
    ));
    t.push(tcase(
        "PacketResult.void_failed_empty",
        PacketResult::<()>::failed(""),
    ));
    t.push(tcase(
        "PacketResult.user_success",
        PacketResult::success(up(3, "carol")),
    ));
    t.push(tcase(
        "PacketResult.user_failed",
        PacketResult::<UserProfile>::failed("没有"),
    ));
    t
}

fn assert_same_names(what: &str, fixture: &BTreeSet<String>, ours: &BTreeSet<String>) {
    let missing: Vec<_> = fixture.difference(ours).collect();
    let extra: Vec<_> = ours.difference(fixture).collect();
    assert!(
        missing.is_empty(),
        "{what}: in the fixture but not in the Rust table: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "{what}: in the Rust table but not in the fixture: {extra:?}"
    );
}

#[test]
fn packet_vectors() {
    let rows = rows("packets.tsv");
    let by_name: BTreeMap<&str, &Vec<String>> = rows.iter().map(|r| (r[0].as_str(), r)).collect();
    assert_eq!(by_name.len(), rows.len(), "duplicate names in packets.tsv");

    let sb = serverbound();
    let cb = clientbound();
    let ours: BTreeSet<String> = sb
        .iter()
        .map(|(n, _)| format!("sb.{n}"))
        .chain(cb.iter().map(|(n, _)| format!("cb.{n}")))
        .collect();
    assert_eq!(
        ours.len(),
        sb.len() + cb.len(),
        "duplicate names in the Rust table"
    );
    assert_same_names(
        "packets.tsv",
        &by_name.keys().map(|k| k.to_string()).collect(),
        &ours,
    );

    for (name, value) in &sb {
        let full = format!("sb.{name}");
        check_packet_sb(&full, value, by_name[full.as_str()]);
    }
    for (name, value) in &cb {
        let full = format!("cb.{name}");
        check_packet_cb(&full, value, by_name[full.as_str()]);
    }
    eprintln!("{} packet vectors", rows.len());
}

#[test]
fn type_vectors() {
    let rows = rows("types.tsv");
    let by_name: BTreeMap<&str, &Vec<String>> = rows.iter().map(|r| (r[0].as_str(), r)).collect();
    let table = types();
    let ours: BTreeSet<String> = table.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(ours.len(), table.len(), "duplicate names in the Rust table");
    assert_same_names(
        "types.tsv",
        &by_name.keys().map(|k| k.to_string()).collect(),
        &ours,
    );
    for (name, check) in &table {
        let row = by_name[name.as_str()];
        check(name, &unhex(&row[1]), &row[2]);
    }
    eprintln!("{} type vectors", rows.len());
}

#[test]
fn registry_matches() {
    let rows = rows("registry.tsv");
    let sb_names: Vec<_> = serverbound()
        .into_iter()
        .map(|(_, p)| (p.id(), p.name()))
        .collect();
    let cb_names: Vec<_> = clientbound()
        .into_iter()
        .map(|(_, p)| (p.id(), p.name()))
        .collect();
    let mut seen = 0;
    for row in &rows {
        let id = u8::from_str_radix(&row[2], 16).unwrap();
        let table = if row[0] == "sb" { &sb_names } else { &cb_names };
        assert!(
            table.contains(&(id, row[1].as_str())),
            "{row:?} is not what the Rust enum says"
        );
        seen += 1;
    }
    assert_eq!(seen, 16 + 20);

    let ids: BTreeSet<u8> = sb_names.iter().map(|(i, _)| *i).collect();
    assert_eq!(ids, (0..16).collect());
    let ids: BTreeSet<u8> = cb_names.iter().map(|(i, _)| *i).collect();
    assert_eq!(ids, (0..20).collect());

    assert_eq!(ServerBoundPacket::Ping.id(), serverbound_id::PING);
    assert_eq!(
        ClientBoundPacket::Abort(PacketResult::success_void()).id(),
        clientbound_id::ABORT
    );
}

#[test]
fn truncated_packets() {
    let packets = rows("packets.tsv");
    let by_name: BTreeMap<&str, &Vec<String>> =
        packets.iter().map(|r| (r[0].as_str(), r)).collect();
    let prefixes = rows("prefixes.tsv");
    assert_eq!(prefixes.len(), packets.len());
    let mut checked = 0;
    for row in &prefixes {
        let name = row[0].as_str();
        let full = unhex(&by_name[name][1]);
        let tokens: Vec<&str> = row
            .get(1)
            .map(|t| t.split(' ').collect())
            .unwrap_or_default();
        assert_eq!(tokens.len(), full.len(), "{name}: one token per prefix");
        for (n, token) in tokens.iter().enumerate() {
            let prefix = &full[..n];
            let outcome = if name.starts_with("sb.") {
                ServerBoundPacket::decode(&mut &prefix[..]).map(|p| crc32(&p.to_bytes()))
            } else {
                ClientBoundPacket::decode(&mut &prefix[..]).map(|p| crc32(&p.to_bytes()))
            };
            let got = match outcome {
                Ok(crc) => format!("K:{crc}"),
                Err(e) => code(&e).to_owned(),
            };
            assert_eq!(&got, token, "{name} cut to {n} of {} bytes", full.len());
            checked += 1;
        }
    }
    eprintln!("{checked} prefixes");
}
