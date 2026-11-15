use std::time::Duration;

use super::*;
use crate::testutil::Fixture;

fn lines(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn normalize_trims_and_skips_blank_lines() {
    assert_eq!(normalize(" \t room list \r\n"), Some("room list"));
    assert_eq!(normalize("   "), None);
    assert_eq!(normalize(""), None);
    assert_eq!(normalize(" stop "), Some("stop"));
}

#[test]
fn words_split_on_whitespace_runs_and_keep_the_rest_of_a_limited_split() {
    assert_eq!(
        words("room  create\tr1   1", usize::MAX),
        ["room", "create", "r1", "1"]
    );
    assert_eq!(
        words("submission reject 3 1001 not good  enough", 5),
        ["submission", "reject", "3", "1001", "not good  enough"]
    );
    assert_eq!(words("submission list", 5), ["submission", "list"]);
    assert_eq!(words("a b c d e", 5), ["a", "b", "c", "d", "e"]);
    assert_eq!(words("a b c d", 5), ["a", "b", "c", "d"]);
}

#[test]
fn stop_is_matched_in_any_case_and_only_as_a_whole_word() {
    assert!(is_stop("STOP"));
    assert!(is_stop("Stop"));
    assert!(!is_stop("Stop-not"));
    assert!(!is_stop("stops"));
}

#[tokio::test]
async fn stop_ends_the_console_in_any_case() {
    let f = Fixture::new().await;
    assert_eq!(f.run_flow("stop").await, (Flow::Stop, vec![]));
    assert_eq!(f.run_flow("  STOP ").await, (Flow::Stop, vec![]));
    assert_eq!(f.run_flow("Stop-not").await.0, Flow::Continue);
}

#[tokio::test]
async fn unknown_and_blank_input() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("foo bar").await,
        lines(&["WARN Unknown command: foo bar"])
    );
    assert_eq!(f.run("   ").await, Vec::<String>::new());
    assert_eq!(
        f.run("Stop-not").await,
        lines(&["WARN Unknown command: Stop-not"])
    );
    assert_eq!(f.run("room").await, lines(&["WARN Unknown command: room"]));
    assert_eq!(
        f.run("admin").await,
        lines(&["WARN Unknown command: admin"])
    );
    assert_eq!(f.run("say").await, lines(&["WARN Unknown command: say"]));
    assert_eq!(
        f.run("room\tlist").await,
        lines(&["WARN Unknown command: room\tlist"])
    );
}

#[tokio::test]
async fn say_prints_nothing() {
    let f = Fixture::new().await;
    assert_eq!(f.run("say hello everyone  ").await, Vec::<String>::new());
    assert_eq!(f.run("SAY upper case").await, Vec::<String>::new());
}

#[tokio::test]
async fn admin_commands() {
    let f = Fixture::new().await;
    assert_eq!(f.run("admin list").await, lines(&["INFO Admins: [105]"]));
    assert_eq!(f.run("ADMIN LIST").await, lines(&["INFO Admins: [105]"]));
    assert_eq!(
        f.run("admin add 106").await,
        lines(&["INFO Added admin 106"])
    );
    assert_eq!(
        f.run("admin add 106").await,
        lines(&["WARN User 106 is already an admin"])
    );
    assert_eq!(
        f.run("admin list").await,
        lines(&["INFO Admins: [105, 106]"])
    );
    assert_eq!(
        f.run("admin remove 106").await,
        lines(&["INFO Removed admin 106"])
    );
    assert_eq!(
        f.run("admin remove 999").await,
        lines(&["WARN User 999 is not an admin"])
    );
    assert_eq!(
        f.run("admin add abc").await,
        lines(&["WARN Admin command failed: Invalid number: abc"])
    );
    assert_eq!(
        f.run("admin foo").await,
        lines(&["WARN Usage: admin list | admin add <userId> | admin remove <userId>"])
    );
    assert_eq!(
        f.run("admin add").await,
        lines(&["WARN Usage: admin list | admin add <userId> | admin remove <userId>"])
    );
    assert_eq!(
        f.run("admin remove 105").await,
        lines(&["INFO Removed admin 105"])
    );
    assert_eq!(f.run("admin list").await, lines(&["INFO No admins"]));
}

const USAGE_ROOM_TEXT: &str = "WARN Usage: room list | room create <id> [poolId...] | room remove <id> | room <id> end | room <id> clear | room <id> config <key> <value> | room <id> pool switch <poolId> | room <id> pool favorite <favoriteId|none>";

#[tokio::test]
async fn room_list_create_remove() {
    let f = Fixture::new().await;
    assert_eq!(f.run("room list").await, lines(&["INFO No rooms"]));
    assert_eq!(
        f.run("room create c1").await,
        lines(&["INFO Created room c1"])
    );
    assert_eq!(
        f.run("room create c1").await,
        lines(&["WARN Room command failed: Room already exists: c1"])
    );
    assert_eq!(
        f.run("room create bad!").await,
        lines(&[
            "WARN Room command failed: Room id must be letters, digits, - or _ with length <= 20"
        ])
    );
    assert_eq!(
        f.run("room create abcdefghijklmnopqrstu").await,
        lines(&[
            "WARN Room command failed: Room id must be letters, digits, - or _ with length <= 20"
        ])
    );
    assert_eq!(
        f.run("room create c3 99").await,
        lines(&["WARN Room command failed: Pool not found: 99"])
    );
    assert_eq!(
        f.run("room create c4 4").await,
        lines(&["WARN Room command failed: Room chart pool list cannot be empty"])
    );
    assert_eq!(
        f.run("room create c5 x").await,
        lines(&["WARN Room command failed: Invalid number: x"])
    );
    assert_eq!(
        f.run("room create bad! x").await,
        lines(&[
            "WARN Room command failed: Room id must be letters, digits, - or _ with length <= 20"
        ])
    );
    assert_eq!(
        f.run("room create c6 99 x").await,
        lines(&["WARN Room command failed: Pool not found: 99"])
    );
    assert_eq!(
        f.run("room list").await,
        lines(&["INFO Room c1 [SelectChart] chart=- players=0/0 monitors=0 pool=1 rounds=0/2"])
    );
    assert_eq!(
        f.run("room remove c1").await,
        lines(&["INFO Removed room c1"])
    );
    assert_eq!(
        f.run("room remove c1").await,
        lines(&["WARN Room command failed: Room not found: c1"])
    );
    assert_eq!(f.run("room list").await, lines(&["INFO No rooms"]));
}

#[tokio::test]
async fn a_pool_named_twice_is_one_pool_a30() {
    let f = Fixture::new().await;
    f.run("room create dup 1 1 2").await;
    let view = f.game.room("dup").unwrap();
    let ids: Vec<i32> = view.pool.pools.iter().map(|p| p.id).collect();
    assert_eq!(ids, [1, 2]);
    f.run("room create one 2 2").await;
    let ids: Vec<i32> = f
        .game
        .room("one")
        .unwrap()
        .pool
        .pools
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, [2]);
}

#[tokio::test]
async fn room_remove_cleans_up_like_http_a31() {
    let f = Fixture::new().await;
    f.run("room create gone").await;
    f.game.flush_rooms().await;
    assert!(
        std::fs::read_to_string(f.dir.path().join("rooms.json"))
            .unwrap()
            .contains("gone")
    );
    f.run("room remove gone").await;
    assert!(!f.game.room_exists("gone"));
    f.game.flush_rooms().await;
    assert!(
        !std::fs::read_to_string(f.dir.path().join("rooms.json"))
            .unwrap()
            .contains("gone")
    );
}

#[tokio::test]
async fn room_config() {
    let f = Fixture::new().await;
    f.run("room create c1").await;
    for (cmd, msg) in [
        ("room c1 config minPlayer 0", "minPlayer must be positive"),
        ("room c1 config maxPlayer 0", "maxPlayer must be positive"),
        (
            "room c1 config selectCountdown 9",
            "selectCountdown must be at least 10",
        ),
        (
            "room c1 config readyCountdown 0",
            "readyCountdown must be positive",
        ),
        (
            "room c1 config forceFinish -1",
            "forceFinish must be positive",
        ),
        ("room c1 config interval 0", "interval must be positive"),
    ] {
        assert_eq!(
            f.run(cmd).await,
            lines(&[&format!("WARN Room command failed: {msg}")]),
            "{cmd}"
        );
    }
    assert_eq!(
        f.run("room c1 config bogus 1").await,
        lines(&[
            "WARN Room command failed: Unknown config key: bogus. Keys: minPlayer, maxPlayer, selectCountdown, readyCountdown, forceFinish, interval, cancelReady, startAllReady, allReadyDelay"
        ])
    );
    assert_eq!(
        f.run("room c1 config bogus x").await,
        lines(&["WARN Room command failed: Invalid number: x"])
    );
    assert_eq!(
        f.run("room c1 config interval x").await,
        lines(&["WARN Room command failed: Invalid number: x"])
    );
    assert_eq!(
        f.run("room c1 config selectcountdown 10").await,
        lines(&["INFO Room c1 config selectcountdown = 10"])
    );
    assert_eq!(
        f.run("room c1 config interval +3").await,
        lines(&["INFO Room c1 config interval = +3"])
    );
    let view = f.game.room("c1").unwrap();
    assert_eq!(view.config.select_countdown, 10);
    assert_eq!(view.config.interval, 3);
    assert_eq!(
        f.run("room nope config interval x").await,
        lines(&["WARN Room command failed: Room not found: nope"])
    );
    assert_eq!(f.run("room c1 config").await, lines(&[USAGE_ROOM_TEXT]));
    assert_eq!(
        f.run("room c1 config interval").await,
        lines(&[USAGE_ROOM_TEXT])
    );
}

#[tokio::test]
async fn room_config_ready_settings() {
    let f = Fixture::new().await;
    f.run("room create c1").await;
    let view = f.game.room("c1").unwrap();
    assert!(view.config.cancel_ready_in_vote && view.config.start_when_all_ready);
    assert_eq!(view.config.all_ready_delay, 5);

    for (cmd, expected) in [
        (
            "room c1 config cancelready off",
            "INFO Room c1 config cancelready = off",
        ),
        (
            "room c1 config startAllReady FALSE",
            "INFO Room c1 config startAllReady = FALSE",
        ),
        (
            "room c1 config allreadydelay 0",
            "INFO Room c1 config allreadydelay = 0",
        ),
    ] {
        assert_eq!(f.run(cmd).await, lines(&[expected]), "{cmd}");
    }
    let config = f.game.room("c1").unwrap().config;
    assert!(!config.cancel_ready_in_vote && !config.start_when_all_ready);
    assert_eq!(config.all_ready_delay, 0);

    for (cmd, expected) in [
        (
            "room c1 config cancelReady 1",
            "INFO Room c1 config cancelReady = 1",
        ),
        (
            "room c1 config STARTALLREADY On",
            "INFO Room c1 config STARTALLREADY = On",
        ),
        (
            "room c1 config allReadyDelay 300",
            "INFO Room c1 config allReadyDelay = 300",
        ),
    ] {
        assert_eq!(f.run(cmd).await, lines(&[expected]), "{cmd}");
    }
    let config = f.game.room("c1").unwrap().config;
    assert!(config.cancel_ready_in_vote && config.start_when_all_ready);
    assert_eq!(config.all_ready_delay, 300);
}

#[tokio::test]
async fn room_config_ready_settings_refuse_bad_values() {
    let f = Fixture::new().await;
    f.run("room create c1").await;
    assert_eq!(
        f.run("room c1 config cancelready maybe").await,
        lines(&["WARN Room command failed: Unknown cancelReady value: maybe. Use on or off"])
    );
    assert_eq!(
        f.run("room c1 config startallready 2").await,
        lines(&["WARN Room command failed: Unknown startAllReady value: 2. Use on or off"])
    );
    assert_eq!(
        f.run("room c1 config allreadydelay 301").await,
        lines(&["WARN Room command failed: allReadyDelay must be between 0 and 300"])
    );
    assert_eq!(
        f.run("room c1 config allreadydelay -1").await,
        lines(&["WARN Room command failed: allReadyDelay must be between 0 and 300"])
    );
    assert_eq!(
        f.run("room c1 config allreadydelay on").await,
        lines(&["WARN Room command failed: Invalid number: on"])
    );
    assert_eq!(
        f.run("room nope config cancelready x").await,
        lines(&["WARN Room command failed: Room not found: nope"])
    );
    assert_eq!(
        f.run("room c1 config cancelready").await,
        lines(&[USAGE_ROOM_TEXT])
    );
    let config = f.game.room("c1").unwrap().config;
    assert!(config.cancel_ready_in_vote && config.start_when_all_ready);
    assert_eq!(config.all_ready_delay, 5);
}

#[tokio::test]
async fn room_pool_switch_and_favorite() {
    let f = Fixture::new().await;
    f.run("room create c1 1 2").await;
    assert_eq!(
        f.run("room c1 pool switch 2").await,
        lines(&["INFO Room c1 pending pool set to 2"])
    );
    assert_eq!(
        f.run("room c1 pool switch 3").await,
        lines(&["WARN Room command failed: Pool not in room chart pool list: 3"])
    );
    assert_eq!(
        f.run("room nope pool switch x").await,
        lines(&["WARN Room command failed: Invalid number: x"])
    );
    assert_eq!(
        f.run("room nope pool switch 1").await,
        lines(&["WARN Room command failed: Room not found: nope"])
    );
    assert_eq!(
        f.run("room c1 pool favorite 55").await,
        lines(&["INFO Set favorite_id for room c1 current pool to 55"])
    );
    assert_eq!(
        f.run("room c1 pool favorite NONE").await,
        lines(&["INFO Cleared favorite_id for room c1 current pool"])
    );
    assert_eq!(
        f.run("room c1 pool favorite x").await,
        lines(&["WARN Room command failed: Invalid number: x"])
    );
    assert_eq!(
        f.run("room nope pool favorite x").await,
        lines(&["WARN Room command failed: Room not found: nope"])
    );
    assert_eq!(
        f.run("room c1 pool bogus 1").await,
        lines(&[USAGE_ROOM_TEXT])
    );
    assert_eq!(
        f.run("room c1 pool switch").await,
        lines(&[USAGE_ROOM_TEXT])
    );
}

#[tokio::test]
async fn room_end() {
    let f = Fixture::new().await;
    f.run("room create c1").await;
    assert_eq!(
        f.run("room c1 end").await,
        lines(&["WARN Room command failed: Room is not playing"])
    );
    assert_eq!(
        f.run("room nope end").await,
        lines(&["WARN Room command failed: Room not found: nope"])
    );
    assert_eq!(
        f.run("room c1 END now please").await,
        lines(&["WARN Room command failed: Room is not playing"])
    );
}

#[tokio::test]
async fn room_clear_f1() {
    let f = Fixture::new().await;
    f.run("room create c1").await;
    assert_eq!(
        f.run("room c1 clear").await,
        lines(&["INFO Cleared room c1: 0 member(s) removed"])
    );
    assert_eq!(
        f.run("room C1 CLEAR").await,
        lines(&["WARN Room command failed: Room not found: C1"])
    );
    assert_eq!(
        f.run("room nope clear").await,
        lines(&["WARN Room command failed: Room not found: nope"])
    );
    assert!(f.game.room_exists("c1"));
    assert_eq!(f.run("room c1 clear now").await, lines(&[USAGE_ROOM_TEXT]));
}

#[tokio::test]
async fn room_usage() {
    let f = Fixture::new().await;
    for cmd in [
        "room c1",
        "room list extra words",
        "room remove",
        "room create",
    ] {
        let out = f.run(cmd).await;
        assert_eq!(out.len(), 1, "{cmd}");
        assert!(out[0].starts_with("WARN "), "{cmd}");
    }
    assert_eq!(f.run("room c1").await, lines(&[USAGE_ROOM_TEXT]));
    assert!(USAGE_ROOM_TEXT.contains("room <id> clear"));
}

#[tokio::test]
async fn pool_list() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("pool list").await,
        lines(&[
            "INFO Pool 1 category=MANUAL default=true order=10 sizeLimit=null roundsPerStay=2 favorite_id=77 charts=3:",
            "INFO   Spasmodic | Lv.IN Lv.15 | ID:1001",
            "INFO   Rain | Lv.AT Lv.17 | ID:1002",
            "INFO   Zero | Lv.HD Lv.12 | ID:1003",
            "INFO Pool 2 category=REGULAR default=true order=20 sizeLimit=null roundsPerStay=null favorite_id=null charts=1:",
            "INFO   Two | Lv.IN Lv.16 | ID:2001",
            "INFO Pool 3 category=TB default=false order=30 sizeLimit=null roundsPerStay=null favorite_id=null charts=1:",
            "INFO   Long One | Lv.IN Lv.14 | ID:3001",
            "INFO Pool 4 category=CONFIGURED default=false order=40 sizeLimit=null roundsPerStay=null favorite_id=null charts=0:",
        ])
    );
}

#[tokio::test]
async fn pool_add_remove() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("pool add 20 1001 5002 1001").await,
        lines(&["INFO Added chart pool 20 with 3 chart(s)"])
    );
    assert_eq!(f.pools.find_pool(20).unwrap().chart_ids, [1001, 5002]);
    assert_eq!(
        f.run("pool add 20 1001").await,
        lines(&["WARN Pool command failed: Pool already exists: 20"])
    );
    assert_eq!(
        f.run("pool add 22 x").await,
        lines(&["WARN Pool command failed: Invalid number: x"])
    );
    assert_eq!(
        f.run("pool add x 1001").await,
        lines(&["WARN Pool command failed: Invalid number: x"])
    );
    assert_eq!(
        f.run("pool add 23 4242").await,
        lines(&["WARN Pool command failed: HTTP 404: chart 4242"])
    );
    assert_eq!(
        f.run("pool add 21").await,
        lines(&["INFO Added chart pool 21 with 0 chart(s)"])
    );
    assert!(f.pools.find_pool(21).unwrap().chart_ids.is_empty());
    assert_eq!(
        f.run("pool remove +21").await,
        lines(&["INFO Removed chart pool +21"])
    );
    assert_eq!(
        f.run("pool remove 21").await,
        lines(&["WARN Pool command failed: Pool not found: 21"])
    );
    assert_eq!(f.run("pool add").await, lines(&[USAGE_POOL_TEXT]));
}

const USAGE_POOL_TEXT: &str = "WARN Usage: pool list | pool add <id> [chartIds...] | pool remove <id> | pool chart add <poolId> <chartId> | pool chart remove <poolId> <chartId> | pool favorite <poolId> <favoriteId|none> | pool default <poolId> <on|off> | pool meta <poolId> <category> [sizeLimit] [roundsPerStay] [order] | pool generate <category> [sizeLimit] [roundsPerStay] [probeDuration]";

#[tokio::test]
async fn pool_chart_add_remove() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("pool chart add 4 5001").await,
        lines(&["INFO Added chart to pool 4: Plain A | Lv.AT Lv.17 | ID:5001"])
    );
    assert_eq!(
        f.run("pool chart add 4 5001").await,
        lines(&["INFO Added chart to pool 4: Plain A | Lv.AT Lv.17 | ID:5001"])
    );
    assert_eq!(
        f.run("pool chart add 99 5001").await,
        lines(&["WARN Pool command failed: Pool not found: 99"])
    );
    assert_eq!(
        f.run("pool chart remove 4 5001").await,
        lines(&["INFO Removed chart 5001 from pool 4"])
    );
    assert_eq!(
        f.run("pool chart remove 4 4242").await,
        lines(&["INFO Removed chart 4242 from pool 4"])
    );
    assert_eq!(f.run("pool chart add 4").await, lines(&[USAGE_POOL_TEXT]));
}

#[tokio::test]
async fn pool_favorite() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("pool favorite 2 99").await,
        lines(&["INFO Set favorite_id for pool 2 to 99"])
    );
    assert_eq!(f.pools.find_pool(2).unwrap().favorite_id, Some(99));
    assert_eq!(
        f.run("pool favorite 2 None").await,
        lines(&["INFO Cleared favorite_id for pool 2"])
    );
    assert_eq!(
        f.run("pool favorite 99 1").await,
        lines(&["WARN Pool command failed: Pool not found: 99"])
    );
    assert_eq!(
        f.run("pool favorite 2 x").await,
        lines(&["WARN Pool command failed: Invalid number: x"])
    );
}

#[tokio::test]
async fn pool_default_flag_takes_only_known_words_a31() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("pool default 3 on").await,
        lines(&["INFO Set default flag for pool 3 to true"])
    );
    assert!(f.pools.find_pool(3).unwrap().default_flag);
    assert_eq!(
        f.run("pool default 3 OFF").await,
        lines(&["INFO Set default flag for pool 3 to false"])
    );
    assert_eq!(
        f.run("pool default 3 True").await,
        lines(&["INFO Set default flag for pool 3 to true"])
    );
    assert_eq!(
        f.run("pool default 3 false").await,
        lines(&["INFO Set default flag for pool 3 to false"])
    );
    assert_eq!(
        f.run("pool default 3 1").await,
        lines(&["INFO Set default flag for pool 3 to true"])
    );
    assert!(f.pools.find_pool(3).unwrap().default_flag);
    assert_eq!(
        f.run("pool default 3 0").await,
        lines(&["INFO Set default flag for pool 3 to false"])
    );
    for word in ["enable", "yes", "onn", "2", "-1", "00", "y"] {
        assert_eq!(
            f.run(&format!("pool default 3 {word}")).await,
            lines(&[&format!(
                "WARN Pool command failed: Unknown default flag value: {word}. Use on or off"
            )]),
            "{word}"
        );
    }
    assert!(!f.pools.find_pool(3).unwrap().default_flag);
    assert_eq!(
        f.run("pool default 99 on").await,
        lines(&["WARN Pool command failed: Pool not found: 99"])
    );
    assert_eq!(
        f.run("pool default x enable").await,
        lines(&["WARN Pool command failed: Invalid number: x"])
    );
}

#[tokio::test]
async fn pool_meta() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("pool meta 3 regular 10 2 5").await,
        lines(&["INFO Pool 3 metadata set: category=REGULAR sizeLimit=10 roundsPerStay=2 order=5"])
    );
    let pool = f.pools.find_pool(3).unwrap();
    assert_eq!(
        (
            pool.category,
            pool.size_limit,
            pool.rounds_per_stay,
            pool.order
        ),
        (PoolCategory::Regular, Some(10), Some(2), 5)
    );
    assert_eq!(
        f.run("pool meta 3 TB -1").await,
        lines(&[
            "INFO Pool 3 metadata set: category=TB sizeLimit=-1 roundsPerStay=null order=null"
        ])
    );
    assert_eq!(f.pools.find_pool(3).unwrap().size_limit, None);
    assert_eq!(
        f.run("pool meta 3 bogus").await,
        lines(&[
            "WARN Pool command failed: Unknown category: bogus. Categories: REGULAR, CONFIGURED, TB, MANUAL"
        ])
    );
    assert_eq!(
        f.run("pool meta 99 TB").await,
        lines(&["WARN Pool command failed: Pool not found: 99"])
    );
    assert_eq!(
        f.run("pool meta 3 tb x").await,
        lines(&["WARN Pool command failed: Invalid number: x"])
    );
    assert_eq!(f.run("pool meta 3").await, lines(&[USAGE_POOL_TEXT]));
}

#[tokio::test]
async fn pool_generate() {
    let f = Fixture::new().await;
    let out = f.run("pool generate configured 1 1").await;
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].starts_with("INFO Generated 3 CONFIGURED pools: ["),
        "{out:?}"
    );
    assert_eq!(
        f.run("pool generate MANUAL").await,
        lines(&["WARN Pool command failed: Category has no screening rule: MANUAL"])
    );
    assert_eq!(
        f.run("pool generate bogus").await,
        lines(&[
            "WARN Pool command failed: Unknown category: bogus. Categories: REGULAR, CONFIGURED, TB, MANUAL"
        ])
    );
    assert_eq!(
        f.run("pool generate REGULAR 0").await,
        lines(&["WARN Pool command failed: Pool size limit must be positive"])
    );
    assert_eq!(
        f.run("pool generate REGULAR x").await,
        lines(&["WARN Pool command failed: Invalid number: x"])
    );
    assert_eq!(f.run("pool generate").await, lines(&[USAGE_POOL_TEXT]));
}

#[tokio::test]
async fn pool_generate_probes_first_and_parses_after() {
    let f = Fixture::new().await;
    f.upstream.set_duration(1001, 300);
    let out = f.run("pool generate TB x 1 5").await;
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(out[0].starts_with("INFO Probed "), "{out:?}");
    assert!(out[0].ends_with(" durations for TB"), "{out:?}");
    assert_eq!(out[1], "WARN Pool command failed: Invalid number: x");
    assert_eq!(
        f.run("pool generate TB 5 1 -2").await,
        lines(&["WARN Pool command failed: Probe budget must not be negative: -2"])
    );
}

#[tokio::test]
async fn chart_index_and_refresh() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("chart index").await,
        lines(&["INFO Chart index holds 8 charts (remote total -1, refreshing false)"])
    );
    f.upstream.hold_pages();
    assert_eq!(
        f.run("chart refresh plain").await,
        lines(&["INFO Refreshing chart index in background (division=plain)"])
    );
    assert!(f.index.is_refreshing());
    assert_eq!(
        f.run("chart refresh").await,
        lines(&[
            "INFO Refreshing chart index in background (division=all)",
            "WARN A chart index refresh is already running, request ignored",
        ])
    );
    f.upstream.release_pages();
    for _ in 0..200 {
        if !f.index.is_refreshing() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!f.index.is_refreshing());
    assert_eq!(f.run("chart refresh a b").await, lines(&[USAGE_CHART_TEXT]));
}

const USAGE_CHART_TEXT: &str = "WARN Usage: chart index | chart refresh [plain|full] | chart search <category> [limit] | chart duration <chartIds...>";

async fn wait_refresh_done(f: &Fixture) {
    for _ in 0..300 {
        if !f.index.is_refreshing() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the refresh did not finish");
}

#[tokio::test]
async fn chart_refresh_full_starts_a_full_sweep() {
    let f = Fixture::new().await;
    f.run("chart refresh").await;
    wait_refresh_done(&f).await;
    assert!(f.upstream.page_divisions().iter().all(Option::is_none));

    assert_eq!(
        f.run("chart refresh full").await,
        lines(&["INFO Refreshing chart index in background (full sweep)"])
    );
    wait_refresh_done(&f).await;
    assert!(
        f.upstream
            .page_divisions()
            .contains(&Some("plain".to_owned()))
    );

    f.upstream.hold_pages();
    assert_eq!(
        f.run("CHART REFRESH FULL").await,
        lines(&["INFO Refreshing chart index in background (full sweep)"])
    );
    assert_eq!(
        f.run("chart refresh full").await,
        lines(&[
            "INFO Refreshing chart index in background (full sweep)",
            "WARN A chart index refresh is already running, request ignored",
        ])
    );
    assert_eq!(
        f.run("chart refresh").await,
        lines(&[
            "INFO Refreshing chart index in background (division=all)",
            "WARN A chart index refresh is already running, request ignored",
        ])
    );
    f.upstream.release_pages();
    wait_refresh_done(&f).await;
    assert_eq!(
        f.run("chart refresh full extra").await,
        lines(&[USAGE_CHART_TEXT])
    );
}

#[tokio::test]
async fn chart_search() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("chart search CONFIGURED 2").await,
        lines(&[
            "INFO CONFIGURED matched 2 of 8 indexed charts, showing 2:",
            "INFO   Plain A | AT Lv.17 | rating=0.99 votes=20 | ID:5001",
            "INFO   Long One | IN Lv.14 | rating=0.91 votes=12 | ID:3001",
        ])
    );
    assert_eq!(
        f.run("chart search tb 0").await,
        lines(&["INFO TB matched 0 of 8 indexed charts, showing 0:"])
    );
    assert_eq!(
        f.run("chart search MANUAL").await,
        lines(&["WARN Chart command failed: Category has no screening rule: MANUAL"])
    );
    assert_eq!(
        f.run("chart search bogus").await,
        lines(&[
            "WARN Chart command failed: Unknown category: bogus. Categories: REGULAR, CONFIGURED, TB, MANUAL"
        ])
    );
    assert_eq!(
        f.run("chart search REGULAR -1").await,
        lines(&["WARN Chart command failed: Limit must not be negative: -1"])
    );
    assert_eq!(
        f.run("chart search REGULAR x").await,
        lines(&["WARN Chart command failed: Invalid number: x"])
    );
    let out = f.run("chart search regular").await;
    assert_eq!(
        out[0],
        "INFO REGULAR matched 3 of 8 indexed charts, showing 3:"
    );
}

#[tokio::test]
async fn chart_duration() {
    let f = Fixture::new().await;
    f.upstream.set_duration(1001, 125);
    let out = f.run("chart duration 1001 1002").await;
    assert!(
        out.contains(&"INFO   Spasmodic | IN Lv.15 -> 125s".to_owned()),
        "{out:?}"
    );
    assert!(
        out.contains(&"INFO   Rain | AT Lv.17 -> unknown".to_owned()),
        "{out:?}"
    );
    assert_eq!(out.last().unwrap(), "INFO Probed 1 of 2 chart(s)");
    assert_eq!(
        f.run("chart duration 424242").await,
        lines(&["WARN Chart command failed: Chart not in index: 424242"])
    );
    assert_eq!(
        f.run("chart duration x").await,
        lines(&["WARN Chart command failed: Invalid number: x"])
    );
    assert_eq!(
        f.run("chart duration 424242 x").await,
        lines(&["WARN Chart command failed: Invalid number: x"])
    );
    assert_eq!(f.run("chart bogus").await, lines(&[USAGE_CHART_TEXT]));
}

#[tokio::test]
async fn submission_commands() {
    let f = Fixture::new().await;
    assert_eq!(
        f.run("submission list").await,
        lines(&["INFO 待审投稿: 无"])
    );
    assert_eq!(
        f.run("submission list 3").await,
        lines(&["INFO 池 3 的投稿: 无"])
    );
    f.submissions
        .submit(3, &[5001], 101, Some("Alice"))
        .await
        .unwrap();
    f.submissions.submit(3, &[5001], 102, None).await.unwrap();
    f.submissions
        .submit(3, &[4242], 101, Some("Alice"))
        .await
        .unwrap();
    let pending = f.run("submission list").await;
    assert_eq!(pending[0], "INFO 待审投稿: 2 条");
    assert!(
        pending.contains(
            &"INFO   池3 谱面5001 [PENDING] Plain A | 2 人: Alice(101), null(102)".to_owned()
        ),
        "{pending:?}"
    );
    assert!(
        pending.contains(&"INFO   池3 谱面4242 [PENDING] - | 1 人: Alice(101)".to_owned()),
        "{pending:?}"
    );

    assert_eq!(
        f.run("submission approve 3 5001").await,
        lines(&["INFO Approved chart 5001 into pool 3 from console"])
    );
    assert!(f.pools.find_pool(3).unwrap().chart_ids.contains(&5001));
    let out = f.run("submission approve 3 4242").await;
    assert_eq!(
        out,
        lines(&["WARN Submission command failed: HTTP 404: chart 4242"])
    );
    assert_eq!(f.submissions.list_pending().len(), 1);
    assert_eq!(
        f.run("submission approve 3 9999").await,
        lines(&["WARN Submission command failed: 投稿不存在"])
    );
    assert_eq!(
        f.run("submission reject 3 4242 too long and  spaced").await,
        lines(&["INFO Rejected chart 4242 in pool 3 from console"])
    );
    let views = f.submissions.list_by_pool(3, None);
    let view = views.iter().find(|v| v.chart_id == 4242).unwrap();
    assert_eq!(view.reason.as_deref(), Some("too long and  spaced"));
    assert_eq!(
        f.run("submission reject 3 9999").await,
        lines(&["WARN Submission command failed: 投稿不存在"])
    );
    assert_eq!(
        f.run("submission list x").await,
        lines(&["WARN Submission command failed: Invalid number: x"])
    );
    let usage = lines(&[
        "WARN Usage: submission list | submission list <poolId> | submission approve <poolId> <chartId> | submission reject <poolId> <chartId> [reason]",
    ]);
    assert_eq!(f.run("submission bogus").await, usage);
    assert_eq!(f.run("submission approve 3").await, usage);
    assert_eq!(f.run("submission list 3 4").await, usage);
}
