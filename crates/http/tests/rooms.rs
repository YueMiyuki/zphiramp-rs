mod support;

use serde_json::{Value, json};
use support::{Env, error_body};
use zphira_http::HttpConfig;
use zphira_protocol::{GameState, ServerBoundPacket as Sb};

const NOT_FOUND: &str = "欲请求的内容不存在";

async fn create(env: &Env, room: &str, body: &str) -> Value {
    env.call(
        "POST",
        &format!("/api/v1/room/{room}/create"),
        Some(&env.admin.clone()),
        Some(body),
    )
    .await
    .json()
}

fn room_ids(v: &Value) -> Vec<String> {
    v["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["roomId"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn room_snapshot_is_exact() {
    let env = Env::new().await;
    assert_eq!(create(&env, "r1", "{}").await, json!({"ok":true}));
    let r = env.get("/api/v1/room/r1").await;
    assert_eq!(
        r.text(),
        concat!(
            r#"{"ok":true,"info":{"roomId":"r1","state":"SelectChart","live":false,"locked":false,"cycle":false,"host":null,"chart":null,"type":"local","#,
            r#""config":{"minPlayer":2,"maxPlayer":1000,"selectCountdown":150,"readyCountdown":60,"forceFinish":3600,"interval":5,"adminIds":[],"chatEnable":true,"cancelReadyInVote":true,"startWhenAllReady":true,"allReadyDelay":5},"#,
            r#""effectiveForceFinish":3600,"pool":{"currentPool":{"id":0,"favoriteId":null,"default":true,"chartIds":[1000,1002],"category":"REGULAR","sizeLimit":15,"roundsPerStay":3,"order":10,"submissionOpen":false},"#,
            r#""pools":[{"id":0,"favoriteId":null,"default":true,"chartIds":[1000,1002],"category":"REGULAR","sizeLimit":15,"roundsPerStay":3,"order":10,"submissionOpen":false}],"#,
            r#""pendingPoolId":null,"favoriteId":null,"finishedRoundsSinceRefresh":0,"refreshIntervalRounds":5,"effectiveRoundsPerStay":3},"players":[],"monitors":[]}}"#
        )
    );
    let pool = env.get("/api/v1/room/r1/pool").await.json();
    assert_eq!(pool["ok"], json!(true));
    assert_eq!(
        pool["pool"],
        env.get("/api/v1/room/r1").await.json()["info"]["pool"]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn create_paths_and_log() {
    let env = Env::new().await;
    let (captured, _guard) = support::replay::capture();
    assert_eq!(
        create(&env, "r2", "{\"type\":\"local\",\"pools\":[2,0,2,77]}").await,
        json!({"ok":true})
    );
    let all = captured.take();
    let logs: Vec<String> = all
        .iter()
        .filter(|l| l.contains("Server:"))
        .cloned()
        .collect();
    assert_eq!(
        logs,
        vec!["INFO Server: HTTP create room: r2 pools=[2, 0]".to_owned()],
        "{all:?}"
    );
    for (room, body, reason) in [
        ("r2", "{}", "房间已存在"),
        ("r2", "{\"type\":\"remote\"}", "房间已存在"),
        ("bad!", "{}", "房间名仅限字母、数字、-、_。"),
        (
            "abcdefghijklmnopqrstu",
            "{}",
            "房间名仅限字母、数字、-、_。",
        ),
        ("r3", "{\"type\":\"remote\"}", "不支持的房间类型：remote"),
        ("r3", "{\"pools\":[77,1001]}", "指定的谱池不存在或均为空池"),
        ("r3", "{\"pools\":[null]}", "请检查是否传入了错误的格式"),
        ("r3", "[", "请检查是否传入了错误的格式"),
    ] {
        assert_eq!(
            create(&env, room, body).await,
            error_body(reason),
            "{room} {body}"
        );
    }
    let r = env
        .call(
            "POST",
            "/api/v1/room/r3/create",
            Some(&env.player.clone()),
            Some("{}"),
        )
        .await;
    assert_eq!(r.json(), error_body("需要权限"));
    assert_eq!(
        create(&env, "r3", "{\"pools\":[]}").await,
        json!({"ok":true})
    );
    assert_eq!(
        env.get("/api/v1/room/r3").await.json()["info"]["pool"]["pools"][0]["id"],
        json!(0)
    );
    assert!(env.game.flush_rooms().await);
    let saved = std::fs::read_to_string(env.data.join("rooms.json")).unwrap();
    assert!(
        saved.contains("\"r2\"") && saved.contains("\"r3\""),
        "{saved}"
    );
}

#[tokio::test]
async fn create_at_the_room_limit_is_400() {
    let args = zphira_common::cli::ServerArgs {
        max_rooms: 1,
        ..zphira_common::cli::ServerArgs::default()
    };
    let env = Env::build_with_args(HttpConfig::default(), |_| {}, args).await;
    assert_eq!(create(&env, "a", "{}").await, json!({"ok":true}));
    assert_eq!(
        create(&env, "b", "{}").await,
        error_body("服务器房间数已达上限（1），无法继续创建。")
    );
}

#[tokio::test]
async fn a_21_room_update_is_all_or_nothing() {
    let env = Env::new().await;
    create(&env, "r1", "{}").await;
    let admin = env.admin.clone();
    let before = env.get("/api/v1/room/r1").await.json();
    for (body, reason) in [
        (
            "{\"live\":true,\"minPlayer\":7,\"maxPlayer\":3,\"chatEnable\":false,\"selectCountdown\":5}",
            "selectCountdown 必须大于等于 10",
        ),
        (
            "{\"live\":true,\"minPlayer\":7,\"maxPlayer\":3}",
            "minPlayer 不能大于 maxPlayer",
        ),
        ("{\"minPlayer\":1001}", "minPlayer 不能大于 maxPlayer"),
        (
            "{\"live\":true,\"adminIds\":[3,0]}",
            "管理员 ID 必须是正整数",
        ),
        (
            "{\"live\":true,\"adminIds\":[3,null]}",
            "请检查是否传入了错误的格式",
        ),
        ("{\"minPlayer\":0}", "minPlayer 必须大于 0"),
        ("{\"maxPlayer\":-1}", "maxPlayer 必须大于 0"),
        ("{\"readyCountdown\":0}", "readyCountdown 必须大于 0"),
        ("{\"forceFinish\":0}", "forceFinish 必须大于 0"),
        ("{\"interval\":0}", "interval 必须大于 0"),
        ("", "请检查是否传入了错误的格式"),
    ] {
        let r = env
            .call("PUT", "/api/v1/room/r1/update", Some(&admin), Some(body))
            .await;
        assert_eq!(r.json(), error_body(reason), "{body}");
        assert_eq!(
            env.get("/api/v1/room/r1").await.json(),
            before,
            "{body}: nothing applied"
        );
    }
    let r = env
        .call("PUT", "/api/v1/room/nope/update", Some(&admin), Some("{}"))
        .await;
    assert_eq!((r.status.as_u16(), r.json()), (404, error_body(NOT_FOUND)));
    let r = env
        .call(
            "PUT",
            "/api/v1/room/r1/update",
            Some(&admin),
            Some("{\"minPlayer\":3,\"maxPlayer\":3,\"adminIds\":[5,5,6]}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    let config = env.get("/api/v1/room/r1").await.json()["info"]["config"].clone();
    assert_eq!(
        (config["minPlayer"].clone(), config["maxPlayer"].clone()),
        (json!(3), json!(3))
    );
    assert_eq!(config["adminIds"], json!([5, 6]));
}

#[tokio::test]
async fn a_22_config_reports_chat_enable() {
    let env = Env::new().await;
    create(&env, "r1", "{}").await;
    let chat = |v: Value| v["info"]["config"]["chatEnable"].clone();
    assert_eq!(chat(env.get("/api/v1/room/r1").await.json()), json!(true));
    env.call(
        "PUT",
        "/api/v1/room/r1/update",
        Some(&env.admin.clone()),
        Some("{\"chatEnable\":false}"),
    )
    .await;
    assert_eq!(chat(env.get("/api/v1/room/r1").await.json()), json!(false));
    let list = env.get("/api/v1/room/list").await.json();
    assert_eq!(list["rooms"][0]["config"]["chatEnable"], json!(false));
    let keys: Vec<String> = env.get("/api/v1/room/r1").await.json()["info"]["config"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        [
            "minPlayer",
            "maxPlayer",
            "selectCountdown",
            "readyCountdown",
            "forceFinish",
            "interval",
            "adminIds",
            "chatEnable",
            "cancelReadyInVote",
            "startWhenAllReady",
            "allReadyDelay"
        ]
    );
}

#[tokio::test]
async fn n_16_update_lock_locks_the_room() {
    let env = Env::new().await;
    create(&env, "r1", "{}").await;
    let mut alice = env.player(100, "Alice").await;
    alice.join("r1", false).await;
    let r = env
        .call(
            "PUT",
            "/api/v1/room/r1/update",
            Some(&env.admin.clone()),
            Some("{\"lock\":true}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    assert_eq!(
        env.get("/api/v1/room/r1").await.json()["info"]["locked"],
        json!(true)
    );
    let mut bob = env.player(200, "Bob").await;
    bob.send(Sb::JoinRoom {
        room_id: "r1".into(),
        monitor: false,
    })
    .await;
    let packets = bob
        .until(|p| matches!(p, zphira_protocol::ClientBoundPacket::JoinRoom(_)))
        .await;
    assert!(
        matches!(packets.last(), Some(zphira_protocol::ClientBoundPacket::JoinRoom(r)) if !r.is_success())
    );
    env.call(
        "PUT",
        "/api/v1/room/r1/update",
        Some(&env.admin.clone()),
        Some("{\"lock\":false}"),
    )
    .await;
    assert_eq!(
        env.get("/api/v1/room/r1").await.json()["info"]["locked"],
        json!(false)
    );
    bob.join("r1", false).await;
}

#[tokio::test]
async fn member_views() {
    let env = Env::new().await;
    create(&env, "r1", "{}").await;
    create(&env, "r2", "{\"pools\":[2]}").await;
    let mut alice = env.player(100, "Alice").await;
    alice.join("r1", false).await;
    let mut mon = env.player(300, "Mon").await;
    mon.join("r1", true).await;
    let alice_token = env.state.jwt().issue_token(100, false);
    let mon_token = env.state.jwt().issue_token(300, false);

    let list = env
        .call("GET", "/api/v1/room/list", Some(&alice_token), None)
        .await
        .json();
    assert_eq!(room_ids(&list), ["r1"]);
    let info = env
        .call("GET", "/api/v1/room/r1/", Some(&alice_token), None)
        .await
        .json()["info"]
        .clone();
    assert_eq!(info["host"], json!(100));
    assert_eq!(info["players"], json!([{"id":100,"name":"Alice"}]));
    assert_eq!(info["monitors"], json!([{"id":300,"name":"Mon"}]));
    assert_eq!(
        env.call("GET", "/api/v1/room/r1/pool", Some(&mon_token), None)
            .await
            .status,
        200
    );
    for target in ["/api/v1/room/r2", "/api/v1/room/r2/pool"] {
        let r = env.call("GET", target, Some(&alice_token), None).await;
        assert_eq!(
            (r.status.as_u16(), r.json()),
            (403, error_body("需要权限")),
            "{target}"
        );
    }
    let r = env
        .call("GET", "/api/v1/room/nope", Some(&alice_token), None)
        .await;
    assert_eq!((r.status.as_u16(), r.json()), (404, error_body(NOT_FOUND)));
    let player = env.player.clone();
    assert_eq!(
        room_ids(
            &env.call("GET", "/api/v1/room/list", Some(&player), None)
                .await
                .json()
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        room_ids(&env.get("/api/v1/room/list").await.json()),
        ["r1", "r2"]
    );
    let r = env
        .call(
            "POST",
            "/api/v1/room/r1/say",
            Some(&alice_token),
            Some("{\"message\":\"x\"}"),
        )
        .await;
    assert_eq!(r.json(), error_body("需要权限"));
    let r = env
        .call("DELETE", "/api/v1/room/r1", Some(&env.admin.clone()), None)
        .await;
    assert_eq!(r.json(), error_body("房间非空，无法删除"));
    let r = env
        .call("DELETE", "/api/v1/room/r2", Some(&env.admin.clone()), None)
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
}

#[tokio::test(flavor = "current_thread")]
async fn say_and_broadcast() {
    let env = Env::new().await;
    create(&env, "r1", "{}").await;
    let mut alice = env.player(100, "Alice").await;
    alice.join("r1", false).await;
    let mut mon = env.player(300, "Mon").await;
    mon.join("r1", true).await;
    let mut bob = env.player(200, "Bob").await;
    let (captured, _guard) = support::replay::capture();
    let admin = env.admin.clone();
    let r = env
        .call(
            "POST",
            "/api/v1/room/r1/say",
            Some(&admin),
            Some("{\"message\":\" 第一行\\r\\n第二行 \"}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true,"delivered":1}));
    alice.system_line("第一行 第二行").await;
    mon.system_line("第一行 第二行").await;
    let r = env
        .call(
            "POST",
            "/api/v1/broadcast",
            Some(&admin),
            Some("{\"message\":\"全体\"}"),
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true,"delivered":3}));
    bob.system_line("全体").await;
    alice.system_line("全体").await;
    let logs: Vec<String> = captured
        .take()
        .into_iter()
        .filter(|l| l.contains("Server: HTTP"))
        .collect();
    assert_eq!(
        logs,
        vec![
            "INFO Server: HTTP room message to r1: 第一行 第二行".to_owned(),
            "INFO Server: HTTP global message: 全体".to_owned()
        ]
    );
    for (target, body) in [
        ("/api/v1/room/r1/say", "{\"message\":\"  \"}"),
        ("/api/v1/broadcast", "{}"),
    ] {
        let r = env.call("POST", target, Some(&admin), Some(body)).await;
        assert_eq!(r.json(), error_body("消息不能为空"), "{target}");
    }
    let r = env
        .call(
            "POST",
            "/api/v1/room/nope/say",
            Some(&admin),
            Some("{\"message\":\"x\"}"),
        )
        .await;
    assert_eq!((r.status.as_u16(), r.json()), (404, error_body(NOT_FOUND)));
}

#[tokio::test(flavor = "current_thread")]
async fn end_a_round() {
    let env = Env::new().await;
    create(&env, "r1", "{}").await;
    env.call(
        "PUT",
        "/api/v1/room/r1/update",
        Some(&env.admin.clone()),
        Some("{\"allReadyDelay\":0}"),
    )
    .await;
    let mut alice = env.player(100, "Alice").await;
    alice.join("r1", false).await;
    let admin = env.admin.clone();
    let r = env
        .call("POST", "/api/v1/room/r1/end", Some(&admin), None)
        .await;
    assert_eq!(r.json(), error_body("房间不在游戏中"));
    alice.send(Sb::RequestStart).await;
    alice.state(GameState::WaitForReady).await;
    alice.send(Sb::Ready).await;
    alice.state(GameState::Playing).await;
    assert_eq!(
        env.get("/api/v1/room/r1").await.json()["info"]["state"],
        json!("Playing")
    );
    let (captured, _guard) = support::replay::capture();
    let r = env
        .call("POST", "/api/v1/room/r1/end", Some(&admin), None)
        .await;
    assert_eq!(r.json(), json!({"ok":true}));
    assert!(
        captured
            .take()
            .iter()
            .any(|l| l == "INFO Server: HTTP force end room: r1")
    );
    assert_eq!(
        env.get("/api/v1/room/r1").await.json()["info"]["state"],
        json!("SelectChart")
    );
    let r = env
        .call("POST", "/api/v1/room/nope/end", Some(&admin), None)
        .await;
    assert_eq!((r.status.as_u16(), r.json()), (404, error_body(NOT_FOUND)));
}

#[tokio::test]
async fn pool_switch_and_favorite() {
    let env = Env::new().await;
    create(&env, "r2", "{\"pools\":[2,0]}").await;
    let admin = env.admin.clone();
    let call = |target: &'static str, body: &'static str| {
        let admin = admin.clone();
        let env = &env;
        async move { env.call("PUT", target, Some(&admin), Some(body)).await }
    };
    let r = call("/api/v1/room/r2/pool/switch", "{\"poolId\":1}").await;
    assert_eq!(r.json(), error_body("Pool not in room chart pool list: 1"));
    assert_eq!(
        call("/api/v1/room/r2/pool/switch", "{\"poolId\":0}")
            .await
            .json(),
        json!({"ok":true})
    );
    assert_eq!(
        call("/api/v1/room/r2/pool/switch", "{\"poolId\":null}")
            .await
            .json(),
        error_body("请检查是否传入了错误的格式")
    );
    let r = call("/api/v1/room/nope/pool/switch", "{\"poolId\":0}").await;
    assert_eq!((r.status.as_u16(), r.json()), (404, error_body(NOT_FOUND)));
    assert_eq!(
        call("/api/v1/room/r2/pool/favorite", "{\"favoriteId\":99}")
            .await
            .json(),
        json!({"ok":true})
    );
    assert_eq!(
        call("/api/v1/room/r2/pool/favorite", "{}").await.json(),
        error_body("请检查是否传入了错误的格式")
    );
    let pool = env.get("/api/v1/room/r2/pool").await.json()["pool"].clone();
    assert_eq!(pool["pendingPoolId"], json!(0));
    assert_eq!(pool["favoriteId"], json!(99));
    assert_eq!(pool["currentPool"]["favoriteId"], json!(99));
}

#[tokio::test(flavor = "current_thread")]
async fn f1_clear_a_room() {
    let env = Env::new().await;
    create(&env, "r1", "{}").await;
    let mut alice = env.player(100, "Alice").await;
    alice.join("r1", false).await;
    let mut bob = env.player(200, "Bob").await;
    bob.join("r1", false).await;
    let mut mon = env.player(300, "Mon").await;
    mon.join("r1", true).await;
    let alice_token = env.state.jwt().issue_token(100, false);
    let bob_token = env.state.jwt().issue_token(200, false);

    for token in [bob_token.clone(), env.player.clone()] {
        let r = env
            .call("POST", "/api/v1/room/r1/clear", Some(&token), None)
            .await;
        assert_eq!((r.status.as_u16(), r.json()), (403, error_body("需要权限")));
    }
    let r = env
        .call("POST", "/api/v1/room/nope/clear", Some(&alice_token), None)
        .await;
    assert_eq!((r.status.as_u16(), r.json()), (404, error_body(NOT_FOUND)));
    let r = env.call("POST", "/api/v1/room/r1/clear", None, None).await;
    assert_eq!(r.status, 401);

    let (captured, _guard) = support::replay::capture();
    let r = env
        .call("POST", "/api/v1/room/r1/clear", Some(&alice_token), None)
        .await;
    assert_eq!(r.json(), json!({"ok":true,"removed":3}));
    assert!(
        captured
            .take()
            .iter()
            .any(|l| l == "INFO Server: HTTP clear room: r1 removed=3")
    );
    for client in [&mut alice, &mut bob, &mut mon] {
        assert!(
            client.closed().await,
            "client {} still connected",
            client.id
        );
    }
    let info = env.get("/api/v1/room/r1").await.json()["info"].clone();
    assert_eq!(
        (info["players"].clone(), info["monitors"].clone()),
        (json!([]), json!([]))
    );
    let r = env
        .call(
            "POST",
            "/api/v1/room/r1/clear",
            Some(&env.admin.clone()),
            None,
        )
        .await;
    assert_eq!(r.json(), json!({"ok":true,"removed":0}));
    let mut bob = env.player(200, "Bob").await;
    bob.join("r1", false).await;
}

#[tokio::test]
async fn submission_names_come_from_the_game() {
    let env = Env::new().await;
    let _carol = env.player(7, "Carol").await;
    let token = env.state.jwt().issue_token(7, false);
    let r = env
        .call(
            "POST",
            "/api/v1/pool/2/submissions",
            Some(&token),
            Some("{\"chartIds\":[1004]}"),
        )
        .await;
    assert_eq!(r.json()["accepted"], json!(1));
    let mine = env
        .call("GET", "/api/v1/submission/mine", Some(&token), None)
        .await
        .json();
    assert_eq!(
        mine["submissions"][0]["submitters"][0]["name"],
        json!("Carol")
    );
    let asked = env
        .phira
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|r| r == "GET /user/7");
    assert!(!asked, "Phira was asked for the name");
}
