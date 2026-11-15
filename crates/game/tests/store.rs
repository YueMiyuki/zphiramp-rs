mod support;

use std::time::Duration;

use support::*;
use zphira_game::{RoomPools, RoomUpdate};

const JAVA_FILE: &str = r#"[
  {
    "room_id": "event",
    "pool_ids": [
      2,
      1
    ],
    "setting": {
      "auto_destroy": false,
      "host": false,
      "max_player": 8,
      "locked": false,
      "cycle": false,
      "live": true,
      "chat": false,
      "min_player": 3,
      "select_chart_countdown_seconds": 90,
      "ready_countdown_seconds": 30,
      "force_finish_seconds": 3600,
      "refresh_interval_rounds": 2,
      "admin_ids": [
        42,
        7
      ]
    },
    "current_pool_id": 1,
    "pending_pool_id": 2,
    "finished_rounds": 1
  },
  {
    "room_id": "old",
    "pool_ids": [
      1
    ],
    "setting": {
      "auto_destroy": false,
      "host": false,
      "max_player": 1000,
      "locked": false,
      "cycle": false,
      "live": false,
      "chat": true,
      "min_player": 2,
      "select_chart_countdown_seconds": 150,
      "ready_countdown_seconds": 60,
      "force_finish_seconds": 3600,
      "refresh_interval_rounds": 5,
      "admin_ids": []
    }
  }
]"#;

#[tokio::test(start_paused = true)]
async fn restores_java_files_old_and_current_shape() {
    let setup = Setup {
        rooms_json: Some(JAVA_FILE.to_owned()),
        ..Setup::default()
    };
    let w = World::with(setup).await;
    assert_eq!(w.ctx.restore_rooms().await, 2);
    assert!(w.logs.contains(&format!(
        "INFO Restored 2 room(s) from {}",
        w.data().join("rooms.json").display()
    )));
    assert!(
        w.logs
            .contains("WARN Room old has a single chart pool (1), so it will never rotate")
    );

    let event = w.ctx.room("event").unwrap();
    assert_eq!(event.config.max_player, 8);
    assert_eq!(event.config.min_player, 3);
    assert_eq!(event.config.select_countdown, 90);
    assert!(!event.config.chat_enable);
    assert!(event.live);
    assert_eq!(event.config.admin_ids, [7, 42]);
    assert_eq!(event.pool.current_pool.id, 1);
    assert_eq!(event.pool.pending_pool_id, Some(2));
    assert_eq!(event.pool.finished_rounds_since_refresh, 1);
    let ids: Vec<i32> = event.pool.pools.iter().map(|p| p.id).collect();
    assert_eq!(ids, [2, 1]);

    for view in [&event, &w.ctx.room("old").unwrap()] {
        assert!(view.config.cancel_ready_in_vote);
        assert!(view.config.start_when_all_ready);
        assert_eq!(view.config.all_ready_delay, 5);
    }

    let old = w.ctx.room("old").unwrap();
    assert_eq!(old.pool.current_pool.id, 1);
    assert_eq!(old.pool.finished_rounds_since_refresh, 0);
    assert_eq!(old.state, "SelectChart");
    assert!(old.players.is_empty());
}

#[tokio::test(start_paused = true)]
async fn flush_content_and_order() {
    let w = World::new().await;
    assert!(!w.ctx.flush_rooms().await, "nothing dirty yet");
    w.ctx.create_room("zeta", RoomPools::Default).unwrap();
    w.ctx
        .create_room("alpha", RoomPools::Exact(vec![1, 2, 1]))
        .unwrap();
    w.ctx
        .update_room(
            "alpha",
            &RoomUpdate {
                admin_ids: Some(vec![9, 3, 9]),
                ..RoomUpdate::default()
            },
        )
        .unwrap();
    w.ctx.switch_room_pool("alpha", 2).unwrap();
    assert!(w.ctx.flush_rooms().await);
    let text = std::fs::read_to_string(w.data().join("rooms.json")).unwrap();
    // The fixture plus the three ready-phase keys this server writes after admin_ids.
    let expected = include_str!("fixtures/rooms_flush_java.json").replace(
        "]\n    },",
        "],\n      \"cancel_ready_in_vote\": true,\n      \"start_when_all_ready\": true,\n      \"all_ready_delay_seconds\": 5\n    },",
    );
    assert_eq!(text, expected);
    assert!(!w.ctx.flush_rooms().await, "clean after a write");

    let bg = w.ctx.start_background();
    w.ctx.remove_room("zeta").unwrap();
    tokio::time::sleep(Duration::from_secs(6)).await;
    let started = std::time::Instant::now();
    loop {
        let text = std::fs::read_to_string(w.data().join("rooms.json")).unwrap();
        if !text.contains("zeta") {
            break;
        }
        assert!(
            started.elapsed() < MUST_ARRIVE,
            "the writer never saved the removal: {text}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    bg.stop();
}

#[tokio::test(start_paused = true)]
async fn a20_empty_pool_at_restore_is_kept_and_skipped() {
    let pools = r#"{"pools":[
      {"id":1,"default_flag":true,"chart_ids":[1,2],"category":"MANUAL","order":0,"rounds_per_stay":1},
      {"id":2,"default_flag":false,"chart_ids":[],"category":"MANUAL","order":1,"rounds_per_stay":1},
      {"id":3,"default_flag":false,"chart_ids":[3,4],"category":"MANUAL","order":2,"rounds_per_stay":1}
    ]}"#;
    let rooms = r#"[{"room_id":"r","pool_ids":[1,2,99,3],"setting":{"auto_destroy":false,"max_player":10,"chat":true,"min_player":2,"select_chart_countdown_seconds":150,"ready_countdown_seconds":60,"force_finish_seconds":3600,"refresh_interval_rounds":5}}]"#;
    let setup = Setup {
        pools_json: pools.to_owned(),
        rooms_json: Some(rooms.to_owned()),
        ..Setup::default()
    };
    let w = World::with(setup).await;
    assert_eq!(w.ctx.restore_rooms().await, 1);
    w.ctx.mark_rooms_dirty();
    assert!(w.ctx.flush_rooms().await);
    let saved: Vec<zphira_game::SavedRoom> = zphira_common::gson::from_str(
        &std::fs::read_to_string(w.data().join("rooms.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved[0].pool_ids.as_deref(), Some(&[1, 2, 99, 3][..]));

    let mut a = w.login(1, "A").await;
    let mut b = w.login(2, "B").await;
    for c in [&mut a, &mut b] {
        c.send(zphira_protocol::ServerBoundPacket::JoinRoom {
            room_id: "r".into(),
            monitor: false,
        })
        .await;
        c.drain().await;
    }
    w.ctx.end_round("r").await.unwrap_err();
    play_one_round(&w, &mut a, &mut b, "r").await;
    assert_eq!(w.ctx.room("r").unwrap().pool.current_pool.id, 3);

    w.catalog.add_chart(2, 5).await.unwrap();
    play_one_round(&w, &mut a, &mut b, "r").await;
    play_one_round(&w, &mut a, &mut b, "r").await;
    assert_eq!(w.ctx.room("r").unwrap().pool.current_pool.id, 2);
}

async fn play_one_round(w: &World, a: &mut Client, b: &mut Client, room: &str) {
    use zphira_protocol::ServerBoundPacket as Sb;
    a.drain().await;
    b.drain().await;
    a.send(Sb::RequestStart).await;
    a.send(Sb::Ready).await;
    b.send(Sb::Ready).await;
    a.drain().await;
    b.drain().await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    a.drain().await;
    b.drain().await;
    w.ctx.end_round(room).await.unwrap();
    a.drain().await;
    b.drain().await;
}

#[tokio::test(start_paused = true)]
async fn rooms_that_cannot_be_restored_are_skipped_with_the_java_warnings() {
    let rooms = r#"[
      {"room_id":"gone","pool_ids":[77],"setting":{"max_player":10,"min_player":2}},
      {"room_id":"a","pool_ids":[1],"setting":{"max_player":10,"min_player":2}},
      {"room_id":"b","pool_ids":[1],"setting":{"max_player":10,"min_player":2}}
    ]"#;
    let mut setup = Setup {
        rooms_json: Some(rooms.to_owned()),
        ..Setup::default()
    };
    setup.args.max_rooms = 1;
    let w = World::with(setup).await;
    assert_eq!(w.ctx.restore_rooms().await, 1);
    assert!(
        w.logs
            .contains("WARN Skipping room gone: its pools no longer exist")
    );
    assert!(
        w.logs
            .contains("WARN Skipping room b: 服务器房间数已达上限（1），无法继续创建。")
    );

    let setup = Setup {
        rooms_json: Some("[{oops".to_owned()),
        ..Setup::default()
    };
    let w = World::with(setup).await;
    assert_eq!(w.ctx.restore_rooms().await, 0);
    assert!(
        w.logs
            .lines()
            .iter()
            .any(|l| l.starts_with("WARN Room store is unreadable (")
                && l.ends_with("), starting with no rooms"))
    );

    let setup = Setup {
        rooms_json: Some("[]".to_owned()),
        ..Setup::default()
    };
    let w = World::with(setup).await;
    assert_eq!(w.ctx.restore_rooms().await, 0);
    assert!(!w.logs.contains("Restored"));
}

#[tokio::test(start_paused = true)]
async fn flush_then_restore_round_trips() {
    let w = World::new().await;
    w.ctx
        .create_room("x", RoomPools::Resolve(vec![2, 1]))
        .unwrap();
    w.ctx
        .update_room(
            "x",
            &RoomUpdate {
                min_player: Some(4),
                interval: Some(3),
                locked: Some(true),
                cancel_ready_in_vote: Some(false),
                start_when_all_ready: Some(false),
                all_ready_delay: Some(42),
                ..RoomUpdate::default()
            },
        )
        .unwrap();
    assert!(w.ctx.flush_rooms().await);
    let file = std::fs::read_to_string(w.data().join("rooms.json")).unwrap();
    let before = w.ctx.room("x").unwrap();

    let setup = Setup {
        rooms_json: Some(file),
        ..Setup::default()
    };
    let w2 = World::with(setup).await;
    assert_eq!(w2.ctx.restore_rooms().await, 1);
    let after = w2.ctx.room("x").unwrap();
    assert_eq!(after.config.min_player, before.config.min_player);
    assert_eq!(after.config.interval, 3);
    assert!(after.locked);
    assert!(!after.config.cancel_ready_in_vote);
    assert!(!after.config.start_when_all_ready);
    assert_eq!(after.config.all_ready_delay, 42);
    let ids = |v: &zphira_game::RoomView| v.pool.pools.iter().map(|p| p.id).collect::<Vec<_>>();
    assert_eq!(ids(&after), ids(&before));
    assert_eq!(ids(&after), [2, 1]);
}

#[tokio::test(start_paused = true)]
async fn n23_destroyed_rooms_leave_the_file() {
    use zphira_protocol::ServerBoundPacket as Sb;
    let w = World::new().await;
    let mut a = w.login(1, "A").await;
    a.send(Sb::CreateRoom {
        room_id: "mine".into(),
    })
    .await;
    a.drain().await;
    w.ctx.create_room("idle", RoomPools::Default).unwrap();
    assert!(w.ctx.flush_rooms().await);
    let text = std::fs::read_to_string(w.data().join("rooms.json")).unwrap();
    assert!(
        text.contains("\"mine\"") && text.contains("\"idle\""),
        "{text}"
    );

    a.send(Sb::LeaveRoom).await;
    a.drain().await;
    assert!(!w.ctx.room_exists("mine"));
    assert!(
        w.ctx.flush_rooms().await,
        "the destruction marked the store dirty"
    );
    let text = std::fs::read_to_string(w.data().join("rooms.json")).unwrap();
    assert!(
        !text.contains("\"mine\"") && text.contains("\"idle\""),
        "{text}"
    );

    tokio::time::sleep(Duration::from_secs(61 * 60)).await;
    w.ctx.reclaim_empty_rooms();
    assert!(!w.ctx.room_exists("idle"));
    assert!(w.ctx.flush_rooms().await);
    let text = std::fs::read_to_string(w.data().join("rooms.json")).unwrap();
    assert_eq!(text, "[]");
}

#[tokio::test(start_paused = true)]
async fn n24_cycle_toggle_is_saved() {
    use zphira_protocol::ServerBoundPacket as Sb;
    let w = World::new().await;
    w.ctx.create_room("r1", RoomPools::Default).unwrap();
    assert!(w.ctx.flush_rooms().await);
    let mut a = w.login(1, "A").await;
    a.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    a.drain().await;
    assert!(
        !w.ctx.flush_rooms().await,
        "joining does not change the file"
    );
    a.send(Sb::CycleRoom { cycle: true }).await;
    a.drain().await;
    assert!(w.ctx.flush_rooms().await);
    let text = std::fs::read_to_string(w.data().join("rooms.json")).unwrap();
    assert!(text.contains("\"cycle\": true"), "{text}");
}

#[tokio::test(start_paused = true)]
async fn ready_phase_keys_from_hand_edited_files() {
    let rooms = r#"[
      {"room_id":"a","pool_ids":[1],"setting":{"max_player":10,"min_player":2,"cancel_ready_in_vote":false,"start_when_all_ready":null,"all_ready_delay_seconds":999}},
      {"room_id":"b","pool_ids":[1],"setting":{"max_player":10,"min_player":2,"all_ready_delay_seconds":-4}}
    ]"#;
    let setup = Setup {
        rooms_json: Some(rooms.to_owned()),
        ..Setup::default()
    };
    let w = World::with(setup).await;
    assert_eq!(w.ctx.restore_rooms().await, 2);
    let a = w.ctx.room("a").unwrap().config;
    assert!(!a.cancel_ready_in_vote && a.start_when_all_ready);
    assert_eq!(a.all_ready_delay, 300);
    let b = w.ctx.room("b").unwrap().config;
    assert!(b.cancel_ready_in_vote && b.start_when_all_ready);
    assert_eq!(b.all_ready_delay, 0);
}
