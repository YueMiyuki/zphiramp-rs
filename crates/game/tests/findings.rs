mod support;

use std::sync::Arc;
use std::time::Duration;

use support::*;
use zphira_game::RoomPools;
use zphira_protocol::ServerBoundPacket as Sb;

const HINT: &str = "> 点击锁定房间按钮查看当前谱池状态。";
const COUNTDOWN_150: &str =
    "> 已达到开局人数，150 秒后锁定投票并进入准备阶段。人数不足会取消倒计时。";

async fn sleep(secs: f64) {
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
}

async fn join(c: &mut Client, room: &str, monitor: bool) {
    c.send(Sb::JoinRoom {
        room_id: room.into(),
        monitor,
    })
    .await;
    c.drain().await;
}

async fn pair(w: &World) -> (Client, Client) {
    w.ctx.create_room("r1", RoomPools::Default).unwrap();
    let mut a = w.login(100, "Alice").await;
    let mut b = w.login(200, "Bob").await;
    join(&mut a, "r1", false).await;
    join(&mut b, "r1", false).await;
    a.drain().await;
    (a, b)
}

async fn into_ready(a: &mut Client, b: &mut Client) {
    a.send(Sb::SelectChart { id: 1 }).await;
    b.send(Sb::SelectChart { id: 1 }).await;
    b.drain().await;
    sleep(10.5).await;
    a.drain().await;
    b.drain().await;
}

async fn into_round(a: &mut Client, b: &mut Client) {
    into_ready(a, b).await;
    a.send(Sb::Ready).await;
    a.drain().await;
    b.send(Sb::Ready).await;
    b.drain().await;
    sleep(5.0).await;
    a.drain().await;
    b.drain().await;
}

async fn relogin(w: &World, id: i32, name: &str) -> (Client, Vec<String>) {
    let mut c = w.connect().await;
    c.send(Sb::Authenticate {
        token: format!("token-{id}-{name}"),
    })
    .await;
    let lines = c.lines().await;
    (c, lines)
}

#[tokio::test(start_paused = true)]
async fn g02_disconnect_during_token_check_does_not_lock_the_user_out() {
    let w = World::new().await;
    w.upstream.user("tok", 100, "Alice");
    let gate = Arc::new(Gate::default());
    *w.upstream.token_gate.lock().unwrap() = Some(gate.clone());
    let mut c = w.connect().await;
    c.send(Sb::Authenticate {
        token: "tok".into(),
    })
    .await;
    gate.wait_reached().await;
    c.shutdown().await;
    settle().await;
    gate.open();
    settle().await;
    assert_eq!(w.ctx.registered_count(), 0, "the dead login was torn down");

    *w.upstream.token_gate.lock().unwrap() = None;
    let mut c = w.connect().await;
    c.send(Sb::Authenticate {
        token: "tok".into(),
    })
    .await;
    let lines = c.lines().await;
    assert_eq!(lines[0], "Authenticate ok 100 Alice room=-");
    assert_eq!(w.logs.count("has logged in as [100] Alice"), 1);
}

#[tokio::test(start_paused = true)]
async fn g02_resume_that_dies_during_the_token_check_leaves_the_player_suspended() {
    let w = World::new().await;
    let (mut a, b) = pair(&w).await;
    b.shutdown().await;
    settle().await;
    a.drain().await;

    let gate = Arc::new(Gate::default());
    *w.upstream.token_gate.lock().unwrap() = Some(gate.clone());
    let mut c = w.connect().await;
    c.send(Sb::Authenticate {
        token: "token-200-Bob".into(),
    })
    .await;
    gate.wait_reached().await;
    c.shutdown().await;
    settle().await;
    gate.open();
    settle().await;
    *w.upstream.token_gate.lock().unwrap() = None;
    assert_eq!(w.ctx.player_room(200).as_deref(), Some("r1"));
    let (_b, lines) = relogin(&w, 200, "Bob").await;
    assert_eq!(
        lines[0],
        "Authenticate ok 200 Bob room=r1:SelectChart:100,200"
    );
}

#[tokio::test(start_paused = true)]
async fn g02_dead_login_does_not_kick_the_live_session() {
    let w = World::new().await;
    let mut a = w.login(100, "Alice").await;
    let gate = Arc::new(Gate::default());
    *w.upstream.token_gate.lock().unwrap() = Some(gate.clone());
    let mut c = w.connect().await;
    c.send(Sb::Authenticate {
        token: "token-100-Alice".into(),
    })
    .await;
    gate.wait_reached().await;
    c.shutdown().await;
    settle().await;
    gate.open();
    settle().await;
    a.send(Sb::Ping).await;
    assert_eq!(a.lines().await, s(&["Pong"]));
    assert_eq!(w.ctx.online_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn g02_simultaneous_drops_in_the_ready_phase_leave_everyone_resumable() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    let mut m = w.login(300, "Carol").await;
    join(&mut m, "r1", true).await;
    into_ready(&mut a, &mut b).await;
    a.send(Sb::Ready).await;
    a.drain().await;
    m.drain().await;
    let (ca, cb, cm) = (a, b, m);
    ca.shutdown().await;
    cb.shutdown().await;
    cm.shutdown().await;
    settle().await;
    let view = w.ctx.room("r1").unwrap();
    let players: Vec<i32> = view.players.iter().map(|p| p.id).collect();
    assert_eq!(players, [100, 200]);
    assert!(view.monitors.is_empty());
    assert_eq!(w.ctx.registered_count(), 2);
    for (id, name) in [(100, "Alice"), (200, "Bob")] {
        let (_c, lines) = relogin(&w, id, name).await;
        assert!(
            lines[0].starts_with(&format!("Authenticate ok {id} {name} room=r1:")),
            "{lines:?}"
        );
    }
    let (_m, lines) = relogin(&w, 300, "Carol").await;
    assert_eq!(lines[0], "Authenticate ok 300 Carol room=-");
}

#[tokio::test(start_paused = true)]
async fn g03_slow_upstream_blocks_nothing_else() {
    let w = World::new().await;
    w.upstream.record(game_record(
        9002,
        200,
        1,
        950_000,
        0.993,
        0.0071,
        EPOCH_MILLIS + 21_000,
    ));
    let (mut a, mut b) = pair(&w).await;
    into_round(&mut a, &mut b).await;
    let gate = Arc::new(Gate::default());
    *w.upstream.record_gate.lock().unwrap() = Some(gate.clone());
    b.send(Sb::Played { id: 9002 }).await;
    gate.wait_reached().await;

    a.send(Sb::Chat {
        message: "hello".into(),
    })
    .await;
    assert_eq!(a.lines().await, s(&["[100] hello", "Chat ok"]));
    w.ctx.create_room("r2", RoomPools::Default).unwrap();
    let mut c = w.login(300, "Carol").await;
    let mut d = w.login(400, "Dave").await;
    join(&mut c, "r2", false).await;
    join(&mut d, "r2", false).await;
    c.drain().await;
    sleep(30.0).await;
    assert_eq!(c.lines().await, s(&["> 投票锁定倒计时：120 秒"]));
    d.drain().await;
    d.send(Sb::Ping).await;
    assert_eq!(d.lines().await, s(&["Pong"]));

    gate.open();
    settle().await;
    let lines = b.lines().await;
    assert!(lines.contains(&"Played ok".to_owned()), "{lines:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g04_coinciding_ready_presses_start_one_round() {
    let w = World::new().await;
    for trial in 0..150 {
        let room = format!("t{trial}");
        let (ida, idb) = (10_000 + trial * 2, 10_001 + trial * 2);
        let mut a = w.login(ida, "A").await;
        let mut b = w.login(idb, "B").await;
        a.send(Sb::CreateRoom {
            room_id: room.clone(),
        })
        .await;
        a.expect_until(|l| l == "CreateRoom ok").await;
        w.ctx
            .update_room(
                &room,
                &zphira_game::RoomUpdate {
                    all_ready_delay: Some(0),
                    ..Default::default()
                },
            )
            .unwrap();
        b.send(Sb::JoinRoom {
            room_id: room.clone(),
            monitor: false,
        })
        .await;
        let joined = b.expect_until(|l| l.starts_with("JoinRoom ")).await;
        assert!(
            joined.last().unwrap().starts_with("JoinRoom ok"),
            "{joined:?}"
        );
        a.send(Sb::RequestStart).await;
        let started = a.expect_until(|l| l.starts_with("RequestStart ")).await;
        assert_eq!(started.last().unwrap(), "RequestStart ok");
        let (ra, rb) = tokio::join!(
            async {
                a.send(Sb::Ready).await;
                a
            },
            async {
                b.send(Sb::Ready).await;
                b
            }
        );
        let (mut a, mut b) = (ra, rb);
        let mut seen = [
            a.expect_until(|l| l.starts_with("Ready ")).await,
            b.expect_until(|l| l.starts_with("Ready ")).await,
        ];
        seen[0].extend(a.barrier().await);
        seen[1].extend(b.barrier().await);
        for lines in &seen {
            assert_eq!(
                lines.iter().filter(|l| *l == "state Playing").count(),
                1,
                "trial {trial}: {lines:?}"
            );
            assert_eq!(lines.iter().filter(|l| *l == "msg StartPlaying").count(), 1);
        }
        a.shutdown().await;
        b.shutdown().await;
    }
}

#[tokio::test(start_paused = true)]
async fn g04_timer_and_packet_at_the_same_instant_transition_once() {
    for offset_ms in [-1i64, 0, 1] {
        let w = World::new().await;
        let (mut a, mut b) = pair(&w).await;
        into_ready(&mut a, &mut b).await;
        a.send(Sb::Ready).await;
        a.drain().await;
        b.drain().await;
        let wait = Duration::from_millis((60_000 - 500 - 3 + offset_ms) as u64);
        tokio::time::sleep(wait).await;
        b.send(Sb::Ready).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let lines = a.lines().await;
        assert_eq!(
            lines.iter().filter(|l| *l == "state Playing").count(),
            1,
            "offset {offset_ms}: {lines:?}"
        );
    }
    for offset_ms in [-1i64, 0, 1] {
        let w = World::new().await;
        let (mut a, mut b) = pair(&w).await;
        tokio::time::sleep(Duration::from_millis((150_000 - 4 + offset_ms) as u64)).await;
        a.send(Sb::RequestStart).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let lines = b.lines().await;
        assert_eq!(
            lines.iter().filter(|l| *l == "state WaitForReady").count(),
            1,
            "offset {offset_ms}: {lines:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn g04_no_phantom_round_end_an_hour_later() {
    let w = World::new().await;
    w.upstream.record(game_record(
        9001,
        100,
        1,
        990_000,
        0.98,
        0.01,
        EPOCH_MILLIS + 20_000,
    ));
    w.upstream.record(game_record(
        9002,
        200,
        1,
        950_000,
        0.97,
        0.01,
        EPOCH_MILLIS + 20_000,
    ));
    let (mut a, mut b) = pair(&w).await;
    into_round(&mut a, &mut b).await;
    a.send(Sb::Played { id: 9001 }).await;
    b.send(Sb::Played { id: 9002 }).await;
    a.drain().await;
    b.drain().await;
    assert_eq!(w.round_records().len(), 1);
    for _ in 0..9 {
        sleep(500.0).await;
        a.send(Sb::Ping).await;
        b.send(Sb::Ping).await;
    }
    let lines = a.lines().await;
    assert!(!lines.contains(&"msg GameEnd".to_owned()), "{lines:?}");
    assert_eq!(w.round_records().len(), 1);
    assert_eq!(w.points.summary(100, "Alice").points, 100);
    assert_eq!(
        w.ctx.room("r1").unwrap().pool.finished_rounds_since_refresh,
        1
    );
}

const INVALID_SELF: &str = "> 成绩记录无效，本轮将视为放弃。";

fn invalid_lines(name: &str, id: i32) -> (Vec<String>, Vec<String>) {
    let room = s(&[
        &format!("> {name} 提交的成绩记录无效，已视为放弃本轮。"),
        &format!("msg Abort {id}"),
    ]);
    let mut own = vec![INVALID_SELF.to_owned()];
    own.extend(room.clone());
    own.extend(s(&[
        "host true",
        "state SelectChart(1)",
        "Played failed: 成绩提交失败，请稍后重试",
    ]));
    (own, room)
}

#[tokio::test(start_paused = true)]
async fn g05_forged_or_reused_records_are_refused() {
    let w = World::new().await;
    let t = EPOCH_MILLIS + 20_000;
    w.upstream
        .record(game_record(9002, 200, 1, 950_000, 0.97, 0.01, t));
    w.upstream
        .record(game_record(8001, 424_242, 1, 1_000_000, 1.0, 0.0, t));
    w.upstream
        .record(game_record(8002, 100, 9999, 1_000_000, 1.0, 0.0, t));
    w.upstream.record(game_record(
        8003,
        100,
        1,
        1_000_000,
        1.0,
        0.0,
        EPOCH_MILLIS - 3_600_000,
    ));
    let (mut a, mut b) = pair(&w).await;

    into_round(&mut a, &mut b).await;
    a.send(Sb::Played { id: 8001 }).await;
    let (own, room) = invalid_lines("Alice", 100);
    assert_eq!(a.lines().await, own);
    assert_eq!(b.lines().await, room);
    b.send(Sb::Played { id: 9002 }).await;
    let lines = b.lines().await;
    assert!(
        lines.contains(
            &"> 1. Bob - 分数: 950000, 准度: 97.0%, 误差: ±10.0ms, 积分: +100, 总积分: 100"
                .to_owned()
        ),
        "{lines:?}"
    );
    a.drain().await;

    into_round(&mut a, &mut b).await;
    b.send(Sb::Played { id: 9002 }).await;
    let (own, room) = invalid_lines("Bob", 200);
    assert_eq!(b.lines().await, own);
    assert_eq!(a.lines().await, room);
    a.send(Sb::Played { id: 8002 }).await;
    let lines = a.lines().await;
    assert_eq!(lines[..5].to_vec(), invalid_lines("Alice", 100).0[..5]);
    b.drain().await;

    into_round(&mut a, &mut b).await;
    a.send(Sb::Played { id: 8003 }).await;
    assert_eq!(a.lines().await, invalid_lines("Alice", 100).0);
    assert!(
        w.logs
            .contains("submitted record 8001 that does not belong to this round")
    );
    assert_eq!(w.points.summary(100, "Alice").points, 0);
    assert!(
        w.round_records()
            .iter()
            .all(|r| r.results.iter().all(|x| x.score != 1_000_000))
    );
}

#[tokio::test(start_paused = true)]
async fn g06_l11_live_data_is_relayed_only_from_active_players_while_playing() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    let mut c = w.login(300, "Carol").await;
    join(&mut c, "r1", false).await;
    let mut m = w.login(400, "Mon").await;
    join(&mut m, "r1", true).await;
    for x in [&mut a, &mut b, &mut c, &mut m] {
        x.drain().await;
    }
    a.send(Sb::Touches { frames: vec![] }).await;
    settle().await;
    assert!(m.lines().await.is_empty(), "voting: nothing relayed");
    a.send(Sb::RequestStart).await;
    for x in [&mut a, &mut b, &mut c, &mut m] {
        x.drain().await;
    }
    a.send(Sb::Judges { judges: vec![] }).await;
    settle().await;
    assert!(m.lines().await.is_empty(), "ready phase: nothing relayed");
    a.send(Sb::Ready).await;
    b.send(Sb::Ready).await;
    m.send(Sb::Ready).await;
    sleep(61.0).await;
    for x in [&mut a, &mut b, &mut c, &mut m] {
        x.drain().await;
    }
    a.send(Sb::Touches { frames: vec![] }).await;
    c.send(Sb::Touches { frames: vec![] }).await;
    b.send(Sb::Judges { judges: vec![] }).await;
    settle().await;
    assert_eq!(m.lines().await, s(&["Touches 100 x0", "Judges 200 x0"]));
    assert!(
        a.lines().await.is_empty(),
        "players never receive the stream"
    );
}

#[tokio::test(start_paused = true)]
async fn g08_a_returning_player_restarts_the_countdown() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    into_round(&mut a, &mut b).await;
    a.send(Sb::Abort).await;
    a.drain().await;
    b.shutdown().await;
    settle().await;
    let lines = a.lines().await;
    assert!(lines.contains(&"msg GameEnd".to_owned()), "{lines:?}");
    assert!(!lines.contains(&COUNTDOWN_150.to_owned()));
    let (_b, lines) = relogin(&w, 200, "Bob").await;
    assert_eq!(lines.last().unwrap(), COUNTDOWN_150);
    assert_eq!(a.lines().await, s(&[COUNTDOWN_150]));
}

#[tokio::test(start_paused = true)]
async fn g11_shutdown_abandons_rounds_and_announces_nothing_unsaved() {
    let w = World::new().await;
    w.upstream.record(game_record(
        9001,
        100,
        1,
        990_000,
        0.98,
        0.01,
        EPOCH_MILLIS + 20_000,
    ));
    let (mut a, mut b) = pair(&w).await;
    let mut c = w.login(300, "Carol").await;
    c.send(Sb::CreateRoom {
        room_id: "mine".into(),
    })
    .await;
    c.drain().await;
    into_round(&mut a, &mut b).await;
    a.send(Sb::Played { id: 9001 }).await;
    a.drain().await;
    b.drain().await;

    w.ctx.shutdown().await;
    let a_lines = a.until_closed().await;
    assert!(
        a_lines.iter().all(|l| !l.contains("本轮排名")),
        "{a_lines:?}"
    );
    assert!(c.closed().await);
    assert!(w.logs.contains("INFO Kicking 3 player(s)..."));
    assert!(w.logs.contains("INFO Closing 3 channel(s)..."));
    assert!(w.logs.contains("INFO Channels closed."));
    assert_eq!(w.logs.count("(KICK)"), 3);
    assert!(w.round_records().is_empty());
    assert_eq!(w.points.summary(100, "Alice").points, 0);
    assert_eq!(
        w.ctx.room("r1").unwrap().pool.finished_rounds_since_refresh,
        0
    );
    w.ctx.mark_rooms_dirty();
    assert!(w.ctx.flush_rooms().await);
    let saved = std::fs::read_to_string(w.data().join("rooms.json")).unwrap();
    assert!(saved.contains("\"room_id\": \"mine\""), "{saved}");
    assert!(saved.contains("\"room_id\": \"r1\""), "{saved}");
}

#[tokio::test(start_paused = true)]
async fn g12_l01_lock_is_released_when_the_countdown_is_cancelled() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    a.send(Sb::SelectChart { id: 1 }).await;
    a.drain().await;
    sleep(149.2).await;
    let lines = a.lines().await;
    assert!(lines.contains(&"> 本轮曲目已锁定，无法继续改票。".to_owned()));
    b.send(Sb::LeaveRoom).await;
    b.drain().await;
    assert_eq!(
        a.lines().await,
        s(&[
            "msg LeaveRoom 200 Bob",
            HINT,
            "> 在线玩家不足，开局倒计时已取消。",
            "> 曲目锁定已解除，可继续投票。",
        ])
    );
    a.send(Sb::SelectChart { id: 2 }).await;
    assert_eq!(a.lines().await.last().unwrap(), "SelectChart ok");
    join(&mut b, "r1", false).await;
    a.drain().await;
    sleep(149.2).await;
    a.drain().await;
    b.shutdown().await;
    settle().await;
    let lines = a.lines().await;
    assert_eq!(
        lines,
        s(&[
            "> 在线玩家不足，开局倒计时已取消。",
            "> 曲目锁定已解除，可继续投票。"
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn g13_stale_suspend_timer_does_not_remove_a_resumed_player() {
    let w = World::new().await;
    let (mut a, b) = pair(&w).await;
    b.shutdown().await;
    settle().await;
    let (mut b, _) = relogin(&w, 200, "Bob").await;
    for _ in 0..8 {
        sleep(500.0).await;
        a.send(Sb::Ping).await;
        b.send(Sb::Ping).await;
    }
    assert_eq!(w.ctx.player_room(200).as_deref(), Some("r1"));
    b.drain().await;
    b.send(Sb::SelectChart { id: 1 }).await;
    assert_eq!(b.lines().await.last().unwrap(), "SelectChart ok");
}

#[tokio::test(start_paused = true)]
async fn g13_old_connection_teardown_does_not_suspend_the_new_one() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    into_ready(&mut a, &mut b).await;
    b.send(Sb::Ready).await;
    b.drain().await;
    a.drain().await;
    let (mut b2, _) = relogin(&w, 200, "Bob").await;
    assert!(b.closed().await);
    settle().await;
    let lines = a.lines().await;
    assert!(
        !lines.contains(&"msg CancelReady 200".to_owned()),
        "{lines:?}"
    );
    a.send(Sb::Ready).await;
    let lines = a.lines().await;
    assert!(
        lines.contains(&"> 全员已准备，5 秒后开始游戏。".to_owned()),
        "{lines:?}"
    );
    sleep(5.0).await;
    let lines = a.lines().await;
    assert!(lines.contains(&"state Playing".to_owned()), "{lines:?}");
    b2.drain().await;
    for _ in 0..8 {
        sleep(500.0).await;
        a.send(Sb::Ping).await;
        b2.send(Sb::Ping).await;
    }
    assert_eq!(w.ctx.player_room(200).as_deref(), Some("r1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn g14_join_racing_the_last_leave_never_lands_in_a_dead_room() {
    let w = World::new().await;
    for trial in 0..150 {
        let room = format!("g{trial}");
        let mut a = w.login(20_000 + trial * 2, "A").await;
        let mut b = w.login(20_001 + trial * 2, "B").await;
        a.send(Sb::CreateRoom {
            room_id: room.clone(),
        })
        .await;
        a.expect_until(|l| l == "CreateRoom ok").await;
        let (mut a, mut b) = tokio::join!(
            async {
                a.send(Sb::LeaveRoom).await;
                a
            },
            async {
                b.send(Sb::JoinRoom {
                    room_id: room.clone(),
                    monitor: false,
                })
                .await;
                b
            }
        );
        let lines = b.expect_until(|l| l.starts_with("JoinRoom ")).await;
        a.expect_until(|l| l.starts_with("LeaveRoom ")).await;
        let joined = lines.iter().any(|l| l.starts_with("JoinRoom ok"));
        if joined {
            assert!(
                w.ctx.room_exists(&room),
                "trial {trial}: joined a dead room"
            );
            assert_eq!(w.ctx.player_room(b.id).as_deref(), Some(room.as_str()));
        } else {
            assert!(
                lines.contains(&"JoinRoom failed: 房间不存在".to_owned()),
                "trial {trial}: {lines:?}"
            );
            assert_eq!(w.ctx.player_room(b.id), None);
        }
        a.shutdown().await;
        b.shutdown().await;
    }
}

#[tokio::test(start_paused = true)]
async fn g15_l02_ready_pressed_while_voting_can_be_cancelled() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    a.send(Sb::Ready).await;
    assert_eq!(a.lines().await, s(&["msg Ready 100", "Ready ok"]));
    a.send(Sb::CancelReady).await;
    assert_eq!(
        a.lines().await,
        s(&["msg CancelReady 100", "CancelReady ok"])
    );
    assert_eq!(
        b.lines().await,
        s(&["msg Ready 100", "msg CancelReady 100"])
    );
    into_ready(&mut a, &mut b).await;
    b.send(Sb::Ready).await;
    let lines = b.lines().await;
    assert!(!lines.contains(&"state Playing".to_owned()), "{lines:?}");
}

#[tokio::test(start_paused = true)]
async fn g15_l03_all_ready_on_entry_starts_at_once() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    a.send(Sb::Ready).await;
    b.send(Sb::Ready).await;
    a.send(Sb::SelectChart { id: 1 }).await;
    b.send(Sb::SelectChart { id: 1 }).await;
    a.drain().await;
    b.drain().await;
    sleep(10.5).await;
    let lines = a.lines().await;
    assert_eq!(
        lines,
        s(&[
            "> 投票锁定倒计时：5 秒",
            "> 投票锁定倒计时：3 秒",
            "> 投票锁定倒计时：2 秒",
            "> 投票锁定倒计时：1 秒",
            "> 本轮曲目已锁定，无法继续改票。",
            SEP,
            "state SelectChart(1)",
            "> 投票结束，本轮曲目：Alpha | Lv.IN 12 | ID:1",
            "msg Ready 100",
            "msg Ready 200",
            "state WaitForReady",
            "> 进入准备阶段：请在 60 秒内准备。未准备玩家将作为观战跳过本轮。",
            "> 全员已准备，5 秒后开始游戏。",
            "> 准备倒计时：60 秒。",
        ])
    );
    sleep(5.0).await;
    assert_eq!(a.lines().await, s(&["state Playing", "msg StartPlaying"]));
}

#[tokio::test(start_paused = true)]
async fn g15_l04_leaving_drops_the_ready_intent() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    let mut c = w.login(300, "Carol").await;
    join(&mut c, "r1", false).await;
    c.send(Sb::Ready).await;
    c.drain().await;
    c.send(Sb::LeaveRoom).await;
    c.drain().await;
    a.drain().await;
    b.drain().await;
    a.send(Sb::RequestStart).await;
    let lines = a.lines().await;
    assert!(!lines.contains(&"msg Ready 300".to_owned()), "{lines:?}");
    join(&mut c, "r1", false).await;
    a.send(Sb::Ready).await;
    b.send(Sb::Ready).await;
    a.drain().await;
    let lines = b.lines().await;
    assert!(!lines.contains(&"state Playing".to_owned()), "{lines:?}");
}

#[tokio::test(start_paused = true)]
async fn g15_disconnect_of_the_last_unready_player_starts_the_round() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    into_ready(&mut a, &mut b).await;
    a.send(Sb::Ready).await;
    a.drain().await;
    b.shutdown().await;
    settle().await;
    assert_eq!(
        a.lines().await,
        s(&["msg CancelReady 200", "> 全员已准备，5 秒后开始游戏。"])
    );
    sleep(5.0).await;
    assert_eq!(a.lines().await, s(&["state Playing", "msg StartPlaying"]));
}

#[tokio::test(start_paused = true)]
async fn g16_vote_after_a_cancelled_ready_phase_has_no_chart() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    into_ready(&mut a, &mut b).await;
    sleep(60.0).await;
    let lines = a.lines().await;
    assert!(lines.contains(&"state SelectChart".to_owned()), "{lines:?}");
    assert!(w.ctx.room("r1").unwrap().chart.is_none());
    let mut c = w.login(300, "Carol").await;
    c.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    let lines = c.lines().await;
    assert!(
        lines.contains(&"JoinRoom ok SelectChart [100,200,300] live=false".to_owned()),
        "{lines:?}"
    );
    b.shutdown().await;
    c.shutdown().await;
    settle().await;
}

#[tokio::test(start_paused = true)]
async fn l06_settlement_recovers_records_without_holding_the_room() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    into_round(&mut a, &mut b).await;
    let gate = Arc::new(Gate::default());
    *w.upstream.recent_gate.lock().unwrap() = Some(gate.clone());
    a.send(Sb::Abort).await;
    b.send(Sb::Abort).await;
    gate.wait_reached().await;
    a.drain().await;
    b.drain().await;
    let mut c = w.login(300, "Carol").await;
    c.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    let lines = c.lines().await;
    assert!(
        lines.contains(&"> 房间当前正在游戏中，请静待游戏结束".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"JoinRoom ok SelectChart [100,200,300] live=false".to_owned()));
    assert_eq!(w.ctx.room("r1").unwrap().state, "Playing");
    gate.open();
    settle().await;
    let lines = c.lines().await;
    assert!(lines.contains(&"msg GameEnd".to_owned()), "{lines:?}");
    assert!(
        w.logs
            .contains("INFO Round in r1 ended: 0 of 2 player(s) submitted a record")
    );
}

#[tokio::test(start_paused = true)]
async fn l07_token_is_masked_in_the_log() {
    let w = World::new().await;
    w.upstream.user("abcdefghijklmnopqrstuvwxyz", 100, "Alice");
    let mut c = w.connect().await;
    c.send(Sb::Authenticate {
        token: "abcdefghijklmnopqrstuvwxyz".into(),
    })
    .await;
    c.drain().await;
    assert!(
        w.logs
            .lines()
            .iter()
            .any(|l| l.contains("sent his token [ab***xyz]"))
    );
    assert!(!w.logs.contains("abcdefghij"));
}

#[tokio::test(start_paused = true)]
async fn l08_player_removed_while_suspended_can_log_in() {
    let w = World::new().await;
    let (mut a, b) = pair(&w).await;
    b.shutdown().await;
    settle().await;
    a.drain().await;
    assert_eq!(w.ctx.clear_room("r1").await, Ok(2));
    let (_b, lines) = relogin(&w, 200, "Bob").await;
    assert_eq!(lines[0], "Authenticate ok 200 Bob room=-");
}

#[tokio::test(start_paused = true)]
async fn settlement_always_reaches_a_fresh_vote() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    into_round(&mut a, &mut b).await;
    w.upstream.failing_recent.lock().unwrap().insert(100);
    let gate = Arc::new(Gate::default());
    *w.upstream.recent_gate.lock().unwrap() = Some(gate.clone());
    a.send(Sb::Abort).await;
    b.send(Sb::Abort).await;
    gate.wait_reached().await;
    sleep(31.0).await;
    let lines = a.lines().await;
    assert!(lines.contains(&"msg GameEnd".to_owned()), "{lines:?}");
    assert_eq!(w.ctx.room("r1").unwrap().state, "SelectChart");
    assert!(w.logs.contains("Could not recover record for player"));
}

#[tokio::test(start_paused = true)]
async fn delayed_wait_for_ready_is_not_sent_after_the_round_started() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    w.ctx
        .update_room(
            "r1",
            &zphira_game::RoomUpdate {
                all_ready_delay: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
    into_ready(&mut a, &mut b).await;
    let mut c = w.login(300, "Carol").await;
    c.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    c.send(Sb::Ready).await;
    a.send(Sb::Ready).await;
    b.send(Sb::Ready).await;
    sleep(0.5).await;
    let lines = c.lines().await;
    let playing = lines.iter().position(|l| l == "state Playing").unwrap();
    assert!(
        !lines[playing..].contains(&"state WaitForReady".to_owned()),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn watchdog_counts_from_the_moment_the_room_became_empty() {
    let w = World::new().await;
    let bg = w.ctx.start_background();
    w.ctx.create_room("r1", RoomPools::Default).unwrap();
    sleep(59.0 * 60.0).await;
    let mut a = w.login(100, "Alice").await;
    join(&mut a, "r1", false).await;
    a.send(Sb::LeaveRoom).await;
    a.drain().await;
    sleep(59.0 * 60.0).await;
    a.send(Sb::Ping).await;
    assert!(w.ctx.room_exists("r1"));
    sleep(90.0).await;
    assert!(!w.ctx.room_exists("r1"));
    assert!(
        w.logs
            .contains("INFO Reclaiming room r1 after 60 minutes with nobody in it")
    );
    bg.stop();
}

#[tokio::test(start_paused = true)]
async fn recovery_uses_the_clock_tolerance_and_skips_accepted_records() {
    let w = World::new().await;
    let (mut a, mut b) = pair(&w).await;
    into_round(&mut a, &mut b).await;
    let start = EPOCH_MILLIS + 11_000;
    w.upstream.recent(
        100,
        vec![game_record(
            7001,
            100,
            1,
            900_000,
            0.9,
            0.01,
            start - 4 * 60_000,
        )],
    );
    w.upstream.recent(
        200,
        vec![game_record(
            7002,
            200,
            1,
            800_000,
            0.9,
            0.01,
            start - 6 * 60_000,
        )],
    );
    a.send(Sb::Abort).await;
    b.send(Sb::Abort).await;
    settle().await;
    let lines = a.lines().await;
    assert!(
        lines.contains(&"> Alice 的成绩已从 Phira 同步：900000".to_owned()),
        "{lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("Bob 的成绩已从 Phira 同步"))
    );
    b.drain().await;
    into_round(&mut a, &mut b).await;
    a.send(Sb::Abort).await;
    b.send(Sb::Abort).await;
    settle().await;
    let lines = a.lines().await;
    assert!(
        !lines.iter().any(|l| l.contains("的成绩已从 Phira 同步")),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn suspended_player_leaves_after_the_hour_and_monitors_at_once() {
    let w = World::new().await;
    let (mut a, b) = pair(&w).await;
    let m = {
        let mut m = w.login(300, "Mon").await;
        join(&mut m, "r1", true).await;
        m
    };
    a.drain().await;
    m.shutdown().await;
    settle().await;
    assert_eq!(a.lines().await, s(&["msg LeaveRoom 300 Mon", HINT]));
    b.shutdown().await;
    settle().await;
    a.drain().await;
    for _ in 0..7 {
        sleep(500.0).await;
        a.send(Sb::Ping).await;
    }
    sleep(99.0).await;
    a.drain().await;
    assert_eq!(w.ctx.player_room(200).as_deref(), Some("r1"));
    sleep(2.0).await;
    let lines = a.lines().await;
    assert_eq!(lines, s(&["msg LeaveRoom 200 Bob", HINT]));
    assert_eq!(w.ctx.player_room(200), None);
    assert_eq!(w.ctx.registered_count(), 1);
    let (_b, lines) = relogin(&w, 200, "Bob").await;
    assert_eq!(lines[0], "Authenticate ok 200 Bob room=-");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn login_is_always_answered_under_churn() {
    let w = World::new().await;
    let mut latencies = Vec::new();
    let mut previous: Option<Client> = None;
    for i in 0..300 {
        let id = 30_000 + i;
        let token = format!("churn-{id}");
        w.upstream.user(&token, id, "C");
        let mut c = w.connect().await;
        let sent = std::time::Instant::now();
        c.send(Sb::Authenticate { token }).await;
        let reply = c.expect_next().await;
        latencies.push(sent.elapsed());
        assert!(reply.starts_with("Authenticate ok"), "{reply}");
        c.expect_count(6).await;
        if let Some(p) = previous.take() {
            p.shutdown().await;
        }
        previous = Some(c);
    }
    latencies.sort();
    let over_1ms = latencies
        .iter()
        .filter(|d| **d > Duration::from_millis(1))
        .count();
    eprintln!(
        "login answer latency over 300 logins: median {:?}, max {:?}, {} over 1 ms",
        latencies[150], latencies[299], over_1ms
    );
}
