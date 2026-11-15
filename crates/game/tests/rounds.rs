mod support;

use std::time::Duration;

use support::*;
use zphira_protocol::ServerBoundPacket as Sb;

const COUNTDOWN_150: &str =
    "> 已达到开局人数，150 秒后锁定投票并进入准备阶段。人数不足会取消倒计时。";
const HINT: &str = "> 点击锁定房间按钮查看当前谱池状态。";
const SHORTENED: &str = "> 全员已投票，倒计时缩短为 10 秒，锁定前仍可改票。";
const LOCKED: &str = "> 本轮曲目已锁定，无法继续改票。";
const READY_PHASE_60: &str = "> 进入准备阶段：请在 60 秒内准备。未准备玩家将作为观战跳过本轮。";

async fn sleep(secs: f64) {
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
}

async fn room_of_two(w: &World) -> (Client, Client) {
    let mut a = w.login(100, "Alice").await;
    let mut b = w.login(200, "Bob").await;
    a.send(Sb::CreateRoom {
        room_id: "r1".into(),
    })
    .await;
    let mut expected = vec!["CreateRoom ok".to_owned()];
    expected.extend(vote_board("r1", 0, 1, 0, 0));
    assert_eq!(a.lines().await, expected);

    b.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    let mut expected = s(&[
        "OnJoinRoom 200 Bob",
        "msg JoinRoom 200 Bob",
        "> 欢迎 Bob（#200） 加入房间",
    ]);
    expected.extend(vote_board("r1", 0, 2, 0, 0));
    expected.extend(s(&[
        COUNTDOWN_150,
        "JoinRoom ok SelectChart [100,200] live=false",
        "host true",
        "> 房间玩法：选谱阶段投票 → 准备阶段 → 游戏，票数最高者开局。",
        "> 投票仅限当前谱池，重复投票会改票，观战者不能投票。",
        "> 当前积分：0，积分排名：#2",
    ]));
    assert_eq!(b.lines().await, expected);
    assert_eq!(
        a.lines().await,
        s(&[
            "OnJoinRoom 200 Bob",
            "msg JoinRoom 200 Bob",
            "> 欢迎 Bob（#200） 加入房间",
            COUNTDOWN_150,
        ])
    );
    (a, b)
}

fn vote_lines(name: &str, leader: i32, chart: i32) -> Vec<String> {
    let label = match chart {
        1 => "Alpha | Lv.IN 12 | ID:1",
        _ => "Beta | Lv.AT 14 | ID:2",
    };
    let leading_name = if chart == 1 { "Alpha" } else { "Beta" };
    vec![
        format!("> {name} 已投票：{label}"),
        format!("msg SelectChart {leader} {leading_name} {chart}"),
        format!("state SelectChart({chart})"),
        HINT.to_owned(),
    ]
}

async fn vote_to_ready(a: &mut Client, b: &mut Client) {
    a.send(Sb::SelectChart { id: 1 }).await;
    let mut expected = vote_lines("Alice", 100, 1);
    expected.push("SelectChart ok".to_owned());
    assert_eq!(a.lines().await, expected);
    assert_eq!(b.lines().await, vote_lines("Alice", 100, 1));

    b.send(Sb::SelectChart { id: 1 }).await;
    let mut both = vote_lines("Bob", 100, 1);
    both.push(SHORTENED.to_owned());
    let mut expected = both.clone();
    expected.push("SelectChart ok".to_owned());
    assert_eq!(b.lines().await, expected);
    assert_eq!(a.lines().await, both);

    sleep(4.9).await;
    assert!(a.lines().await.is_empty());
    sleep(0.2).await;
    assert_eq!(a.lines().await, s(&["> 投票锁定倒计时：5 秒"]));
    sleep(2.0).await;
    assert_eq!(a.lines().await, s(&["> 投票锁定倒计时：3 秒"]));
    sleep(1.0).await;
    assert_eq!(a.lines().await, s(&["> 投票锁定倒计时：2 秒"]));
    sleep(1.0).await;
    let lock = s(&[
        "> 投票锁定倒计时：1 秒",
        LOCKED,
        SEP,
        "state SelectChart(1)",
    ]);
    assert_eq!(a.lines().await, lock);
    sleep(1.0).await;
    let ready = s(&[
        "> 投票结束，本轮曲目：Alpha | Lv.IN 12 | ID:1",
        "state WaitForReady",
        READY_PHASE_60,
        "> 准备倒计时：60 秒。",
    ]);
    assert_eq!(a.lines().await, ready);
    let mut b_all = s(&[
        "> 投票锁定倒计时：5 秒",
        "> 投票锁定倒计时：3 秒",
        "> 投票锁定倒计时：2 秒",
    ]);
    b_all.extend(lock);
    b_all.extend(ready);
    assert_eq!(b.lines().await, b_all);
}

async fn both_ready(a: &mut Client, b: &mut Client) {
    a.send(Sb::Ready).await;
    assert_eq!(a.lines().await, s(&["msg Ready 100", "Ready ok"]));
    assert_eq!(b.lines().await, s(&["msg Ready 100"]));
    b.send(Sb::Ready).await;
    let wait = s(&["msg Ready 200", "> 全员已准备，5 秒后开始游戏。"]);
    let mut expected = wait.clone();
    expected.push("Ready ok".to_owned());
    assert_eq!(b.lines().await, expected);
    assert_eq!(a.lines().await, wait);
    sleep(4.9).await;
    assert!(a.lines().await.is_empty());
    sleep(0.1).await;
    let start = s(&["state Playing", "msg StartPlaying"]);
    assert_eq!(a.lines().await, start);
    assert_eq!(b.lines().await, start);
}

fn alice_record() -> zphira_phira::GameRecord {
    let mut r = game_record(
        9001,
        100,
        1,
        990_000,
        0.987_654,
        0.012_345_6,
        EPOCH_MILLIS + 20_000,
    );
    r.std_score = 95.5;
    r
}

fn bob_record() -> zphira_phira::GameRecord {
    game_record(9002, 200, 1, 950_000, 0.993, 0.0071, EPOCH_MILLIS + 21_000)
}

const ALICE_PLAYED: &str = "> [100] Alice 的分数: 990000, 准度: 98.765396%, 误差: ±12.3456ms, 无暇度分数: 95.5\n    Perfect: 100, Good: 2, Bad: 1, Miss: 0\n";
const BOB_PLAYED: &str = "> [200] Bob 的分数: 950000, 准度: 99.299995%, 误差: ±7.1ms, 无暇度分数: 90.5\n    Perfect: 100, Good: 2, Bad: 1, Miss: 0\n";

#[tokio::test(start_paused = true)]
async fn normal_two_player_round() {
    let w = World::new().await;
    w.upstream.record(alice_record());
    w.upstream.record(bob_record());
    let (mut a, mut b) = room_of_two(&w).await;
    vote_to_ready(&mut a, &mut b).await;
    both_ready(&mut a, &mut b).await;
    assert!(w.logs.contains(
        "INFO Room r1 round on 'Alpha' (nulls chart) will force finish after 3600s (configured 3600)"
    ));

    a.send(Sb::Played { id: 9001 }).await;
    assert_eq!(
        a.lines().await,
        s(&[
            ALICE_PLAYED,
            "host true",
            "state SelectChart(1)",
            "Played ok"
        ])
    );
    assert_eq!(b.lines().await, s(&[ALICE_PLAYED]));

    b.send(Sb::Played { id: 9002 }).await;
    let end = s(&[
        SEP,
        "> 本轮排名",
        "> 1. Alice - 分数: 990000, 准度: 98.765396%, 误差: ±12.3456ms, 积分: +100, 总积分: 100",
        "> 2. Bob - 分数: 950000, 准度: 99.299995%, 误差: ±7.1ms, 积分: +75, 总积分: 75",
        SEP,
        "host true",
        "msg GameEnd",
        "state SelectChart",
        "> 当前谱池 #1 已进行 1/5 轮。",
        HINT,
        COUNTDOWN_150,
    ]);
    let mut expected = s(&[BOB_PLAYED, "host true", "state SelectChart(1)"]);
    expected.extend(end.clone());
    expected.push("Played ok".to_owned());
    assert_eq!(b.lines().await, expected);
    let mut expected = s(&[BOB_PLAYED]);
    expected.extend(end);
    assert_eq!(a.lines().await, expected);

    assert!(
        w.logs
            .contains("INFO Round in r1: Alice submitted record 9001 (score 990000)")
    );
    assert!(
        w.logs
            .contains("INFO Round in r1 ended: 2 of 2 player(s) submitted a record")
    );
    assert_eq!(w.points.summary(100, "Alice").points, 100);
    assert_eq!(w.points.summary(200, "Bob").points, 75);
    let records = w.round_records();
    assert_eq!(records.len(), 1);
    let round = &records[0];
    assert_eq!(round.room_id, "r1");
    assert_eq!(round.chart_id, 1);
    assert_eq!(round.chart_name.as_deref(), Some("Alpha"));
    assert_eq!(round.results.len(), 2);
    assert_eq!(round.results[0].player_id, 100);
    assert_eq!(round.results[0].rank, 1);
    assert_eq!(round.results[0].accuracy, f64::from(0.987_654_f32));
    assert_eq!(round.results[1].gained_points, 75);
    assert_eq!(round.results[1].total_points, 75);
    assert!(round.finished_at >= round.started_at);
}

async fn pings(clients: &mut [&mut Client]) {
    for c in clients.iter_mut() {
        c.send(Sb::Ping).await;
    }
}

async fn quiet(c: &mut Client) -> Vec<String> {
    c.lines()
        .await
        .into_iter()
        .filter(|l| l != "Pong")
        .collect()
}

async fn into_round(a: &mut Client, b: &mut Client) {
    a.send(Sb::SelectChart { id: 1 }).await;
    b.send(Sb::SelectChart { id: 1 }).await;
    b.drain().await;
    sleep(10.5).await;
    a.drain().await;
    b.drain().await;
    a.send(Sb::Ready).await;
    a.drain().await;
    b.send(Sb::Ready).await;
    b.drain().await;
    sleep(5.0).await;
    a.drain().await;
    b.drain().await;
}

async fn into_ready(a: &mut Client, b: &mut Client) {
    a.send(Sb::SelectChart { id: 1 }).await;
    b.send(Sb::SelectChart { id: 1 }).await;
    b.drain().await;
    sleep(10.5).await;
    a.drain().await;
    b.drain().await;
}

fn round_end_tail() -> Vec<String> {
    s(&[
        "host true",
        "msg GameEnd",
        "state SelectChart",
        "> 当前谱池 #1 已进行 1/5 轮。",
        HINT,
    ])
}

#[tokio::test(start_paused = true)]
async fn tie_is_broken_at_random_at_the_lock() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    a.send(Sb::SelectChart { id: 1 }).await;
    b.send(Sb::SelectChart { id: 2 }).await;
    b.drain().await;
    a.drain().await;
    sleep(9.05).await;
    let lines = a.lines().await;
    let at = lines
        .iter()
        .position(|l| l == "> 投票锁定倒计时：1 秒")
        .unwrap();
    let lines = lines[at..].to_vec();
    assert_eq!(lines[1], "> 最高票出现并列，将随机抽选本轮曲目。");
    assert_eq!(lines[2], LOCKED);
    assert_eq!(lines[3], SEP);
    assert!(lines[4] == "state SelectChart(1)" || lines[4] == "state SelectChart(2)");
    let locked = if lines[4].ends_with("(1)") {
        "Alpha | Lv.IN 12 | ID:1"
    } else {
        "Beta | Lv.AT 14 | ID:2"
    };
    sleep(1.0).await;
    let lines = a.lines().await;
    assert_eq!(lines[0], format!("> 投票结束，本轮曲目：{locked}"));
    b.drain().await;
}

#[tokio::test(start_paused = true)]
async fn change_of_vote_and_vote_outside_the_pool() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    a.send(Sb::SelectChart { id: 1 }).await;
    b.send(Sb::SelectChart { id: 1 }).await;
    a.drain().await;
    b.drain().await;
    b.send(Sb::SelectChart { id: 2 }).await;
    let mut expected = s(&[
        "> Bob 已投票：Beta | Lv.AT 14 | ID:2",
        "msg SelectChart 100 Alpha 1",
        "state SelectChart(1)",
        HINT,
    ]);
    expected.push("SelectChart ok".to_owned());
    assert_eq!(b.lines().await, expected);

    a.send(Sb::SelectChart { id: 3 }).await;
    assert_eq!(
        a.lines().await.last().unwrap(),
        "SelectChart failed: 谱面信息获取失败"
    );
    assert!(
        w.logs
            .contains("INFO Vote rejected in r1: chart 3 is not in the current pool (2 charts)")
    );
    sleep(9.5).await;
    a.drain().await;
    a.send(Sb::SelectChart { id: 2 }).await;
    assert_eq!(
        a.lines().await,
        s(&[LOCKED, "SelectChart failed: 你不能在当前状态执行这个操作"])
    );
}

#[tokio::test(start_paused = true)]
async fn monitor_cannot_vote_and_receives_live_data_only_while_playing() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    let mut m = w.login(300, "Carol").await;
    m.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: true,
    })
    .await;
    let mut expected = s(&[
        "OnJoinRoom 300 Carol monitor",
        "msg JoinRoom 300 Carol",
        "> Carol（#300） 进入房间观战",
    ]);
    expected.extend(vote_board("r1", 0, 3, 0, 0));
    expected.extend(s(&[
        "JoinRoom ok SelectChart [100,200,300m] live=false",
        "host true",
        "> 房间玩法：选谱阶段投票 → 准备阶段 → 游戏，票数最高者开局。",
        "> 投票仅限当前谱池，重复投票会改票，观战者不能投票。",
        "> 当前积分：0，积分排名：#3",
    ]));
    assert_eq!(m.lines().await, expected);
    let seen = s(&[
        "OnJoinRoom 300 Carol monitor",
        "msg JoinRoom 300 Carol",
        "> Carol（#300） 进入房间观战",
    ]);
    assert_eq!(a.lines().await, seen);
    assert_eq!(b.lines().await, seen);

    m.send(Sb::SelectChart { id: 1 }).await;
    assert_eq!(m.lines().await, s(&["SelectChart failed: 你没有权限"]));
    assert!(
        w.logs
            .contains("INFO Vote rejected in r1: Carol is a monitor")
    );

    a.send(Sb::Touches { frames: vec![] }).await;
    assert!(m.lines().await.is_empty());

    into_ready(&mut a, &mut b).await;
    m.drain().await;
    a.send(Sb::Ready).await;
    b.send(Sb::Ready).await;
    a.drain().await;
    b.drain().await;
    assert_eq!(
        m.lines().await,
        s(&[
            "msg Ready 100",
            "msg Ready 200",
            "> 全员已准备，5 秒后开始游戏。"
        ])
    );
    sleep(5.0).await;
    assert_eq!(m.lines().await, s(&["state Playing", "msg StartPlaying"]));
    a.drain().await;
    b.drain().await;

    a.send(Sb::Touches { frames: vec![] }).await;
    a.send(Sb::Judges { judges: vec![] }).await;
    assert_eq!(m.lines().await, s(&["Touches 100 x0", "Judges 100 x0"]));
    assert!(b.lines().await.is_empty());
    m.send(Sb::Touches { frames: vec![] }).await;
    settle().await;
    assert!(a.lines().await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn host_force_start_and_refusals() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    b.send(Sb::RequestStart).await;
    assert_eq!(b.lines().await, s(&["RequestStart failed: 你不是房主"]));
    a.send(Sb::SelectChart { id: 2 }).await;
    a.drain().await;
    b.drain().await;
    a.send(Sb::RequestStart).await;
    let start = s(&[
        "> Alice 提前开始本轮，曲目：Beta | Lv.AT 14 | ID:2",
        "state WaitForReady",
        READY_PHASE_60,
        "> 准备倒计时：60 秒。",
    ]);
    let mut expected = start.clone();
    // The answer precedes the zero-delay notice.
    expected.insert(3, "RequestStart ok".to_owned());
    assert_eq!(a.lines().await, expected);
    assert_eq!(b.lines().await, start);
    a.send(Sb::RequestStart).await;
    assert_eq!(a.lines().await, s(&["RequestStart failed: 你没有权限"]));
    a.send(Sb::Abort).await;
    assert_eq!(
        a.lines().await,
        s(&["Abort failed: 你不能在当前状态执行这个操作"])
    );
    a.send(Sb::Played { id: 1 }).await;
    assert_eq!(
        a.lines().await,
        s(&["Played failed: 你不能在当前状态执行这个操作"])
    );
}

#[tokio::test(start_paused = true)]
async fn nobody_ready_cancels_the_round_after_the_notices() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    into_ready(&mut a, &mut b).await;
    let mut seen = Vec::new();
    for (wait, notice) in [
        (30.0, 30),
        (20.0, 10),
        (5.0, 5),
        (2.0, 3),
        (1.0, 2),
        (1.0, 1),
    ] {
        sleep(wait).await;
        let lines = a.lines().await;
        assert_eq!(lines, s(&[&format!("> 准备倒计时：{notice} 秒。")]));
        seen.extend(lines);
    }
    sleep(1.0).await;
    let end = s(&[
        "host true",
        "state SelectChart(1)",
        "> 没有已准备的在线玩家，本轮取消。",
        "state SelectChart",
        COUNTDOWN_150,
    ]);
    assert_eq!(a.lines().await, end);
    seen.extend(end);
    assert_eq!(b.lines().await, seen);
}

#[tokio::test(start_paused = true)]
async fn not_enough_players_cancels_the_countdown() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    b.send(Sb::LeaveRoom).await;
    assert_eq!(b.lines().await, s(&["LeaveRoom ok"]));
    assert_eq!(
        a.lines().await,
        s(&[
            "msg LeaveRoom 200 Bob",
            HINT,
            "> 在线玩家不足，开局倒计时已取消。"
        ])
    );
    b.send(Sb::Ready).await;
    assert!(b.closed().await);
    settle().await;
    assert!(w.logs.contains("Client disconnected: 127.0.0.1:"));
    assert!(w.logs.lines().iter().any(|l| l.ends_with("(KICK)")));
}

#[tokio::test(start_paused = true)]
async fn abort_then_the_other_player_finishes_the_round() {
    let w = World::new().await;
    w.upstream.record(bob_record());
    let (mut a, mut b) = room_of_two(&w).await;
    into_round(&mut a, &mut b).await;
    a.send(Sb::Abort).await;
    assert_eq!(
        a.lines().await,
        s(&[
            "msg Abort 100",
            "host true",
            "state SelectChart(1)",
            "Abort ok"
        ])
    );
    assert_eq!(b.lines().await, s(&["msg Abort 100"]));
    a.send(Sb::Abort).await;
    assert_eq!(a.lines().await, s(&["Abort ok"]));

    b.send(Sb::Played { id: 9002 }).await;
    let mut end = s(&[
        BOB_PLAYED,
        SEP,
        "> 本轮排名",
        "> 1. Bob - 分数: 950000, 准度: 99.299995%, 误差: ±7.1ms, 积分: +100, 总积分: 100",
        SEP,
    ]);
    end.extend(round_end_tail());
    end.push(COUNTDOWN_150.to_owned());
    assert_eq!(a.lines().await, end);
    let lines = b.lines().await;
    assert_eq!(lines.last().unwrap(), "Played ok");
    assert!(
        w.logs
            .contains("INFO Round in r1 ended: 1 of 2 player(s) submitted a record")
    );
    assert!(w.upstream.calls().contains(&"recent 100".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn record_fetch_failure_counts_as_giving_up() {
    let w = World::new().await;
    w.upstream.failing_records.lock().unwrap().insert(7777);
    let (mut a, mut b) = room_of_two(&w).await;
    into_round(&mut a, &mut b).await;
    a.send(Sb::Played { id: 7777 }).await;
    assert_eq!(
        a.lines().await,
        s(&[
            "> 网络错误导致成绩提交失败，本轮将视为放弃。",
            "> 因网络错误导致 Alice 成绩提交失败，已视为放弃本轮。",
            "msg Abort 100",
            "host true",
            "state SelectChart(1)",
            "Played failed: 成绩提交失败，请稍后重试",
        ])
    );
    assert_eq!(
        b.lines().await,
        s(&[
            "> 因网络错误导致 Alice 成绩提交失败，已视为放弃本轮。",
            "msg Abort 100"
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn missing_record_is_recovered_after_a_disconnect() {
    let w = World::new().await;
    w.upstream.record(alice_record());
    w.upstream.recent(
        200,
        vec![
            game_record(5, 200, 2, 1, 0.5, 0.1, EPOCH_MILLIS + 25_000),
            bob_record(),
        ],
    );
    let (mut a, mut b) = room_of_two(&w).await;
    into_round(&mut a, &mut b).await;
    a.send(Sb::Played { id: 9001 }).await;
    a.drain().await;
    b.drain().await;
    b.shutdown().await;
    settle().await;
    let mut expected = s(&[
        "msg Abort 200",
        "> Bob 的成绩已从 Phira 同步：950000",
        SEP,
        "> 本轮排名",
        "> 1. Alice - 分数: 990000, 准度: 98.765396%, 误差: ±12.3456ms, 积分: +100, 总积分: 100",
        "> 2. Bob - 分数: 950000, 准度: 99.299995%, 误差: ±7.1ms, 积分: +75, 总积分: 75",
        SEP,
    ]);
    expected.extend(round_end_tail());
    assert_eq!(a.lines().await, expected);
    assert!(
        w.logs
            .contains("INFO Recovered record 9002 for Bob (score 950000) from Phira")
    );
    assert_eq!(w.round_records()[0].results.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn forced_end_by_timer_and_by_the_api() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    into_round(&mut a, &mut b).await;
    for _ in 0..7 {
        sleep(500.0).await;
        pings(&mut [&mut a, &mut b]).await;
    }
    assert!(quiet(&mut a).await.is_empty());
    sleep(89.0).await;
    assert!(quiet(&mut a).await.is_empty());
    sleep(1.0).await;
    assert_eq!(
        quiet(&mut a).await,
        s(&["> 本轮游戏将在 10 秒后强制结束。"])
    );
    sleep(10.0).await;
    let mut expected = round_end_tail();
    expected.push(COUNTDOWN_150.to_owned());
    assert_eq!(quiet(&mut a).await, expected);
    assert!(
        w.logs
            .contains("INFO Round in r1 ended: 0 of 2 player(s) submitted a record")
    );
    assert_eq!(w.round_records().len(), 1);
    assert!(w.round_records()[0].results.is_empty());
    b.drain().await;

    assert_eq!(
        w.ctx.end_round("r1").await,
        Err(zphira_game::EndRoundError::NotPlaying)
    );
    into_round(&mut a, &mut b).await;
    w.ctx.end_round("r1").await.unwrap();
    let mut expected = round_end_tail();
    expected[3] = "> 当前谱池 #1 已进行 2/5 轮。".to_owned();
    expected.push(COUNTDOWN_150.to_owned());
    assert_eq!(quiet(&mut a).await, expected);
}

#[tokio::test(start_paused = true)]
async fn disconnect_and_resume_while_voting() {
    let w = World::new().await;
    let (mut a, b) = room_of_two(&w).await;
    b.shutdown().await;
    settle().await;
    assert_eq!(a.lines().await, s(&["> 在线玩家不足，开局倒计时已取消。"]));
    let mut b = w.connect().await;
    b.send(Sb::Authenticate {
        token: "token-200-Bob".into(),
    })
    .await;
    let mut expected = s(&[
        "Authenticate ok 200 Bob room=r1:SelectChart:100,200",
        SEP,
        "> 欢迎加入 Zenith 音游战队 服务器！",
        "> 玩家：Bob（#200）",
        "> 当前积分：0，积分排名：#2",
        "> 输入房间名可创建或加入房间，房间名仅限字母、数字、-、_。",
        SEP,
        "host true",
    ]);
    expected.push(COUNTDOWN_150.to_owned());
    assert_eq!(b.lines().await, expected);
    assert_eq!(a.lines().await, s(&[COUNTDOWN_150]));
}

#[tokio::test(start_paused = true)]
async fn disconnect_and_resume_in_the_ready_phase() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    into_ready(&mut a, &mut b).await;
    b.shutdown().await;
    settle().await;
    assert_eq!(a.lines().await, s(&["msg CancelReady 200"]));
    let mut b = w.connect().await;
    b.send(Sb::Authenticate {
        token: "token-200-Bob".into(),
    })
    .await;
    let lines = b.lines().await;
    assert_eq!(
        lines[0],
        "Authenticate ok 200 Bob room=r1:SelectChart(1):100,200"
    );
    assert_eq!(lines.last().unwrap(), "host true");
    assert_eq!(lines.len(), 8);
    sleep(0.1).await;
    assert_eq!(b.lines().await, s(&["state WaitForReady"]));
    a.send(Sb::Ready).await;
    a.drain().await;
    b.drain().await;
    b.shutdown().await;
    settle().await;
    let lines = a.lines().await;
    assert_eq!(
        lines,
        s(&["msg CancelReady 200", "> 全员已准备，5 秒后开始游戏。"])
    );
    sleep(5.0).await;
    assert_eq!(a.lines().await, s(&["state Playing", "msg StartPlaying"]));
}

#[tokio::test(start_paused = true)]
async fn disconnect_and_resume_while_playing() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    into_round(&mut a, &mut b).await;
    b.shutdown().await;
    settle().await;
    assert_eq!(a.lines().await, s(&["msg Abort 200"]));
    let mut b = w.connect().await;
    b.send(Sb::Authenticate {
        token: "token-200-Bob".into(),
    })
    .await;
    let lines = b.lines().await;
    assert_eq!(
        lines[0],
        "Authenticate ok 200 Bob room=r1:SelectChart:100,200"
    );
    assert_eq!(lines.last().unwrap(), "host true");
    b.send(Sb::Played { id: 9002 }).await;
    assert_eq!(b.lines().await, s(&["Played ok"]));
    assert!(w.upstream.calls().iter().all(|c| c != "record 9002"));
}

#[tokio::test(start_paused = true)]
async fn duplicate_login_takes_over_the_session() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    let mut a2 = w.connect().await;
    a2.send(Sb::Authenticate {
        token: "token-100-Alice".into(),
    })
    .await;
    let lines = a2.lines().await;
    assert_eq!(
        lines[0],
        "Authenticate ok 100 Alice room=r1:SelectChart:100,200"
    );
    assert_eq!(lines.last().unwrap(), "host true");
    assert_eq!(a.until_closed().await, s(&["> 账号在其他地方登录"]));
    settle().await;
    assert!(w.logs.lines().iter().any(|l| l.ends_with("(DUPLICATE)")));
    assert!(b.lines().await.is_empty());
    a2.send(Sb::SelectChart { id: 1 }).await;
    assert_eq!(a2.lines().await.last().unwrap(), "SelectChart ok");
    assert_eq!(w.ctx.player_room(100).as_deref(), Some("r1"));
}

#[tokio::test(start_paused = true)]
async fn kick_paths() {
    let w = World::new().await;
    let (mut a, mut b) = room_of_two(&w).await;
    b.send(Sb::CreateRoom {
        room_id: "x".into(),
    })
    .await;
    assert!(b.closed().await);
    settle().await;
    assert_eq!(
        a.lines().await,
        s(&[
            "msg LeaveRoom 200 Bob",
            HINT,
            "> 在线玩家不足，开局倒计时已取消。"
        ])
    );
    assert_eq!(w.ctx.player_room(200), None);
    assert_eq!(w.ctx.registered_count(), 1);
    a.send(Sb::Authenticate { token: "x".into() }).await;
    assert!(a.closed().await);
    settle().await;
    assert!(!w.ctx.room_exists("r1"));
    let mut c = w.connect().await;
    c.send(Sb::Chat {
        message: "hi".into(),
    })
    .await;
    assert!(c.closed().await);
    settle().await;
    assert!(w.logs.lines().last().unwrap().ends_with("(ACTIVE)"));
}

#[tokio::test(start_paused = true)]
async fn room_full_locked_and_limit() {
    let mut setup = Setup::default();
    setup.args.max_rooms = 1;
    let w = World::with(setup).await;
    let mut a = w.login(100, "Alice").await;
    a.send(Sb::CreateRoom {
        room_id: "r1".into(),
    })
    .await;
    a.drain().await;
    let mut b = w.login(200, "Bob").await;
    b.send(Sb::CreateRoom {
        room_id: "r2".into(),
    })
    .await;
    assert_eq!(
        b.lines().await,
        s(&["CreateRoom failed: 服务器房间数已达上限（1），无法继续创建。"])
    );
    b.send(Sb::CreateRoom {
        room_id: "r1".into(),
    })
    .await;
    assert_eq!(b.lines().await, s(&["CreateRoom failed: 房间已存在"]));
    b.send(Sb::CreateRoom {
        room_id: "bad room".into(),
    })
    .await;
    assert_eq!(
        b.lines().await,
        s(&["CreateRoom failed: 房间名仅限字母、数字、-、_。"])
    );

    w.ctx
        .update_room(
            "r1",
            &zphira_game::RoomUpdate {
                max_player: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
    b.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    assert_eq!(b.lines().await, s(&["JoinRoom failed: 房间已满"]));

    w.ctx
        .update_room(
            "r1",
            &zphira_game::RoomUpdate {
                max_player: Some(10),
                locked: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(w.ctx.room("r1").unwrap().locked);
    b.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    assert_eq!(b.lines().await, s(&["JoinRoom failed: 房间已锁定"]));
    b.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: true,
    })
    .await;
    let lines = b.lines().await;
    assert!(
        lines.contains(&"JoinRoom ok SelectChart [100,200m] live=false".to_owned()),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn rank_room_and_room_not_found_hints() {
    let w = World::new().await;
    w.points.add_points(7, "Seven", 30);
    let mut a = w.login(100, "Alice").await;
    a.send(Sb::JoinRoom {
        room_id: "nope".into(),
        monitor: false,
    })
    .await;
    assert_eq!(
        a.lines().await,
        s(&[
            "> 房间 \"nope\" 不存在。",
            "> 当前没有房间，可创建一个。",
            "JoinRoom failed: 房间不存在",
        ])
    );
    a.send(Sb::CreateRoom {
        room_id: "rank".into(),
    })
    .await;
    assert_eq!(
        a.lines().await,
        s(&[
            "CreateRoom failed: 排行榜已发送",
            SEP,
            "> 积分排行榜 TOP 10",
            "> 排行榜每一分钟刷新一次，可能有滞后。",
            "> #1 Seven：30 分",
            "> #2 Alice：0 分",
            "> 你的积分：0，积分排名：#2",
            SEP,
        ])
    );
    w.ctx
        .create_room("r1", zphira_game::RoomPools::Default)
        .unwrap();
    let mut b = w.login(200, "Bob").await;
    b.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    b.drain().await;
    a.send(Sb::JoinRoom {
        room_id: "zzz".into(),
        monitor: false,
    })
    .await;
    assert_eq!(
        a.lines().await,
        s(&[
            "> 房间 \"zzz\" 不存在。",
            "> 当前可用房间：",
            "> - r1（在线 1/1 人）",
            "JoinRoom failed: 房间不存在",
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn n17_select_countdown_is_read_when_the_countdown_starts() {
    let w = World::new().await;
    w.ctx
        .create_room("r1", zphira_game::RoomPools::Default)
        .unwrap();
    w.ctx
        .update_room(
            "r1",
            &zphira_game::RoomUpdate {
                select_countdown: Some(30),
                ready_countdown: Some(20),
                ..Default::default()
            },
        )
        .unwrap();
    let mut a = w.login(100, "Alice").await;
    let mut b = w.login(200, "Bob").await;
    for c in [&mut a, &mut b] {
        c.send(Sb::JoinRoom {
            room_id: "r1".into(),
            monitor: false,
        })
        .await;
        c.drain().await;
    }
    let lines = a.lines().await;
    assert!(
        lines.contains(
            &"> 已达到开局人数，30 秒后锁定投票并进入准备阶段。人数不足会取消倒计时。".to_owned()
        ),
        "{lines:?}"
    );
    sleep(30.0).await;
    let lines = a.lines().await;
    assert!(
        lines.contains(
            &"> 进入准备阶段：请在 20 秒内准备。未准备玩家将作为观战跳过本轮。".to_owned()
        ),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn normal_round_matches_the_recorded_java_transcript() {
    let fixture = include_str!("fixtures/java_normal_round.txt");
    let mut java: Vec<(String, Vec<String>)> = Vec::new();
    for line in fixture.lines() {
        if let Some(step) = line.strip_prefix("== ") {
            java.push((step.to_owned(), Vec::new()));
        } else {
            java.last_mut().unwrap().1.push(line.replace("\\n", "\n"));
        }
    }
    let w = World::new().await;
    w.upstream.record(alice_record());
    w.upstream.record(bob_record());
    w.upstream.user("tok100", 100, "Alice");
    w.upstream.user("tok200", 200, "Bob");
    let mut rust: Vec<(String, Vec<String>)> = Vec::new();
    let mut a = w.connect().await;
    let mut b = w.connect().await;
    a.send(Sb::Authenticate {
        token: "tok100".into(),
    })
    .await;
    rust.push(("A login".into(), a.lines().await));
    b.send(Sb::Authenticate {
        token: "tok200".into(),
    })
    .await;
    rust.push(("B login".into(), b.lines().await));
    a.send(Sb::CreateRoom {
        room_id: "r1".into(),
    })
    .await;
    rust.push(("A create".into(), a.lines().await));
    no_all_ready_wait(&w, "r1");
    b.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    rust.push(("B join".into(), b.lines().await));
    rust.push(("A on B join".into(), a.lines().await));
    a.send(Sb::SelectChart { id: 1 }).await;
    rust.push(("A vote".into(), a.lines().await));
    rust.push(("B on A vote".into(), b.lines().await));
    b.send(Sb::SelectChart { id: 1 }).await;
    rust.push(("B vote".into(), b.lines().await));
    rust.push(("A on B vote".into(), a.lines().await));
    sleep(10.5).await;
    rust.push(("A countdown".into(), a.lines().await));
    rust.push(("B countdown".into(), b.lines().await));
    a.send(Sb::Ready).await;
    rust.push(("A ready".into(), a.lines().await));
    rust.push(("B on A ready".into(), b.lines().await));
    b.send(Sb::Ready).await;
    rust.push(("B ready".into(), b.lines().await));
    rust.push(("A on B ready".into(), a.lines().await));
    a.send(Sb::Played { id: 9001 }).await;
    rust.push(("A played".into(), a.lines().await));
    rust.push(("B on A played".into(), b.lines().await));
    b.send(Sb::Played { id: 9002 }).await;
    rust.push(("B played".into(), b.lines().await));
    rust.push(("A on B played".into(), a.lines().await));
    assert_eq!(rust.len(), java.len());
    for ((rs, rl), (js, jl)) in rust.iter().zip(&java) {
        assert_eq!(rs, js);
        assert_eq!(rl, jl, "step {rs}");
    }
}

fn java_steps(fixture: &str) -> Vec<(String, Vec<String>)> {
    let mut steps: Vec<(String, Vec<String>)> = Vec::new();
    for line in fixture.lines() {
        if let Some(step) = line.strip_prefix("== ") {
            steps.push((step.to_owned(), Vec::new()));
        } else {
            steps.last_mut().unwrap().1.push(line.replace("\\n", "\n"));
        }
    }
    steps
}

#[tokio::test(start_paused = true)]
async fn variants_match_the_recorded_java_transcript() {
    let mut java = java_steps(include_str!("fixtures/java_variants.txt"));
    for line in java[0].1.iter_mut() {
        if line == "> 当前积分：0，积分排名：#2" {
            *line = "> 当前积分：0，积分排名：#3".to_owned();
        }
    }
    let w = World::new().await;
    w.upstream.failing_records.lock().unwrap().insert(7777);
    let mut a = w.login(100, "Alice").await;
    let mut b = w.login(200, "Bob").await;
    let mut c = w.login(300, "Carol").await;
    a.send(Sb::CreateRoom {
        room_id: "r1".into(),
    })
    .await;
    a.drain().await;
    no_all_ready_wait(&w, "r1");
    b.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    b.drain().await;
    a.drain().await;
    let mut rust: Vec<(String, Vec<String>)> = Vec::new();
    c.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: true,
    })
    .await;
    rust.push(("C join monitor".into(), c.lines().await));
    rust.push(("A on C join".into(), a.lines().await));
    c.send(Sb::SelectChart { id: 1 }).await;
    rust.push(("C vote".into(), c.lines().await));
    a.send(Sb::SelectChart { id: 3 }).await;
    rust.push(("A vote outside".into(), a.lines().await));
    b.send(Sb::RequestStart).await;
    rust.push(("B start".into(), b.lines().await));
    a.send(Sb::SelectChart { id: 2 }).await;
    rust.push(("A vote 2".into(), a.lines().await));
    b.drain().await;
    c.drain().await;
    a.send(Sb::RequestStart).await;
    rust.push(("A start".into(), a.lines().await));
    rust.push(("C on A start".into(), c.lines().await));
    b.drain().await;
    a.send(Sb::RequestStart).await;
    rust.push(("A start again".into(), a.lines().await));
    b.send(Sb::CancelReady).await;
    rust.push(("B cancel ready".into(), b.lines().await));
    a.drain().await;
    c.drain().await;
    a.send(Sb::Ready).await;
    rust.push(("A ready".into(), a.lines().await));
    b.drain().await;
    c.drain().await;
    c.send(Sb::LeaveRoom).await;
    rust.push(("C leave".into(), c.lines().await));
    rust.push(("A on C leave".into(), a.lines().await));
    rust.push(("B on C leave".into(), b.lines().await));
    b.send(Sb::Ready).await;
    rust.push(("B ready".into(), b.lines().await));
    rust.push(("A on B ready".into(), a.lines().await));
    a.send(Sb::Played { id: 7777 }).await;
    rust.push(("A played fails".into(), a.lines().await));
    rust.push(("B on A played".into(), b.lines().await));
    b.send(Sb::Chat {
        message: "hello".into(),
    })
    .await;
    rust.push(("B chat while playing".into(), b.lines().await));
    a.drain().await;
    b.send(Sb::Abort).await;
    rust.push(("B abort".into(), b.lines().await));
    rust.push(("A on B abort".into(), a.lines().await));
    b.send(Sb::CycleRoom { cycle: true }).await;
    rust.push(("B cycle".into(), b.lines().await));
    a.send(Sb::CycleRoom { cycle: true }).await;
    rust.push(("A cycle".into(), a.lines().await));
    rust.push(("B on A cycle".into(), b.lines().await));
    b.send(Sb::LockRoom { lock: true }).await;
    rust.push(("B lock".into(), b.lines().await));
    b.send(Sb::LeaveRoom).await;
    rust.push(("B leave".into(), b.lines().await));
    rust.push(("A on B leave".into(), a.lines().await));
    assert_eq!(rust.len(), java.len());
    for ((rs, rl), (js, jl)) in rust.iter().zip(&java) {
        assert_eq!(rs, js);
        assert_eq!(rl, jl, "step {rs}");
    }
}

#[tokio::test(start_paused = true)]
async fn failures_use_the_players_language() {
    let w = World::new().await;
    w.upstream.user_with_language("tok-en", 7, "Eve", "en-US");
    let mut c = w.connect().await;
    c.send(Sb::Authenticate {
        token: "tok-en".into(),
    })
    .await;
    c.drain().await;
    c.send(Sb::JoinRoom {
        room_id: "none".into(),
        monitor: false,
    })
    .await;
    let lines = c.lines().await;
    assert_eq!(lines.last().unwrap(), "JoinRoom failed: Room not found");
    c.send(Sb::CreateRoom {
        room_id: "a b".into(),
    })
    .await;
    assert_eq!(
        c.lines().await,
        s(&["CreateRoom failed: 房间名仅限字母、数字、-、_。"])
    );
    let mut d = w.connect().await;
    d.send(Sb::Authenticate {
        token: "nope".into(),
    })
    .await;
    assert_eq!(
        d.until_closed().await,
        s(&[r#"Authenticate failed: HTTP 401: {"error":"invalid token"}"#])
    );
}

fn no_all_ready_wait(w: &World, room: &str) {
    w.ctx
        .update_room(
            room,
            &zphira_game::RoomUpdate {
                all_ready_delay: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
}
