mod support;

use std::fmt::Debug;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use support::read_tsv;
use zphira_common::time::Timestamp;
use zphira_phira::{ChartInfo, ChartListPage, GameRecord, LoginResult, UserInfo};

fn shape(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let members: Vec<String> = map
                .iter()
                .map(|(key, value)| format!("{key}:{}", shape(value)))
                .collect();
            format!("{{{}}}", members.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(shape).collect();
            format!("[{}]", items.join(","))
        }
        Value::Null => "null".into(),
        _ => "_".into(),
    }
}

fn check<T>(kind: &str, label: &str, input: &str, java_pretty: &str, java_compact: &str)
where
    T: Serialize + DeserializeOwned + Debug + PartialEq,
{
    let context = format!("{kind} / {label} / {input}");
    let read: T = serde_json::from_str(input)
        .unwrap_or_else(|e| panic!("{context}: Java read it, Rust failed: {e}"));

    let from_pretty: T = serde_json::from_str(java_pretty)
        .unwrap_or_else(|e| panic!("{context}: Java's data file does not load: {e}"));
    let from_compact: T = serde_json::from_str(java_compact)
        .unwrap_or_else(|e| panic!("{context}: Java's API output does not load: {e}"));
    assert_eq!(from_pretty, read, "{context}: Java's data file");
    assert_eq!(from_compact, read, "{context}: Java's API output");

    let written = serde_json::to_string_pretty(&read).unwrap();
    let back: T = serde_json::from_str(&written)
        .unwrap_or_else(|e| panic!("{context}: what Rust wrote does not load: {e}"));
    assert_eq!(back, read, "{context}: round trip");

    let rust: Value = serde_json::from_str(&written).unwrap();
    let java: Value = serde_json::from_str(java_pretty).unwrap();
    assert_eq!(
        shape(&rust),
        shape(&java),
        "{context}: the keys, in order, that the Java server writes"
    );
    assert!(
        !written.contains("null"),
        "{context}: optional values are left out"
    );
}

#[test]
fn java_documents_load_and_what_rust_writes_has_the_same_keys() {
    let rows = read_tsv("models.tsv");
    assert!(rows.len() > 300, "fixture looks truncated: {}", rows.len());
    for row in &rows {
        let [kind, label, input, pretty, compact] = row.as_slice() else {
            panic!("bad row {row:?}");
        };
        match kind.as_str() {
            "user" => check::<UserInfo>(kind, label, input, pretty, compact),
            "chart" => check::<ChartInfo>(kind, label, input, pretty, compact),
            "record" => check::<GameRecord>(kind, label, input, pretty, compact),
            "login" => check::<LoginResult>(kind, label, input, pretty, compact),
            "page" => check::<ChartListPage>(kind, label, input, pretty, compact),
            "records" => check::<Vec<GameRecord>>(kind, label, input, pretty, compact),
            other => panic!("unknown kind {other}"),
        }
    }
}

fn keys(value: &impl Serialize) -> Vec<String> {
    serde_json::to_value(value)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[test]
fn key_names_in_declaration_order() {
    let now = Some(Timestamp::now());
    let user = UserInfo {
        name: Some("n".into()),
        avatar: Some("a".into()),
        language: Some("l".into()),
        bio: Some("b".into()),
        joined: now,
        last_login: now,
        email: Some("e".into()),
        ..Default::default()
    };
    assert_eq!(
        keys(&user),
        [
            "id",
            "name",
            "avatar",
            "language",
            "bio",
            "exp",
            "rks",
            "joined",
            "lastLogin",
            "roles",
            "banned",
            "loginBanned",
            "followerCount",
            "followingCount",
            "email"
        ]
    );

    let chart: ChartInfo = serde_json::from_str(
        r#"{"name":"n","level":"l","charter":"c","composer":"c","illustrator":"i",
            "description":"d","illustration":"i","preview":"p","file":"f","tags":[],
            "created":"2026-01-01T00:00:00Z","updated":"2026-01-01T00:00:00Z",
            "chartUpdated":"2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    chart.set_duration_seconds(Some(1));
    assert_eq!(
        keys(&chart),
        [
            "id",
            "name",
            "level",
            "difficulty",
            "charter",
            "composer",
            "illustrator",
            "description",
            "ranked",
            "reviewed",
            "stable",
            "stableRequest",
            "illustration",
            "preview",
            "file",
            "uploader",
            "tags",
            "rating",
            "ratingCount",
            "created",
            "updated",
            "chartUpdated",
            "duration_seconds"
        ]
    );

    let record = GameRecord {
        time: now,
        ..Default::default()
    };
    assert_eq!(
        keys(&record),
        [
            "id",
            "player",
            "chart",
            "score",
            "accuracy",
            "perfect",
            "good",
            "bad",
            "miss",
            "speed",
            "max_combo",
            "best",
            "bestStd",
            "mods",
            "fullCombo",
            "time",
            "std",
            "stdScore"
        ]
    );

    let login = LoginResult {
        token: Some("t".into()),
        refresh_token: Some("r".into()),
        expire_at: Some("e".into()),
        ..Default::default()
    };
    assert_eq!(keys(&login), ["id", "token", "refresh_token", "expire_at"]);
    let page = ChartListPage {
        results: Some(vec![]),
        ..Default::default()
    };
    assert_eq!(keys(&page), ["count", "results"]);
}

#[test]
fn unset_optional_values_are_left_out() {
    assert_eq!(
        keys(&UserInfo::default()),
        [
            "id",
            "exp",
            "rks",
            "roles",
            "banned",
            "loginBanned",
            "followerCount",
            "followingCount"
        ]
    );
    assert_eq!(
        keys(&ChartInfo::default()),
        [
            "id",
            "difficulty",
            "ranked",
            "reviewed",
            "stable",
            "stableRequest",
            "uploader",
            "rating",
            "ratingCount"
        ]
    );
    assert_eq!(
        keys(&GameRecord::default()),
        [
            "id",
            "player",
            "chart",
            "score",
            "accuracy",
            "perfect",
            "good",
            "bad",
            "miss",
            "speed",
            "max_combo",
            "best",
            "bestStd",
            "mods",
            "fullCombo",
            "std",
            "stdScore"
        ]
    );
    assert_eq!(keys(&LoginResult::default()), ["id"]);
    assert_eq!(keys(&ChartListPage::default()), ["count"]);
}

#[test]
fn input_is_strict_json() {
    for text in [
        r#"{"id":"5"}"#,
        r#"{"id":5.0}"#,
        r#"{"id":null}"#,
        r#"{"banned":"true"}"#,
        r#"{id:5}"#,
        r#"{'id':5}"#,
        r#"{"id":5,}"#,
        r#"{"id":1,"id":2}"#,
        r#"{"id":5} x"#,
    ] {
        assert!(serde_json::from_str::<UserInfo>(text).is_err(), "{text}");
    }
    assert!(serde_json::from_str::<UserInfo>("{}").is_ok());
    assert!(serde_json::from_str::<UserInfo>(r#"{"unknown":[1,{"a":null}]}"#).is_ok());
}

#[test]
fn game_record_alternate_names() {
    let snake: GameRecord = serde_json::from_str(
        r#"{"best_std":true,"full_combo":true,"std_score":2.5,"max_combo":9}"#,
    )
    .unwrap();
    let camel: GameRecord =
        serde_json::from_str(r#"{"bestStd":true,"fullCombo":true,"stdScore":2.5,"max_combo":9}"#)
            .unwrap();
    assert_eq!(snake, camel);
    assert!(snake.best_std && snake.full_combo);
    assert_eq!(snake.std_score, 2.5);
    assert_eq!(snake.max_combo, 9);
    let camel_combo: GameRecord = serde_json::from_str(r#"{"maxCombo":105}"#).unwrap();
    assert_eq!(camel_combo.max_combo, 0);
}

#[test]
fn timestamps_read_both_forms_and_are_written_with_seconds() {
    let with: UserInfo = serde_json::from_str(r#"{"joined":"2026-11-14T15:26:00Z"}"#).unwrap();
    let without: UserInfo = serde_json::from_str(r#"{"joined":"2026-11-14T15:26Z"}"#).unwrap();
    assert_eq!(with, without);
    let joined = with.joined.unwrap();
    assert_eq!(joined.epoch_millis(), 1_794_669_960_000);
    assert_eq!(joined.to_string(), "2026-11-14T15:26:00Z");
    assert!(
        serde_json::to_string(&without)
            .unwrap()
            .contains("2026-11-14T15:26:00Z")
    );

    let offset: GameRecord =
        serde_json::from_str(r#"{"time":"2026-10-01T12:34:56.123456789+08:00"}"#).unwrap();
    let time = offset.time.unwrap();
    assert_eq!(time.epoch_millis(), 1_790_829_296_123);
    assert!(
        serde_json::to_string(&offset)
            .unwrap()
            .contains("12:34:56.123456789+08:00")
    );
    assert!(serde_json::from_str::<GameRecord>(r#"{"time":"yesterday"}"#).is_err());
}

#[test]
fn chart_duration_is_interior_mutable_and_serializes_as_optional_integer() {
    let chart = ChartInfo::default();
    assert_eq!(chart.duration_seconds(), None);
    assert!(
        !serde_json::to_string_pretty(&chart)
            .unwrap()
            .contains("duration_seconds")
    );

    let shared = Arc::new(chart);
    let seen_by_room = shared.clone();
    shared.set_duration_seconds(Some(321));
    assert_eq!(seen_by_room.duration_seconds(), Some(321));
    let pretty = serde_json::to_string_pretty(&*seen_by_room).unwrap();
    assert!(pretty.ends_with("\"duration_seconds\": 321\n}"), "{pretty}");

    let copy = (*shared).clone();
    shared.set_duration_seconds(None);
    assert_eq!(copy.duration_seconds(), Some(321));
    assert_eq!(seen_by_room.duration_seconds(), None);

    let back: ChartInfo = serde_json::from_str(r#"{"id":1,"duration_seconds":77}"#).unwrap();
    assert_eq!(back.duration_seconds(), Some(77));
    let back: ChartInfo = serde_json::from_str(r#"{"id":1,"duration_seconds":null}"#).unwrap();
    assert_eq!(back.duration_seconds(), None);
    let back: ChartInfo = serde_json::from_str(r#"{"id":1,"durationSeconds":77}"#).unwrap();
    assert_eq!(back.duration_seconds(), None, "camelCase is not the key");
}

#[test]
fn chart_duration_is_shared_across_threads() {
    let chart = Arc::new(ChartInfo::default());
    let writers: Vec<_> = (0..8)
        .map(|i| {
            let chart = chart.clone();
            std::thread::spawn(move || {
                for n in 0..1000 {
                    chart.set_duration_seconds(Some(i * 1000 + n));
                    let seen = chart.duration_seconds().expect("always set");
                    assert!((0..8000).contains(&seen));
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    chart.set_duration_seconds(Some(i32::MIN));
    assert_eq!(chart.duration_seconds(), Some(i32::MIN));
    chart.set_duration_seconds(Some(i32::MAX));
    assert_eq!(chart.duration_seconds(), Some(i32::MAX));
}

#[test]
fn doubles_keep_every_digit() {
    let user: UserInfo = serde_json::from_str(r#"{"rks":15.123456789012345}"#).unwrap();
    assert_eq!(user.rks, 15.123456789012345);
    let written = serde_json::to_string(&user).unwrap();
    let back: UserInfo = serde_json::from_str(&written).unwrap();
    assert_eq!(back.rks.to_bits(), user.rks.to_bits());
}

#[test]
fn helpers() {
    let mut chart = ChartInfo::default();
    assert_eq!(chart.name_or_null(), "null");
    assert_eq!(chart.level_or_null(), "null");
    assert!(!chart.has_tag("regular"));
    chart.name = Some("Song".into());
    chart.level = Some("AT 17".into());
    chart.tags = Some(vec!["plain".into(), "regular".into()]);
    assert_eq!(chart.name_or_null(), "Song");
    assert_eq!(chart.level_or_null(), "AT 17");
    assert!(chart.has_tag("regular") && chart.has_tag("plain") && !chart.has_tag("Regular"));

    let mut user = UserInfo::default();
    assert_eq!(user.name_or_null(), "null");
    user.name = Some("Alice".into());
    assert_eq!(user.name_or_null(), "Alice");
}
