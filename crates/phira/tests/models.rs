mod support;

use std::fmt::Debug;

use serde::Serialize;
use serde::de::DeserializeOwned;
use support::read_tsv;
use zphira_common::gson;
use zphira_phira::{ChartInfo, ChartListPage, GameRecord, LoginResult, UserInfo};

fn rust_refuses_null_elements(kind: &str, label: &str) -> bool {
    (kind == "records" && label == "null element")
        || (kind == "page" && label == "result is null element")
}

fn check<T>(kind: &str, label: &str, input: &str, outcome: &str, pretty: &str, compact: &str)
where
    T: Serialize + DeserializeOwned + Debug + PartialEq,
{
    let context = format!("{kind} / {label} / {input}");
    if outcome == "null" {
        let parsed = gson::from_str_opt::<T>(input)
            .unwrap_or_else(|e| panic!("{context}: Java returned null, Rust failed: {e}"));
        assert!(parsed.is_none(), "{context}: Java returned null");
        return;
    }
    if let Some(java_error) = outcome.strip_prefix("EXC ") {
        let parsed = gson::from_str_opt::<T>(input);
        assert!(
            parsed.is_err(),
            "{context}: Java threw {java_error}, Rust accepted: {parsed:?}"
        );
        return;
    }
    if let Some(java_error) = outcome.strip_prefix("WRITE_EXC ") {
        let parsed: T = gson::from_str(input)
            .unwrap_or_else(|e| panic!("{context}: Java read it, Rust failed: {e}"));
        assert!(
            gson::to_string_pretty(&parsed).is_err() && gson::to_string_compact(&parsed).is_err(),
            "{context}: Java could not write it ({java_error}), Rust did"
        );
        return;
    }
    assert_eq!(outcome, "ok", "{context}");
    if rust_refuses_null_elements(kind, label) {
        assert!(
            gson::from_str_opt::<T>(input).is_err(),
            "{context}: null element is refused"
        );
        return;
    }
    let parsed: T = gson::from_str_opt(input)
        .unwrap_or_else(|e| panic!("{context}: Java parsed it, Rust failed: {e}"))
        .unwrap_or_else(|| panic!("{context}: Rust returned null"));
    assert_eq!(
        gson::to_string_pretty(&parsed).unwrap(),
        pretty,
        "{context}: pretty"
    );
    assert_eq!(
        gson::to_string_compact(&parsed).unwrap(),
        compact,
        "{context}: compact"
    );
    let again: T = gson::from_str(pretty)
        .unwrap_or_else(|e| panic!("{context}: reading Java's pretty output: {e}"));
    assert_eq!(again, parsed, "{context}: pretty round trip");
    let again: T = gson::from_str(compact)
        .unwrap_or_else(|e| panic!("{context}: reading Java's compact output: {e}"));
    assert_eq!(again, parsed, "{context}: compact round trip");
}

#[test]
fn every_row_matches_the_real_gson() {
    let rows = read_tsv("gson_models.tsv");
    assert!(rows.len() > 400, "fixture looks truncated: {}", rows.len());
    let mut counts = std::collections::BTreeMap::<String, (usize, usize)>::new();
    for row in &rows {
        let [kind, label, input, outcome, pretty, compact] = row.as_slice() else {
            panic!("bad row {row:?}");
        };
        let entry = counts.entry(kind.clone()).or_default();
        if outcome == "ok" {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
        if label.starts_with("null for a primitive record component")
            || label.starts_with("null count")
        {
            continue;
        }
        match kind.as_str() {
            "user" => check::<UserInfo>(kind, label, input, outcome, pretty, compact),
            "chart" => check::<ChartInfo>(kind, label, input, outcome, pretty, compact),
            "record" => check::<GameRecord>(kind, label, input, outcome, pretty, compact),
            "login" => check::<LoginResult>(kind, label, input, outcome, pretty, compact),
            "page" => check::<ChartListPage>(kind, label, input, outcome, pretty, compact),
            "records" => check::<Vec<GameRecord>>(kind, label, input, outcome, pretty, compact),
            other => panic!("unknown kind {other}"),
        }
    }
    for (kind, (ok, rejected)) in counts {
        assert!(ok >= 10, "{kind}: only {ok} accepted rows");
        assert!(rejected >= 1, "{kind}: no rejected rows");
    }
}

#[test]
fn records_reject_null_primitives_in_java_but_rust_keeps_the_default() {
    let rows = read_tsv("gson_models.tsv");
    let java: Vec<_> = rows
        .iter()
        .filter(|r| r[1].starts_with("null for a primitive") || r[1].starts_with("null count"))
        .collect();
    assert_eq!(java.len(), 2);
    for row in java {
        assert_eq!(row[3], "EXC JsonParseException");
    }
    let login: LoginResult = gson::from_str(r#"{"id":null,"token":"t"}"#).unwrap();
    assert_eq!(login.id, 0);
    assert_eq!(login.token.as_deref(), Some("t"));
    let page: ChartListPage = gson::from_str(r#"{"count":null}"#).unwrap();
    assert_eq!(page.count, 0);
}

#[test]
fn key_names_in_declaration_order() {
    fn keys(json: &str) -> Vec<String> {
        let element = gson::JsonElement::parse(json).unwrap();
        element
            .as_object()
            .unwrap()
            .keys()
            .map(ToOwned::to_owned)
            .collect()
    }
    let user = gson::to_string_compact(&UserInfo::default()).unwrap();
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
    let chart = gson::to_string_compact(&ChartInfo::default()).unwrap();
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
    let record = gson::to_string_compact(&GameRecord::default()).unwrap();
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
    let login = gson::to_string_compact(&LoginResult::default()).unwrap();
    assert_eq!(keys(&login), ["id", "token", "refresh_token", "expire_at"]);
    let page = gson::to_string_compact(&ChartListPage::default()).unwrap();
    assert_eq!(keys(&page), ["count", "results"]);
}

#[test]
fn game_record_alternate_names() {
    let snake: GameRecord =
        gson::from_str(r#"{"best_std":true,"full_combo":true,"std_score":2.5,"max_combo":9}"#)
            .unwrap();
    let camel: GameRecord =
        gson::from_str(r#"{"bestStd":true,"fullCombo":true,"stdScore":2.5,"max_combo":9}"#)
            .unwrap();
    assert_eq!(snake, camel);
    assert!(snake.best_std && snake.full_combo);
    assert_eq!(snake.std_score, 2.5);
    assert_eq!(snake.max_combo, 9);
    let camel_combo: GameRecord = gson::from_str(r#"{"maxCombo":105}"#).unwrap();
    assert_eq!(camel_combo.max_combo, 0);
}

#[test]
fn chart_duration_is_interior_mutable_and_serializes_as_optional_integer() {
    let chart = ChartInfo::default();
    assert_eq!(chart.duration_seconds(), None);
    assert!(
        !gson::to_string_pretty(&chart)
            .unwrap()
            .contains("duration_seconds")
    );
    assert!(
        gson::to_string_compact(&chart)
            .unwrap()
            .ends_with(r#""duration_seconds":null}"#)
    );

    let shared = std::sync::Arc::new(chart);
    let seen_by_room = shared.clone();
    shared.set_duration_seconds(Some(321));
    assert_eq!(seen_by_room.duration_seconds(), Some(321));
    let pretty = gson::to_string_pretty(&*seen_by_room).unwrap();
    assert!(pretty.ends_with("\"duration_seconds\": 321\n}"), "{pretty}");

    let copy = (*shared).clone();
    shared.set_duration_seconds(None);
    assert_eq!(copy.duration_seconds(), Some(321));
    assert_eq!(seen_by_room.duration_seconds(), None);

    let back: ChartInfo = gson::from_str(r#"{"id":1,"duration_seconds":"77"}"#).unwrap();
    assert_eq!(back.duration_seconds(), Some(77));
    let back: ChartInfo = gson::from_str(r#"{"id":1,"durationSeconds":77}"#).unwrap();
    assert_eq!(back.duration_seconds(), None, "camelCase is not the key");
}

#[test]
fn chart_duration_is_shared_across_threads() {
    let chart = std::sync::Arc::new(ChartInfo::default());
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
