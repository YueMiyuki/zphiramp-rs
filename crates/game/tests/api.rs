mod support;

use std::time::Duration;

use support::*;
use zphira_game::{
    CreateRoomError, RemoveRoomError, RoomPools, RoomUpdate, SwitchPoolError, UpdateRoomError,
};
use zphira_protocol::ServerBoundPacket as Sb;

async fn join(c: &mut Client, room: &str, monitor: bool) {
    c.send(Sb::JoinRoom {
        room_id: room.into(),
        monitor,
    })
    .await;
    c.drain().await;
}

#[tokio::test(start_paused = true)]
async fn room_view_serializes_like_room_snapshot() {
    let w = World::new().await;
    w.ctx
        .create_room("r1", RoomPools::Resolve(vec![1, 2]))
        .unwrap();
    let json = serde_json::to_string(&w.ctx.room("r1").unwrap()).unwrap();
    assert_eq!(
        json,
        concat!(
            r#"{"roomId":"r1","state":"SelectChart","live":false,"locked":false,"cycle":false,"host":null,"chart":null,"type":"local","#,
            r#""config":{"minPlayer":2,"maxPlayer":1000,"selectCountdown":150,"readyCountdown":60,"forceFinish":3600,"interval":5,"adminIds":[],"chatEnable":true,"cancelReadyInVote":true,"startWhenAllReady":true,"allReadyDelay":5},"#,
            r#""effectiveForceFinish":3600,"#,
            r#""pool":{"currentPool":{"id":1,"favoriteId":null,"default":true,"chartIds":[1,2],"category":"MANUAL","sizeLimit":null,"roundsPerStay":null,"order":0,"submissionOpen":false},"#,
            r#""pools":[{"id":1,"favoriteId":null,"default":true,"chartIds":[1,2],"category":"MANUAL","sizeLimit":null,"roundsPerStay":null,"order":0,"submissionOpen":false},"#,
            r#"{"id":2,"favoriteId":null,"default":false,"chartIds":[3,4],"category":"MANUAL","sizeLimit":null,"roundsPerStay":null,"order":1,"submissionOpen":false}],"#,
            r#""pendingPoolId":null,"favoriteId":null,"finishedRoundsSinceRefresh":0,"refreshIntervalRounds":5,"effectiveRoundsPerStay":5},"#,
            r#""players":[],"monitors":[]}"#
        )
    );

    let mut b = w.login(200, "Bob").await;
    let mut a = w.login(100, "Alice").await;
    join(&mut b, "r1", false).await;
    join(&mut a, "r1", false).await;
    let mut m = w.login(50, "Mon").await;
    join(&mut m, "r1", true).await;
    a.send(Sb::SelectChart { id: 1 }).await;
    a.drain().await;
    b.send(Sb::RequestStart).await;
    b.drain().await;
    let view = w.ctx.room("r1").unwrap();
    assert_eq!(view.state, "WaitForReady");
    assert_eq!(view.host, Some(200));
    let json = serde_json::to_string(&view).unwrap();
    assert!(json.contains(r#""chart":{"id":1,"name":"Alpha","level":"IN 12","difficulty":0.0,"charter":null,"composer":null,"illustrator":null,"description":null,"ranked":false,"reviewed":false,"stable":false,"stableRequest":false,"illustration":null,"preview":null,"file":null,"uploader":0,"tags":[],"rating":0.0,"ratingCount":0,"created":null,"updated":null,"chartUpdated":null,"durationSeconds":null}"#), "{json}");
    assert!(json.ends_with(r#""players":[{"id":100,"name":"Alice"},{"id":200,"name":"Bob"}],"monitors":[{"id":50,"name":"Mon"}]}"#), "{json}");
    assert_eq!(view.online_players, 2);
    assert_eq!(w.ctx.room_pool("r1").unwrap().current_pool.id, 1);
    assert_eq!(w.ctx.room_ids(), ["r1"]);
    assert_eq!(w.ctx.rooms().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn create_room_rules_and_messages() {
    let w = World::new().await;
    assert_eq!(
        w.ctx.create_room("bad id", RoomPools::Default),
        Err(CreateRoomError::InvalidId)
    );
    assert_eq!(
        w.ctx.create_room("ok", RoomPools::Exact(vec![2, 2, 1])),
        Ok(vec![2, 1])
    );
    assert_eq!(
        w.ctx.create_room("ok", RoomPools::Default),
        Err(CreateRoomError::AlreadyExists)
    );
    assert_eq!(
        w.ctx.create_room("x", RoomPools::Exact(vec![1, 77])),
        Err(CreateRoomError::PoolNotFound(77))
    );
    assert_eq!(
        w.ctx.create_room("x", RoomPools::Resolve(vec![77])),
        Err(CreateRoomError::NoUsablePools)
    );
    assert_eq!(
        CreateRoomError::NoUsablePools.http_message(),
        "指定的谱池不存在或均为空池"
    );
    assert_eq!(
        CreateRoomError::PoolNotFound(77).console_message("x"),
        "Pool not found: 77"
    );
    assert!(!w.logs.contains("WARN Room x has a single chart pool"));

    let mut setup = Setup::default();
    setup.args.max_rooms = 1;
    let w = World::with(setup).await;
    w.ctx.create_room("a", RoomPools::Default).unwrap();
    let err = w.ctx.create_room("b", RoomPools::Default).unwrap_err();
    assert_eq!(
        err.http_message(),
        "服务器房间数已达上限（1），无法继续创建。"
    );
    assert!(
        w.logs
            .contains("WARN Room a has a single chart pool (1), so it will never rotate")
    );
}

#[tokio::test(start_paused = true)]
async fn update_room_is_all_or_nothing() {
    let w = World::new().await;
    w.ctx.create_room("r1", RoomPools::Default).unwrap();
    let bad = RoomUpdate {
        live: Some(true),
        chat_enable: Some(false),
        min_player: Some(7),
        max_player: Some(3),
        select_countdown: Some(5),
        reject_min_above_max: true,
        ..RoomUpdate::default()
    };
    assert_eq!(
        w.ctx.update_room("r1", &bad),
        Err(UpdateRoomError::SelectCountdownTooShort)
    );
    let view = w.ctx.room("r1").unwrap();
    assert!(!view.live && view.config.chat_enable && view.config.min_player == 2);
    let cross = RoomUpdate {
        min_player: Some(7),
        max_player: Some(3),
        reject_min_above_max: true,
        ..RoomUpdate::default()
    };
    assert_eq!(
        w.ctx.update_room("r1", &cross),
        Err(UpdateRoomError::MinAboveMax)
    );
    assert_eq!(
        UpdateRoomError::MinAboveMax.http_message(),
        "minPlayer 不能大于 maxPlayer"
    );
    assert_eq!(
        w.ctx.update_room(
            "r1",
            &RoomUpdate {
                admin_ids: Some(vec![3, 0]),
                ..RoomUpdate::default()
            }
        ),
        Err(UpdateRoomError::AdminIdNotPositive)
    );
    w.ctx
        .update_room(
            "r1",
            &RoomUpdate {
                admin_ids: Some(vec![9, 3, 9]),
                live: Some(true),
                interval: Some(2),
                ..RoomUpdate::default()
            },
        )
        .unwrap();
    let view = w.ctx.room("r1").unwrap();
    assert_eq!(view.config.admin_ids, [9, 3]);
    assert!(view.live);
    assert_eq!(view.pool.effective_rounds_per_stay, 2);
    assert_eq!(
        w.ctx.update_room("nope", &RoomUpdate::default()),
        Err(UpdateRoomError::RoomNotFound)
    );
}

#[tokio::test(start_paused = true)]
async fn messages_pools_removal_and_membership_queries() {
    let w = World::new().await;
    w.ctx
        .create_room("r1", RoomPools::Exact(vec![1, 2]))
        .unwrap();
    let mut a = w.login(100, "Alice").await;
    let mut b = w.login(200, "Bob").await;
    let mut m = w.login(300, "Mon").await;
    join(&mut a, "r1", false).await;
    join(&mut m, "r1", true).await;
    a.drain().await;

    assert_eq!(w.ctx.say_to_room("r1", "公告"), Some(1));
    assert_eq!(a.lines().await, s(&["> 公告"]));
    assert_eq!(m.lines().await, s(&["> 公告"]));
    assert!(b.lines().await.is_empty());
    assert_eq!(w.ctx.say_to_room("nope", "x"), None);
    assert_eq!(w.ctx.say_to_all("全体"), 3);
    assert_eq!(b.lines().await, s(&["> 全体"]));

    w.ctx.set_room_favorite("r1", Some(4242)).unwrap();
    assert_eq!(w.ctx.room_pool("r1").unwrap().favorite_id, Some(4242));
    a.send(Sb::LockRoom { lock: true }).await;
    let lines = a.lines().await;
    assert!(!lines.iter().any(|l| l.contains("4242")), "{lines:?}");
    a.send(Sb::RequestStart).await;
    a.send(Sb::Ready).await;
    m.send(Sb::Ready).await;
    a.drain().await;
    m.drain().await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    w.ctx.end_round("r1").await.unwrap();
    a.drain().await;
    m.drain().await;
    a.send(Sb::LockRoom { lock: true }).await;
    let lines = a.lines().await;
    assert!(
        lines.contains(&"> 谱面收藏夹 ID：4242".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"> 你可以通过导入收藏夹来一键导入谱池".to_owned()));

    assert_eq!(
        w.ctx.switch_room_pool("r1", 9),
        Err(SwitchPoolError::NotInRoom(
            "Pool not in room chart pool list: 9".into()
        ))
    );
    w.ctx.switch_room_pool("r1", 2).unwrap();
    assert_eq!(w.ctx.room_pool("r1").unwrap().pending_pool_id, Some(2));

    assert!(w.ctx.is_in_room(100, "r1"));
    assert!(w.ctx.is_in_room(300, "r1"));
    assert!(!w.ctx.is_in_room(200, "r1"));
    assert!(w.ctx.can_control_room(100, "r1"));
    assert!(!w.ctx.can_control_room(300, "r1"));
    let online = w.ctx.online_players();
    assert_eq!(online.len(), 3);
    assert_eq!(online[0].room.as_deref(), Some("r1"));
    assert_eq!(online[1].room, None);

    assert_eq!(w.ctx.remove_room("r1"), Err(RemoveRoomError::NotEmpty));
    assert!(w.ctx.kick_player(100));
    assert!(a.closed().await);
    assert!(w.ctx.kick_player(300));
    assert!(m.closed().await);
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(w.ctx.remove_room("r1"), Ok(()));
    assert!(!w.ctx.room_exists("r1"));
    assert_eq!(w.ctx.remove_room("r1"), Err(RemoveRoomError::RoomNotFound));
    assert!(!w.ctx.kick_player(100));
}

#[tokio::test]
async fn tcp_listener_end_to_end() {
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::AsyncWriteExt;
    let w = World::new().await;
    w.upstream.user("tok", 1, "A");
    let listener = w.ctx.bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut task = listener.spawn();
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(&[1]).await.unwrap();
    let mut framed = tokio_util::codec::Framed::new(stream, zphira_protocol::ClientCodec::new());
    framed
        .send(Sb::Authenticate {
            token: "tok".into(),
        })
        .await
        .unwrap();
    let first = tokio::time::timeout(MUST_ARRIVE, framed.next())
        .await
        .expect("login answered")
        .unwrap()
        .unwrap();
    assert!(render(&first).starts_with("Authenticate ok 1 A"));
    w.ctx.shutdown().await;
    let exit = tokio::time::timeout(MUST_ARRIVE, task.finished())
        .await
        .expect("the accept loop ends after shutdown");
    assert_eq!(exit.task, zphira_game::TaskName::Listener);
    assert_eq!(exit.outcome, zphira_game::TaskOutcome::Returned);
    assert!(!exit.is_failure());
    tokio::time::timeout(MUST_ARRIVE, async {
        while let Some(Ok(_)) = framed.next().await {}
    })
    .await
    .expect("closed by the shutdown");
    assert!(w.logs.contains("INFO Closing 2 channel(s)..."));
}

#[tokio::test(start_paused = true)]
async fn background_task_exits_are_reported() {
    use zphira_game::{TaskName, TaskOutcome};
    let w = World::new().await;
    let mut bg = w.ctx.start_background();
    assert!(
        tokio::time::timeout(Duration::from_secs(120), bg.finished())
            .await
            .is_err(),
        "no task ended"
    );
    bg.stop();

    for task in [TaskName::Watchdog, TaskName::RoomStoreWriter] {
        let mut setup = Setup::default();
        setup.options.fail_task = Some(task);
        let w = World::with(setup).await;
        let mut bg = w.ctx.start_background();
        let exit = tokio::time::timeout(Duration::from_secs(60), bg.finished())
            .await
            .expect("the failing task is reported");
        assert_eq!(exit.task, task);
        assert_eq!(
            exit.outcome,
            TaskOutcome::Panicked(format!("injected fault in {task:?}"))
        );
        assert!(exit.is_failure());
        assert!(
            tokio::time::timeout(Duration::from_secs(60), bg.finished())
                .await
                .is_err()
        );
        bg.stop();
    }
}
