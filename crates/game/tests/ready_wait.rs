mod support;

use std::time::Duration;

use support::*;
use zphira_game::{RemoveRoomError, RoomPools, RoomUpdate, UpdateRoomError};
use zphira_protocol::ServerBoundPacket as Sb;

const WAIT_5: &str = "> 全员已准备，5 秒后开始游戏。";
const CALLED_OFF: &str = "> 有玩家尚未准备，开始已取消。";

async fn sleep(secs: f64) {
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
}

async fn join(c: &mut Client, monitor: bool) {
    c.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor,
    })
    .await;
    c.drain().await;
}

async fn room(w: &World, players: &[(i32, &str)], monitors: &[(i32, &str)]) -> Vec<Client> {
    w.ctx.create_room("r1", RoomPools::Default).unwrap();
    let mut clients = Vec::new();
    for (id, name) in players {
        let mut c = w.login(*id, name).await;
        join(&mut c, false).await;
        clients.push(c);
    }
    for (id, name) in monitors {
        let mut c = w.login(*id, name).await;
        join(&mut c, true).await;
        clients.push(c);
    }
    for c in &mut clients {
        c.drain().await;
    }
    clients
}

fn update(w: &World, u: RoomUpdate) {
    w.ctx.update_room("r1", &u).unwrap();
}

async fn to_ready(clients: &mut [Client]) {
    clients[0].send(Sb::SelectChart { id: 1 }).await;
    clients[0].send(Sb::RequestStart).await;
    for c in clients.iter_mut() {
        c.drain().await;
    }
}

async fn drain_all(clients: &mut [Client]) {
    for c in clients.iter_mut() {
        c.drain().await;
    }
}

fn count(lines: &[String], line: &str) -> usize {
    lines.iter().filter(|l| *l == line).count()
}

#[tokio::test(start_paused = true)]
async fn o03_cancel_ready_in_vote_on_and_off() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
    c[0].send(Sb::Ready).await;
    c[0].send(Sb::CancelReady).await;
    assert_eq!(
        c[0].lines().await,
        s(&[
            "msg Ready 100",
            "Ready ok",
            "msg CancelReady 100",
            "CancelReady ok"
        ])
    );
    assert_eq!(
        c[1].lines().await,
        s(&["msg Ready 100", "msg CancelReady 100"])
    );

    update(
        &w,
        RoomUpdate {
            cancel_ready_in_vote: Some(false),
            ..RoomUpdate::default()
        },
    );
    assert!(!w.ctx.room("r1").unwrap().config.cancel_ready_in_vote);
    c[1].send(Sb::Ready).await;
    c[1].send(Sb::CancelReady).await;
    assert_eq!(
        c[1].lines().await,
        s(&[
            "msg Ready 200",
            "Ready ok",
            "CancelReady failed: 你不能在当前状态执行这个操作"
        ])
    );
    assert_eq!(c[0].lines().await, s(&["msg Ready 200"]));
    c[0].send(Sb::RequestStart).await;
    let lines = c[0].lines().await;
    assert!(lines.contains(&"msg Ready 200".to_owned()), "{lines:?}");
    c[1].drain().await;
    c[1].send(Sb::CancelReady).await;
    assert_eq!(
        c[1].lines().await,
        s(&["msg CancelReady 200", "CancelReady ok"])
    );
}

#[tokio::test(start_paused = true)]
async fn o03_wait_after_the_last_ready_press_keeps_the_countdown_running() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
    to_ready(&mut c).await;
    c[0].send(Sb::Ready).await;
    drain_all(&mut c).await;
    sleep(27.0).await;
    drain_all(&mut c).await;
    c[1].send(Sb::Ready).await;
    assert_eq!(
        c[1].lines().await,
        s(&["msg Ready 200", WAIT_5, "Ready ok"])
    );
    assert_eq!(c[0].lines().await, s(&["msg Ready 200", WAIT_5]));
    sleep(3.0).await;
    assert_eq!(c[0].lines().await, s(&["> 准备倒计时：30 秒。"]));
    sleep(1.9).await;
    assert!(c[0].lines().await.is_empty());
    sleep(0.1).await;
    assert_eq!(
        c[0].lines().await,
        s(&["state Playing", "msg StartPlaying"])
    );
    sleep(40.0).await;
    let lines = c[0].lines().await;
    assert_eq!(count(&lines, "state Playing"), 0, "{lines:?}");
}

#[tokio::test(start_paused = true)]
async fn o03_wait_on_entering_the_ready_phase() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
    c[0].send(Sb::Ready).await;
    c[1].send(Sb::Ready).await;
    drain_all(&mut c).await;
    c[0].send(Sb::SelectChart { id: 1 }).await;
    c[0].send(Sb::RequestStart).await;
    let lines = c[0].lines().await;
    let at = lines.iter().position(|l| l == WAIT_5).expect("announced");
    assert_eq!(
        lines[at - 1],
        "> 进入准备阶段：请在 60 秒内准备。未准备玩家将作为观战跳过本轮。"
    );
    sleep(5.0).await;
    assert_eq!(
        c[0].lines().await,
        s(&["state Playing", "msg StartPlaying"])
    );
}

#[tokio::test(start_paused = true)]
async fn o03_wait_after_a_disconnect() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
    to_ready(&mut c).await;
    c[0].send(Sb::Ready).await;
    drain_all(&mut c).await;
    let b = c.pop().unwrap();
    b.shutdown().await;
    settle().await;
    assert_eq!(c[0].lines().await, s(&["msg CancelReady 200", WAIT_5]));
    sleep(5.0).await;
    assert_eq!(
        c[0].lines().await,
        s(&["state Playing", "msg StartPlaying"])
    );
}

#[tokio::test(start_paused = true)]
async fn o03_call_off_by_a_cancel_then_a_new_wait() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
    to_ready(&mut c).await;
    c[0].send(Sb::Ready).await;
    c[1].send(Sb::Ready).await;
    assert_eq!(
        c[1].lines().await,
        s(&["msg Ready 100", "msg Ready 200", WAIT_5, "Ready ok"])
    );
    c[0].drain().await;
    sleep(2.0).await;
    c[0].send(Sb::CancelReady).await;
    assert_eq!(
        c[0].lines().await,
        s(&["msg CancelReady 100", CALLED_OFF, "CancelReady ok"])
    );
    assert_eq!(c[1].lines().await, s(&["msg CancelReady 100", CALLED_OFF]));
    sleep(4.0).await;
    assert!(c[1].lines().await.is_empty());
    c[0].send(Sb::Ready).await;
    assert_eq!(c[1].lines().await, s(&["msg Ready 100", WAIT_5]));
    sleep(5.0).await;
    let lines = c[1].lines().await;
    assert_eq!(lines, s(&["state Playing", "msg StartPlaying"]));
}

#[tokio::test(start_paused = true)]
async fn o03_call_off_by_a_join_of_a_player_who_is_not_ready() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
    to_ready(&mut c).await;
    c[0].send(Sb::Ready).await;
    c[1].send(Sb::Ready).await;
    drain_all(&mut c).await;
    let mut carol = w.login(300, "Carol").await;
    carol
        .send(Sb::JoinRoom {
            room_id: "r1".into(),
            monitor: false,
        })
        .await;
    let lines = carol.lines().await;
    assert!(lines.contains(&CALLED_OFF.to_owned()), "{lines:?}");
    assert_eq!(
        c[0].lines().await,
        s(&[
            "OnJoinRoom 300 Carol",
            "msg JoinRoom 300 Carol",
            "> 欢迎 Carol（#300） 加入房间",
            CALLED_OFF,
        ])
    );
    sleep(6.0).await;
    let lines = c[0].lines().await;
    assert!(!lines.contains(&"state Playing".to_owned()), "{lines:?}");
    carol.send(Sb::Ready).await;
    assert_eq!(c[0].lines().await, s(&["msg Ready 300", WAIT_5]));
}

#[tokio::test(start_paused = true)]
async fn o03_call_off_by_the_resume_of_a_player_who_is_not_ready() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B"), (300, "C")], &[]).await;
    to_ready(&mut c).await;
    c[0].send(Sb::Ready).await;
    c[1].send(Sb::Ready).await;
    drain_all(&mut c).await;
    let carol = c.pop().unwrap();
    carol.shutdown().await;
    settle().await;
    assert_eq!(c[0].lines().await, s(&["msg CancelReady 300", WAIT_5]));
    sleep(2.0).await;
    let mut carol = w.connect().await;
    carol
        .send(Sb::Authenticate {
            token: "token-300-C".into(),
        })
        .await;
    let lines = carol.lines().await;
    assert_eq!(
        lines[0],
        "Authenticate ok 300 C room=r1:SelectChart(1):100,200,300"
    );
    assert_eq!(lines.last().unwrap(), CALLED_OFF);
    assert_eq!(c[0].lines().await, s(&[CALLED_OFF]));
    sleep(4.0).await;
    let lines = c[0].lines().await;
    assert!(!lines.contains(&"state Playing".to_owned()), "{lines:?}");
}

#[tokio::test(start_paused = true)]
async fn o03_leaver_and_dropper_during_the_wait_do_not_restart_it() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B"), (300, "C"), (400, "D")], &[]).await;
    to_ready(&mut c).await;
    for x in c.iter_mut() {
        x.send(Sb::Ready).await;
    }
    drain_all(&mut c).await;
    sleep(1.0).await;
    c[2].send(Sb::LeaveRoom).await;
    let d = c.pop().unwrap();
    d.shutdown().await;
    settle().await;
    assert_eq!(
        c[0].lines().await,
        s(&["msg LeaveRoom 300 C", "msg CancelReady 400"])
    );
    sleep(3.9).await;
    assert!(c[0].lines().await.is_empty());
    sleep(0.1).await;
    assert_eq!(
        c[0].lines().await,
        s(&["state Playing", "msg StartPlaying"])
    );
}

#[tokio::test(start_paused = true)]
async fn o03_countdown_end_during_the_wait_starts_once() {
    for (delay, ready_at) in [(30, 8.0), (5, 5.0)] {
        let w = World::new().await;
        let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
        update(
            &w,
            RoomUpdate {
                ready_countdown: Some(10),
                all_ready_delay: Some(delay),
                ..RoomUpdate::default()
            },
        );
        to_ready(&mut c).await;
        c[0].send(Sb::Ready).await;
        drain_all(&mut c).await;
        sleep(ready_at - 0.002).await;
        c[1].send(Sb::Ready).await;
        let lines = c[0].lines().await;
        assert!(
            lines.contains(&format!("> 全员已准备，{delay} 秒后开始游戏。")),
            "{lines:?}"
        );
        sleep(40.0).await;
        let lines = c[0].lines().await;
        assert_eq!(
            count(&lines, "state Playing"),
            1,
            "delay {delay}: {lines:?}"
        );
        assert_eq!(count(&lines, "msg StartPlaying"), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn o03_delay_zero_starts_at_once_without_announcement() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
    update(
        &w,
        RoomUpdate {
            all_ready_delay: Some(0),
            ..RoomUpdate::default()
        },
    );
    to_ready(&mut c).await;
    c[0].send(Sb::Ready).await;
    drain_all(&mut c).await;
    c[1].send(Sb::Ready).await;
    assert_eq!(
        c[1].lines().await,
        s(&[
            "msg Ready 200",
            "state Playing",
            "msg StartPlaying",
            "Ready ok"
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn o03_start_when_all_ready_off_runs_the_full_countdown() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[]).await;
    update(
        &w,
        RoomUpdate {
            start_when_all_ready: Some(false),
            ..RoomUpdate::default()
        },
    );
    c[0].send(Sb::Ready).await;
    c[1].send(Sb::Ready).await;
    drain_all(&mut c).await;
    to_ready(&mut c).await;
    sleep(59.9).await;
    let lines = c[0].lines().await;
    assert!(!lines.contains(&WAIT_5.to_owned()), "{lines:?}");
    assert!(!lines.contains(&"state Playing".to_owned()), "{lines:?}");
    sleep(0.2).await;
    assert_eq!(
        c[0].lines().await,
        s(&["state Playing", "msg StartPlaying"])
    );
}

#[tokio::test(start_paused = true)]
async fn o03_update_validation_and_defaults() {
    let w = World::new().await;
    w.ctx.create_room("r1", RoomPools::Default).unwrap();
    let config = w.ctx.room("r1").unwrap().config;
    assert!(config.cancel_ready_in_vote && config.start_when_all_ready);
    assert_eq!(config.all_ready_delay, 5);
    for bad in [-1, 301] {
        let refused = w.ctx.update_room(
            "r1",
            &RoomUpdate {
                live: Some(true),
                start_when_all_ready: Some(false),
                all_ready_delay: Some(bad),
                ..RoomUpdate::default()
            },
        );
        assert_eq!(refused, Err(UpdateRoomError::AllReadyDelayOutOfRange));
    }
    let view = w.ctx.room("r1").unwrap();
    assert!(
        !view.live && view.config.start_when_all_ready,
        "nothing applied"
    );
    assert_eq!(
        UpdateRoomError::AllReadyDelayOutOfRange.http_message(),
        "allReadyDelay 必须在 0 到 300 之间"
    );
    assert_eq!(
        UpdateRoomError::AllReadyDelayOutOfRange.console_message("r1"),
        "allReadyDelay must be between 0 and 300"
    );
    for ok in [0, 300] {
        update(
            &w,
            RoomUpdate {
                all_ready_delay: Some(ok),
                cancel_ready_in_vote: Some(false),
                ..RoomUpdate::default()
            },
        );
        assert_eq!(w.ctx.room("r1").unwrap().config.all_ready_delay, ok);
    }
    let json = serde_json::to_string(&w.ctx.room("r1").unwrap()).unwrap();
    assert!(
        json.contains(r#""chatEnable":true,"cancelReadyInVote":false,"startWhenAllReady":true,"allReadyDelay":300}"#),
        "{json}"
    );
}

#[tokio::test(start_paused = true)]
async fn o10_monitors_do_not_count_and_all_watch() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[(900, "M1"), (901, "M2")]).await;
    to_ready(&mut c).await;
    c[2].send(Sb::Ready).await;
    assert_eq!(c[2].lines().await, s(&["msg Ready 900", "Ready ok"]));
    assert_eq!(c[0].lines().await, s(&["msg Ready 900"]));
    c[0].send(Sb::Ready).await;
    drain_all(&mut c).await;
    c[1].send(Sb::Ready).await;
    let lines = c[3].lines().await;
    assert_eq!(lines, s(&["msg Ready 200", WAIT_5]), "M2 never readied");
    c[2].drain().await;
    drain_all(&mut c[..2]).await;
    sleep(5.0).await;
    let ready_monitor = c[2].lines().await;
    let unready_monitor = c[3].lines().await;
    assert_eq!(ready_monitor, s(&["state Playing", "msg StartPlaying"]));
    assert_eq!(unready_monitor, ready_monitor);
    drain_all(&mut c).await;
    c[0].send(Sb::Touches { frames: vec![] }).await;
    assert_eq!(c[2].lines().await, s(&["Touches 100 x0"]));
    assert_eq!(c[3].lines().await, s(&["Touches 100 x0"]));
}

#[tokio::test(start_paused = true)]
async fn o10_monitors_never_call_the_wait_off() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[(900, "M1")]).await;
    to_ready(&mut c).await;
    c[2].send(Sb::Ready).await;
    c[0].send(Sb::Ready).await;
    c[1].send(Sb::Ready).await;
    drain_all(&mut c).await;
    c[2].send(Sb::CancelReady).await;
    assert_eq!(c[0].lines().await, s(&["msg CancelReady 900"]));
    let mut m2 = w.login(901, "M2").await;
    join(&mut m2, true).await;
    let mut m1 = w.connect().await;
    m1.send(Sb::Authenticate {
        token: "token-900-M1".into(),
    })
    .await;
    let lines = m1.lines().await;
    assert!(
        lines[0].starts_with("Authenticate ok 900 M1 room=r1:"),
        "{lines:?}"
    );
    assert!(!lines.contains(&CALLED_OFF.to_owned()), "{lines:?}");
    let lines = c[0].lines().await;
    assert!(!lines.contains(&CALLED_OFF.to_owned()), "{lines:?}");
    sleep(5.0).await;
    let lines = c[0].lines().await;
    assert_eq!(lines, s(&["state Playing", "msg StartPlaying"]));
    let lines = m2.lines().await;
    assert_eq!(
        lines,
        // M2 joined the ready phase: the 100 ms WaitForReady re-send comes first.
        s(&["state WaitForReady", "state Playing", "msg StartPlaying"])
    );
}

#[tokio::test(start_paused = true)]
async fn o10_only_monitors_online_never_start() {
    let w = World::new().await;
    let mut c = room(&w, &[(100, "A"), (200, "B")], &[(900, "M1")]).await;
    to_ready(&mut c).await;
    c[2].send(Sb::Ready).await;
    drain_all(&mut c).await;
    let m = c.pop().unwrap();
    let b = c.pop().unwrap();
    let a = c.pop().unwrap();
    let mut m = m;
    a.shutdown().await;
    b.shutdown().await;
    settle().await;
    let lines = m.lines().await;
    assert!(
        lines.contains(&"> 当前没有在线玩家，本轮准备已取消。".to_owned()),
        "{lines:?}"
    );
    assert!(!lines.contains(&"state Playing".to_owned()));
    assert!(!lines.iter().any(|l| l.starts_with("> 全员已准备")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn o09_remove_racing_a_join_never_both_succeed() {
    let mut setup = Setup::default();
    setup.args.max_rooms = 1000;
    let w = World::with(setup).await;
    let (mut removed, mut joined) = (0, 0);
    for trial in 0..400 {
        let room = format!("x{trial}");
        w.ctx.create_room(&room, RoomPools::Default).unwrap();
        let mut b = w.login(40_000 + trial, "B").await;
        let early = (trial % 3 == 0).then(|| w.ctx.remove_room(&room));
        b.send(Sb::JoinRoom {
            room_id: room.clone(),
            monitor: false,
        })
        .await;
        let mut answered = None;
        if trial % 3 == 1 {
            answered = Some(b.expect_until(|l| l.starts_with("JoinRoom ")).await);
        }
        let ctx = w.ctx.clone();
        let target = room.clone();
        let spins = trial % 40;
        let remover = tokio::spawn(async move {
            if let Some(early) = early {
                return early;
            }
            for _ in 0..spins {
                tokio::task::yield_now().await;
            }
            ctx.remove_room(&target)
        });
        let lines = match answered {
            Some(lines) => lines,
            None => b.expect_until(|l| l.starts_with("JoinRoom ")).await,
        };
        let removal = remover.await.unwrap();
        let join_ok = lines.last().unwrap().starts_with("JoinRoom ok");
        match removal {
            Ok(()) => {
                removed += 1;
                assert!(!join_ok, "trial {trial}: removed and joined: {lines:?}");
                assert_eq!(lines.last().unwrap(), "JoinRoom failed: 房间不存在");
                assert_eq!(w.ctx.player_room(b.id), None);
            }
            Err(RemoveRoomError::NotEmpty) => {
                joined += 1;
                assert!(join_ok, "trial {trial}: {lines:?}");
                assert!(w.ctx.room_exists(&room));
            }
            Err(other) => panic!("trial {trial}: {other:?}"),
        }
        b.shutdown().await;
    }
    eprintln!("O-09 race: {removed} removals won, {joined} joins won");
    assert!(removed > 0 && joined > 0, "both orders occurred");
}
