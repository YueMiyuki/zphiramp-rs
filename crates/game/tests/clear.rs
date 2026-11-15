mod support;

use std::time::Duration;

use support::*;
use zphira_game::{RoomPools, RoomUpdate};
use zphira_protocol::ServerBoundPacket as Sb;

const CLEARED: &str = "> 房间已被清空，你已被移出房间。";
const HINT: &str = "> 点击锁定房间按钮查看当前谱池状态。";

async fn join(c: &mut Client, room: &str, monitor: bool) {
    c.send(Sb::JoinRoom {
        room_id: room.into(),
        monitor,
    })
    .await;
    c.drain().await;
}

async fn crowded(w: &World) -> (Client, Client, Client) {
    let mut a = w.login(100, "Alice").await;
    a.send(Sb::CreateRoom {
        room_id: "r1".into(),
    })
    .await;
    a.drain().await;
    let mut b = w.login(200, "Bob").await;
    join(&mut b, "r1", false).await;
    let mut c = w.login(300, "Carol").await;
    join(&mut c, "r1", false).await;
    let mut m = w.login(400, "Mon").await;
    join(&mut m, "r1", true).await;
    c.shutdown().await;
    settle().await;
    a.drain().await;
    b.drain().await;
    m.drain().await;
    (a, b, m)
}

async fn relogin_room(w: &World, id: i32, name: &str) -> String {
    let mut c = w.connect().await;
    c.send(Sb::Authenticate {
        token: format!("token-{id}-{name}"),
    })
    .await;
    c.lines().await.remove(0)
}

#[tokio::test(start_paused = true)]
async fn host_clears_the_room_in_game() {
    let w = World::new().await;
    let (mut a, mut b, mut m) = crowded(&w).await;
    a.send(Sb::Chat {
        message: "  /CLEAR \t".into(),
    })
    .await;
    assert_eq!(
        a.lines().await,
        s(&[
            "msg LeaveRoom 200 Bob",
            HINT,
            "> 在线玩家不足，开局倒计时已取消。",
            "msg LeaveRoom 300 Carol",
            HINT,
            "msg LeaveRoom 400 Mon",
            HINT,
            "Chat ok",
        ])
    );
    assert_eq!(b.until_closed().await, s(&[CLEARED]));
    assert_eq!(m.until_closed().await, s(&[CLEARED]));
    settle().await;
    assert_eq!(w.logs.count("(KICK)"), 2);
    assert!(
        w.logs
            .contains("INFO Room r1 cleared by Alice (#100): 3 member(s) removed")
    );
    let view = w.ctx.room("r1").unwrap();
    assert_eq!(view.players.len(), 1);
    assert!(view.monitors.is_empty());
    assert_eq!(view.host, Some(100));
    for (id, name) in [(200, "Bob"), (300, "Carol"), (400, "Mon")] {
        assert_eq!(
            relogin_room(&w, id, name).await,
            format!("Authenticate ok {id} {name} room=-")
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_player_who_does_not_control_the_room_is_refused() {
    let w = World::new().await;
    let (mut a, mut b, _m) = crowded(&w).await;
    b.send(Sb::Chat {
        message: "/clear".into(),
    })
    .await;
    assert_eq!(b.lines().await, s(&["Chat failed: 你不是房主"]));
    assert!(a.lines().await.is_empty(), "the command is never broadcast");
    assert_eq!(w.ctx.room("r1").unwrap().players.len(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_room_admin_clears_and_the_host_goes_too() {
    let w = World::new().await;
    let (mut a, mut b, _m) = crowded(&w).await;
    w.ctx
        .update_room(
            "r1",
            &RoomUpdate {
                admin_ids: Some(vec![200]),
                chat_enable: Some(false),
                ..RoomUpdate::default()
            },
        )
        .unwrap();
    b.send(Sb::Chat {
        message: "hi".into(),
    })
    .await;
    assert_eq!(b.lines().await, s(&["Chat failed: 房间未启用聊天"]));
    b.send(Sb::Chat {
        message: "/clear".into(),
    })
    .await;
    let lines = b.lines().await;
    assert_eq!(
        lines[0], "host true",
        "Bob is promoted when the host leaves: {lines:?}"
    );
    assert_eq!(lines[1], "msg LeaveRoom 100 Alice");
    assert_eq!(lines.last().unwrap(), "Chat ok");
    assert_eq!(a.until_closed().await, s(&[CLEARED]));
    assert_eq!(w.ctx.room("r1").unwrap().host, Some(200));
}

#[tokio::test(start_paused = true)]
async fn clearing_during_the_ready_phase_and_the_round() {
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
    let mut a = w.login(100, "Alice").await;
    a.send(Sb::CreateRoom {
        room_id: "r1".into(),
    })
    .await;
    a.drain().await;
    let mut b = w.login(200, "Bob").await;
    join(&mut b, "r1", false).await;
    a.drain().await;
    w.upstream.record(game_record(
        9003,
        100,
        1,
        990_000,
        0.98,
        0.01,
        EPOCH_MILLIS + 20_000,
    ));
    a.send(Sb::SelectChart { id: 1 }).await;
    a.send(Sb::RequestStart).await;
    a.send(Sb::Ready).await;
    a.drain().await;
    b.drain().await;
    a.send(Sb::Chat {
        message: "/clear".into(),
    })
    .await;
    let lines = a.lines().await;
    assert_eq!(
        lines,
        s(&[
            "msg LeaveRoom 200 Bob",
            "> 全员已准备，5 秒后开始游戏。",
            "Chat ok"
        ])
    );
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert_eq!(a.lines().await, s(&["state Playing", "msg StartPlaying"]));
    assert!(b.closed().await);

    a.send(Sb::Played { id: 9001 }).await;
    let lines = a.lines().await;
    assert!(lines.contains(&"msg GameEnd".to_owned()), "{lines:?}");
    let mut c = w.login(300, "Carol").await;
    join(&mut c, "r1", false).await;
    a.drain().await;
    a.send(Sb::SelectChart { id: 1 }).await;
    a.send(Sb::RequestStart).await;
    a.send(Sb::Ready).await;
    a.drain().await;
    c.send(Sb::Ready).await;
    a.drain().await;
    c.drain().await;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    a.drain().await;
    c.drain().await;
    a.send(Sb::Played { id: 9003 }).await;
    a.drain().await;
    a.send(Sb::Chat {
        message: "/clear".into(),
    })
    .await;
    let lines = a.lines().await;
    assert_eq!(lines[0], "msg LeaveRoom 300 Carol");
    assert!(lines.contains(&"msg GameEnd".to_owned()), "{lines:?}");
    assert_eq!(lines.last().unwrap(), "Chat ok");
    assert!(
        w.logs
            .contains("INFO Round in r1 ended: 1 of 2 player(s) submitted a record"),
        "{:?}",
        w.logs.lines()
    );
}

#[tokio::test(start_paused = true)]
async fn api_clear_removes_everyone_and_destroys_an_auto_destroy_room() {
    let w = World::new().await;
    let (a, b, m) = crowded(&w).await;
    assert_eq!(w.ctx.clear_room("r1").await, Ok(4));
    for mut c in [a, b, m] {
        assert_eq!(c.until_closed().await, s(&[CLEARED]));
    }
    settle().await;
    assert!(
        !w.ctx.room_exists("r1"),
        "an autoDestroy room ends with its last member"
    );
    assert_eq!(w.ctx.registered_count(), 0);
    assert_eq!(w.ctx.clear_room("r1").await, Err(zphira_game::RoomNotFound));
}

#[tokio::test(start_paused = true)]
async fn api_clear_of_an_operator_room_keeps_the_room() {
    let w = World::new().await;
    w.ctx.create_room("op", RoomPools::Default).unwrap();
    let mut a = w.login(100, "Alice").await;
    join(&mut a, "op", false).await;
    let mut b = w.login(200, "Bob").await;
    join(&mut b, "op", true).await;
    a.drain().await;
    assert_eq!(w.ctx.clear_room("op").await, Ok(2));
    assert_eq!(a.until_closed().await, s(&[CLEARED]));
    assert!(w.ctx.room_exists("op"));
    assert!(w.ctx.room("op").unwrap().players.is_empty());
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(w.ctx.online_count(), 0);
}
