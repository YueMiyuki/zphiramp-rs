mod support;

use std::sync::Arc;
use std::time::Duration;

use support::*;
use zphira_catalog::{CatalogConfig, ChartPool, PoolCategory, PoolSnapshot};
use zphira_common::gson;
use zphira_game::{RoomChartPool, RoomUpdate, Rotation, SavedRoom};
use zphira_protocol::ServerBoundPacket as Sb;

fn pool(id: i32, rounds_per_stay: Option<i32>) -> Arc<PoolSnapshot> {
    Arc::new(PoolSnapshot {
        id,
        favorite_id: None,
        default_flag: false,
        chart_ids: vec![1, 2],
        category: PoolCategory::Regular,
        size_limit: Some(15),
        rounds_per_stay,
        order: id * 10,
        submission_open: false,
    })
}

async fn catalog_with(pools_json: &str) -> (ChartPool, Arc<FakeCatalog>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("chart-pools.json"), pools_json).unwrap();
    std::fs::write(
        dir.path()
            .join(zphira_catalog::files::DURATION_RESET_MARKER),
        "",
    )
    .unwrap();
    let upstream = Arc::new(FakeCatalog::default());
    for c in default_charts() {
        upstream.add(c);
    }
    let up: Arc<dyn zphira_catalog::Upstream> = upstream.clone();
    let catalog = ChartPool::load(&CatalogConfig::new(dir.path()), up)
        .await
        .unwrap();
    (catalog, upstream, dir)
}

async fn unrelated_catalog() -> (ChartPool, Arc<FakeCatalog>, tempfile::TempDir) {
    catalog_with(
        r#"{"pools":[{"id":900,"default_flag":true,"chart_ids":[1],"category":"MANUAL"}]}"#,
    )
    .await
}

#[tokio::test]
async fn rotates_on_the_rooms_interval_when_the_pool_has_no_quota() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(catalog, &[pool(1, None), pool(2, None)]).unwrap();
    assert_eq!(p.resolve_rounds_per_stay(3), 3);
    p.finish_playing_round(3);
    p.finish_playing_round(3);
    assert_eq!(p.current_pool_snapshot().id, 1);
    p.finish_playing_round(3);
    assert_eq!(p.current_pool_snapshot().id, 2);
}

#[tokio::test]
async fn the_pool_quota_overrides_the_rooms_interval() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(catalog, &[pool(1, Some(1)), pool(2, Some(1))]).unwrap();
    assert_eq!(p.resolve_rounds_per_stay(9), 1);
    p.finish_playing_round(9);
    assert_eq!(p.current_pool_snapshot().id, 2);
}

#[tokio::test]
async fn a_zero_quota_falls_back_instead_of_rotating_every_round() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let p = RoomChartPool::new(catalog, &[pool(1, Some(0)), pool(2, None)]).unwrap();
    assert_eq!(p.resolve_rounds_per_stay(4), 4);
}

#[tokio::test]
async fn rotation_wraps_around() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(
        catalog,
        &[pool(1, Some(1)), pool(2, Some(1)), pool(3, Some(1))],
    )
    .unwrap();
    for expected in [2, 3, 1, 2] {
        p.finish_playing_round(9);
        assert_eq!(p.current_pool_snapshot().id, expected);
    }
}

#[tokio::test]
async fn a_manual_switch_is_applied_on_the_next_refresh() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(catalog, &[pool(1, Some(2)), pool(2, Some(2))]).unwrap();
    p.switch_pool(2).unwrap();
    assert_eq!(p.pending_pool_id(), Some(2));
    assert_eq!(p.current_pool_snapshot().id, 1);
    p.finish_playing_round(9);
    assert_eq!(p.current_pool_snapshot().id, 1);
    p.finish_playing_round(9);
    assert_eq!(p.current_pool_snapshot().id, 2);
    assert_eq!(p.pending_pool_id(), None);
}

#[tokio::test]
async fn switching_outside_the_frozen_list_is_refused() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(catalog, &[pool(1, Some(5))]).unwrap();
    assert_eq!(
        p.switch_pool(99).unwrap_err(),
        "Pool not in room chart pool list: 99"
    );
}

#[tokio::test]
async fn empty_pools_never_reach_a_room() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let empty = Arc::new(PoolSnapshot {
        id: 9,
        favorite_id: None,
        default_flag: false,
        chart_ids: vec![],
        category: PoolCategory::Tb,
        size_limit: Some(15),
        rounds_per_stay: Some(1),
        order: 90,
        submission_open: false,
    });
    let p = RoomChartPool::new(catalog.clone(), &[empty.clone(), pool(1, Some(1))]).unwrap();
    assert_eq!(p.pools().len(), 1);
    assert_eq!(p.current_pool_snapshot().id, 1);
    assert_eq!(
        RoomChartPool::new(catalog, &[empty]).unwrap_err(),
        "Room chart pool list cannot be empty"
    );
}

#[tokio::test]
async fn the_frozen_list_keeps_the_given_rotation_order() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let p = RoomChartPool::new(
        catalog,
        &[pool(3, Some(1)), pool(1, Some(1)), pool(2, Some(1))],
    )
    .unwrap();
    let ids: Vec<i32> = p.pools().iter().map(|p| p.id).collect();
    assert_eq!(ids, [3, 1, 2]);
}

#[tokio::test]
async fn finishing_a_round_reports_whether_the_pool_rotated() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(catalog, &[pool(1, Some(2)), pool(2, Some(2))]).unwrap();
    assert!(!p.finish_playing_round(9));
    assert!(p.finish_playing_round(9));
    assert_eq!(p.current_pool_snapshot().id, 2);
}

#[tokio::test]
async fn a_restored_room_resumes_its_stay_instead_of_starting_over() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(
        catalog,
        &[pool(1, Some(3)), pool(2, Some(3)), pool(3, Some(3))],
    )
    .unwrap();
    p.restore_rotation(Some(Rotation {
        current_pool_id: Some(2),
        pending_pool_id: None,
        finished_rounds: 2,
    }));
    assert_eq!(p.current_pool_snapshot().id, 2);
    assert_eq!(p.finished_rounds_since_refresh(), 2);
    assert!(p.finish_playing_round(9));
    assert_eq!(p.current_pool_snapshot().id, 3);
}

#[tokio::test]
async fn rotation_state_round_trips_through_restore() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut original =
        RoomChartPool::new(catalog.clone(), &[pool(1, Some(3)), pool(2, Some(3))]).unwrap();
    original.finish_playing_round(9);
    original.switch_pool(2).unwrap();
    let mut restored = RoomChartPool::new(catalog, &[pool(1, Some(3)), pool(2, Some(3))]).unwrap();
    restored.restore_rotation(Some(original.rotation_state()));
    assert_eq!(restored.rotation_state(), original.rotation_state());
}

#[tokio::test]
async fn restoring_a_pool_that_left_the_room_starts_over() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(catalog, &[pool(1, Some(3)), pool(2, Some(3))]).unwrap();
    let fresh = Rotation {
        current_pool_id: Some(1),
        pending_pool_id: None,
        finished_rounds: 0,
    };
    p.restore_rotation(Some(Rotation {
        current_pool_id: Some(99),
        pending_pool_id: Some(2),
        finished_rounds: 2,
    }));
    assert_eq!(p.rotation_state(), fresh);
    p.restore_rotation(Some(Rotation::default()));
    assert_eq!(p.rotation_state(), fresh);
}

#[tokio::test]
async fn a_restored_switch_to_an_unknown_pool_is_dropped() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(catalog, &[pool(1, Some(3)), pool(2, Some(3))]).unwrap();
    p.restore_rotation(Some(Rotation {
        current_pool_id: Some(2),
        pending_pool_id: Some(99),
        finished_rounds: 1,
    }));
    assert_eq!(
        p.rotation_state(),
        Rotation {
            current_pool_id: Some(2),
            pending_pool_id: None,
            finished_rounds: 1
        }
    );
}

#[tokio::test]
async fn a_favorite_override_lasts_only_for_the_current_stay() {
    let (catalog, _, _dir) = unrelated_catalog().await;
    let mut p = RoomChartPool::new(catalog, &[pool(1, Some(1)), pool(2, Some(1))]).unwrap();
    p.set_favorite(Some(42));
    assert_eq!(p.current_pool_snapshot().favorite_id, Some(42));
    p.finish_playing_round(9);
    assert_eq!(p.current_pool_snapshot().favorite_id, None);
}

#[tokio::test]
async fn edits_to_a_pool_apply_to_rooms_that_already_exist() {
    let (catalog, _, _dir) = catalog_with(
        r#"{"pools":[{"id":1,"default_flag":true,"chart_ids":[1,2],"category":"REGULAR","order":10},
                     {"id":2,"default_flag":false,"chart_ids":[1,2],"category":"REGULAR","order":20}]}"#,
    )
    .await;
    let mut p = RoomChartPool::new(catalog.clone(), &[pool(1, None), pool(2, None)]).unwrap();
    catalog.add_chart(1, 3).await.unwrap();
    catalog
        .update_pool(1, None, None, Some(3), None, None)
        .await
        .unwrap();
    assert_eq!(p.resolve_rounds_per_stay(5), 3);
    assert_eq!(p.current_pool_snapshot().chart_ids, [1, 2, 3]);
    p.finish_playing_round(5);
    p.finish_playing_round(5);
    assert!(p.finish_playing_round(5));
    assert_eq!(p.current_pool_snapshot().id, 2);
}

#[tokio::test]
async fn a_deleted_or_emptied_pool_falls_back_to_its_creation_snapshot() {
    let (catalog, _, _dir) = catalog_with(
        r#"{"pools":[{"id":1,"default_flag":true,"chart_ids":[1,2],"category":"REGULAR","order":10,"rounds_per_stay":4},
                     {"id":2,"default_flag":false,"chart_ids":[1,2],"category":"REGULAR","order":20},
                     {"id":3,"default_flag":true,"chart_ids":[1],"category":"REGULAR","order":30}]}"#,
    )
    .await;
    let p = RoomChartPool::new(catalog.clone(), &[pool(1, Some(4)), pool(2, Some(4))]).unwrap();
    assert_eq!(p.resolve_rounds_per_stay(9), 4);

    catalog.remove_chart(1, 1).await.unwrap();
    catalog.remove_chart(1, 2).await.unwrap();
    assert_eq!(p.current_pool_snapshot().chart_ids, [1, 2]);
    assert_eq!(p.resolve_rounds_per_stay(9), 4);

    catalog.remove_pool(1).await.unwrap();
    assert_eq!(p.current_pool_snapshot().chart_ids, [1, 2]);
}

const SETTING: &str = r#"{"auto_destroy": false, "host": false, "max_player": 1000, "locked": false, "cycle": false,
 "live": false, "chat": true, "min_player": 2, "select_chart_countdown_seconds": 150,
 "ready_countdown_seconds": 60, "force_finish_seconds": 3600, "refresh_interval_rounds": 3,
 "admin_ids": []}"#;

#[test]
fn a_file_written_before_rotation_was_saved_still_loads() {
    let rooms: Vec<SavedRoom> = gson::from_str(&format!(
        r#"[{{"room_id": "a", "pool_ids": [1, 2], "setting": {SETTING}}}]"#
    ))
    .unwrap();
    assert_eq!(rooms[0].rotation(), Rotation::default());
}

#[test]
fn rotation_progress_is_read_back() {
    let rooms: Vec<SavedRoom> = gson::from_str(&format!(
        r#"[{{"room_id": "a", "pool_ids": [1, 2], "setting": {SETTING}, "current_pool_id": 2, "pending_pool_id": 1, "finished_rounds": 2}}]"#
    ))
    .unwrap();
    assert_eq!(
        rooms[0].rotation(),
        Rotation {
            current_pool_id: Some(2),
            pending_pool_id: Some(1),
            finished_rounds: 2
        }
    );
    let written = gson::to_string_pretty(&rooms).unwrap();
    for key in [
        "\"current_pool_id\": 2",
        "\"pending_pool_id\": 1",
        "\"finished_rounds\": 2",
    ] {
        assert!(written.contains(key), "{written}");
    }
}

const SHORTENED: &str = "全员已投票";

async fn room_with(w: &World, players: &[(i32, &str)]) -> Vec<Client> {
    let mut clients = Vec::new();
    for (i, (id, name)) in players.iter().enumerate() {
        let mut c = w.login(*id, name).await;
        if i == 0 {
            c.send(Sb::CreateRoom {
                room_id: "test".into(),
            })
            .await;
        } else {
            c.send(Sb::JoinRoom {
                room_id: "test".into(),
                monitor: false,
            })
            .await;
        }
        c.drain().await;
        clients.push(c);
    }
    for c in &mut clients {
        c.drain().await;
    }
    clients
}

async fn seconds_until_vote_ends(c: &mut Client) -> u64 {
    let start = tokio::time::Instant::now();
    loop {
        let packet = c.recv().await.expect("connection open");
        if render(&packet).starts_with("> 投票结束") {
            return start.elapsed().as_secs_f64().round() as u64;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_countdown_shortens_once_every_online_player_has_voted() {
    let w = World::new().await;
    let mut c = room_with(&w, &[(1, "P1"), (2, "P2")]).await;
    c[0].send(Sb::SelectChart { id: 1 }).await;
    let lines = c[0].lines().await;
    assert!(lines.iter().all(|l| !l.contains(SHORTENED)));

    c[1].send(Sb::SelectChart { id: 2 }).await;
    c[1].drain().await;
    let lines = c[0].lines().await;
    assert_eq!(lines.iter().filter(|l| l.contains(SHORTENED)).count(), 1);

    c[1].send(Sb::SelectChart { id: 1 }).await;
    c[1].drain().await;
    let lines = c[0].lines().await;
    assert!(lines.iter().all(|l| !l.contains(SHORTENED)));
    let remaining = seconds_until_vote_ends(&mut c[0]).await;
    assert!(remaining <= 10, "{remaining}");
}

#[tokio::test(start_paused = true)]
async fn offline_players_do_not_hold_the_countdown() {
    let w = World::new().await;
    let mut c = room_with(&w, &[(1, "P1"), (2, "P2"), (3, "P3")]).await;
    let third = c.pop().unwrap();
    third.shutdown().await;
    settle().await;
    c[0].drain().await;
    c[0].send(Sb::SelectChart { id: 1 }).await;
    c[1].send(Sb::SelectChart { id: 1 }).await;
    c[1].drain().await;
    let lines = c[0].lines().await;
    assert_eq!(lines.iter().filter(|l| l.contains(SHORTENED)).count(), 1);
    assert!(seconds_until_vote_ends(&mut c[0]).await <= 10);
}

#[tokio::test(start_paused = true)]
async fn the_countdown_starts_short_when_everyone_voted_before_enough_players_were_online() {
    let w = World::new().await;
    w.ctx
        .create_room("test", zphira_game::RoomPools::Default)
        .unwrap();
    w.ctx
        .update_room(
            "test",
            &RoomUpdate {
                min_player: Some(3),
                ..RoomUpdate::default()
            },
        )
        .unwrap();
    let mut p1 = w.login(1, "P1").await;
    let mut p2 = w.login(2, "P2").await;
    for p in [&mut p1, &mut p2] {
        p.send(Sb::JoinRoom {
            room_id: "test".into(),
            monitor: false,
        })
        .await;
        p.drain().await;
    }
    p1.send(Sb::SelectChart { id: 1 }).await;
    p2.send(Sb::SelectChart { id: 2 }).await;
    p2.drain().await;
    let lines = p1.lines().await;
    assert!(
        lines.iter().all(|l| !l.contains("秒后锁定投票")),
        "{lines:?}"
    );

    w.ctx
        .update_room(
            "test",
            &RoomUpdate {
                min_player: Some(2),
                ..RoomUpdate::default()
            },
        )
        .unwrap();
    p1.send(Sb::Ready).await;
    let lines = p1.lines().await;
    assert!(
        lines.contains(
            &"> 已达到开局人数，10 秒后锁定投票并进入准备阶段。人数不足会取消倒计时。".to_owned()
        ),
        "{lines:?}"
    );
    assert!(seconds_until_vote_ends(&mut p1).await <= 10);
}

#[tokio::test(start_paused = true)]
async fn the_last_undecided_player_leaving_shortens_the_countdown() {
    let w = World::new().await;
    let mut c = room_with(&w, &[(1, "P1"), (2, "P2"), (3, "P3")]).await;
    c[0].send(Sb::SelectChart { id: 1 }).await;
    c[1].send(Sb::SelectChart { id: 1 }).await;
    c[1].drain().await;
    let lines = c[0].lines().await;
    assert!(lines.iter().all(|l| !l.contains(SHORTENED)));
    c[2].drain().await;

    c[2].send(Sb::LeaveRoom).await;
    assert_eq!(c[2].lines().await, s(&["LeaveRoom ok"]));
    let lines = c[0].lines().await;
    assert_eq!(lines.iter().filter(|l| l.contains(SHORTENED)).count(), 1);
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert!(seconds_until_vote_ends(&mut c[0]).await <= 10);
}
