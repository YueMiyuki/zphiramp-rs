mod support;

use std::path::Path;
use std::sync::Arc;

use support::{Script, json, read_fixture};
use zphira_ledger::{ImportMode, MAX_RECORDS, PlayerResult, RoundRecord, RoundRecords};

fn load(dir: &Path) -> RoundRecords {
    RoundRecords::load(dir).unwrap()
}

fn read_records(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("round-records.json")).unwrap()
}

fn ids(list: &[Arc<RoundRecord>]) -> String {
    list.iter()
        .map(|r| r.id.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn all_ids(records: &RoundRecords) -> String {
    ids(&records.query(100_000, 0, None, None))
}

fn result(
    (player_id, name): (i32, &str),
    rank: i32,
    score: i32,
    accuracy: f32,
    std: f32,
    gained: i32,
    total: i32,
) -> PlayerResult {
    PlayerResult {
        player_id,
        player_name: name.to_owned(),
        rank,
        score,
        accuracy: f64::from(accuracy),
        std: f64::from(std),
        gained_points: gained,
        total_points: total,
    }
}

fn round(
    id: String,
    room: &str,
    chart: i32,
    chart_name: Option<&str>,
    started: i64,
    finished: i64,
    results: Vec<PlayerResult>,
) -> RoundRecord {
    RoundRecord {
        id,
        room_id: room.to_owned(),
        chart_id: chart,
        chart_name: chart_name.map(str::to_owned),
        started_at: started,
        finished_at: finished,
        results,
    }
}

fn simple(id: &str, room: &str, finished: i64) -> RoundRecord {
    round(
        id.to_owned(),
        room,
        1,
        Some("c"),
        finished - 1,
        finished,
        vec![],
    )
}

fn java_rounds(records: &RoundRecords) -> Vec<RoundRecord> {
    let name = "Spasmodic <Lv.15> & 'co' \u{e9}";
    vec![
        round(
            records.next_id("room-a", 1_700_000_000_000),
            "room-a",
            101,
            Some(name),
            1_699_999_990_000,
            1_700_000_000_000,
            vec![
                result((3, "Alice"), 1, 990_000, 0.9876, 0.0123, 100, 100),
                result(
                    (4, "Bob \u{4e2d}\u{6587}\u{1f600}"),
                    2,
                    950_000,
                    0.95,
                    0.02,
                    75,
                    175,
                ),
            ],
        ),
        round(
            records.next_id("room-b", 1_700_000_100_000),
            "room-b",
            102,
            None,
            1_700_000_050_000,
            1_700_000_100_000,
            vec![result((5, "Eve"), 1, 1_000_000, 1.0, 0.0, 100, 300)],
        ),
        round(
            records.next_id("room-a", 1_700_000_200_000),
            "room-a",
            101,
            Some(name),
            1_700_000_150_000,
            1_700_000_200_000,
            vec![],
        ),
        round(
            records.next_id("room-a", 1_700_000_300_000),
            "room-a",
            103,
            Some("Tiny"),
            1_700_000_250_000,
            1_700_000_300_000,
            vec![
                result((3, "Alice"), 1, 123_456, 0.999_999_94, 1.0e-4, 100, 200),
                result((6, "Dan"), 1, 123_456, 0.999_999_94, 1.0e-4, 100, 100),
                result((7, "Fay"), 3, 1, 0.0, 12_345_678.0, 50, 50),
            ],
        ),
        round(
            records.next_id("room-c", 1_700_000_400_000),
            "room-c",
            104,
            Some("Long \"quoted\" name\nline"),
            1_700_000_350_000,
            1_700_000_400_000,
            vec![
                result((3, "Alice"), 2, 500_000, 0.5, 0.123_456_79, 75, 275),
                result((5, "Eve"), 1, 600_000, 0.876_543_2, 1.5, 100, 400),
            ],
        ),
    ]
}

#[test]
fn replays_the_java_scenario() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    let mut java = Script::new("ledger_records.tsv");

    let made = java_rounds(&records);
    let made_ids: Vec<&str> = made.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(made_ids.join(","), java.expect("next_ids")[0]);
    for r in made.iter().cloned() {
        records.append(r);
    }

    records.flush().unwrap();
    let file = read_records(dir.path());
    assert_eq!(json(&file), json(&java.expect("file_after_flush")[0]));
    assert_eq!(json(&file), json(&read_fixture("records_java.json")));
    assert_eq!(records.len().to_string(), java.expect("size")[0]);

    let again = RoundRecords::load(dir.path()).unwrap();
    assert_eq!(all_ids(&again), all_ids(&records));
    assert_eq!(*again.find(&made[0].id).unwrap(), made[0]);

    let limits = [(10, 0), (2, 1), (0, 0), (10, 99), (1, 4)];
    let rooms = [None, Some("room-a"), Some("room-c"), Some("nope")];
    let players = [None, Some(3), Some(5), Some(99)];
    for (limit, offset) in limits {
        for room in rooms {
            for player in players {
                let fields = java.expect("query");
                assert_eq!(
                    fields[0],
                    format!(
                        "{limit},{offset},{},{}",
                        room.unwrap_or("null"),
                        player.map_or("null".to_owned(), |p| p.to_string())
                    )
                );
                let expected = if limit == 0 { "" } else { &fields[1] };
                assert_eq!(
                    ids(&records.query(limit, offset, room, player)),
                    expected,
                    "query {}",
                    fields[0]
                );
            }
        }
    }
    for player in [3, 5, 99] {
        for limit in [10, 2, 1, 0] {
            let fields = java.expect("by_player");
            let rounds: Vec<String> = records
                .by_player(player, limit)
                .iter()
                .map(|r| {
                    format!(
                        "{}|{}|{}|{}|{}|{}|{}",
                        r.record_id,
                        r.room_id,
                        r.chart_id,
                        r.chart_name.as_deref().unwrap_or("null"),
                        r.finished_at,
                        r.result.player_id,
                        r.result.rank
                    )
                })
                .collect();
            let expected = if limit == 0 { "" } else { &fields[1] };
            assert_eq!(rounds.join(";"), expected, "by_player {}", fields[0]);
        }
    }
    for r in &made {
        let fields = java.expect("find");
        assert_eq!(fields[0], r.id);
        assert_eq!(records.find(&r.id).unwrap().chart_id.to_string(), fields[1]);
    }
    let fields = java.expect("find");
    assert_eq!((fields[0].as_str(), fields[1].as_str()), ("nope", "none"));
    assert!(records.find("nope").is_none());
    assert_eq!(records.next_id("zz", 5), java.expect("next_id_after")[0]);

    replay_imports(&records, &mut java);
}

fn replay_imports(records: &RoundRecords, java: &mut Script) {
    let generated: String = {
        let rows: Vec<String> = (0..1003)
            .map(|i| {
                format!(
                    "{{\"id\":\"m{i}\",\"room_id\":\"m\",\"chart_id\":1,\"started_at\":{i},\"finished_at\":{},\"results\":[]}}",
                    i + 1
                )
            })
            .collect();
        format!("[{}]", rows.join(","))
    };
    while let Some(row) = java.next_row() {
        let label = row[0].as_str();
        if label.starts_with("load_") {
            break;
        }
        if label.starts_with("import_") {
            let mode = if row[1].ends_with("merge") {
                ImportMode::Merge
            } else {
                ImportMode::Replace
            };
            let document = if row[2].starts_with("<generated") {
                generated.as_str()
            } else {
                row[2].as_str()
            };
            assert_eq!(row[3], "OK", "{label}");
            if label.ends_with("_dups") {
                let before = all_ids(records);
                let err = records.import(document, mode).unwrap_err();
                assert!(
                    err.is_invalid() && err.to_string().contains("重复"),
                    "{err}"
                );
                assert_eq!(all_ids(records), before, "nothing changed");
                continue;
            }
            let report = records.import(document, mode).unwrap();
            let rows = if label == "import_over_cap" {
                "1000"
            } else {
                row[4].as_str()
            };
            assert_eq!(report.imported.to_string(), rows, "{label}: imported");
            assert_eq!(report.total.to_string(), rows, "{label}: total");
        } else if label.starts_with("state_") {
            if label.ends_with("_dups") {
                continue;
            }
            let expected = match label {
                "state_merge_overlap" => "i2,i3,i1,i9",
                "state_merge_into_empty" => "i3,i1",
                _ => row[1].as_str(),
            };
            assert_eq!(all_ids(records), expected, "{label}");
        } else if label == "size_over_cap" {
            assert_eq!(records.len(), MAX_RECORDS);
        } else if label.starts_with("size_") && label != "size_after_append" {
            assert_eq!(records.len().to_string(), row[1], "{label}");
        } else if label == "size_after_append" {
            records.append(simple("after-cap", "m", 2));
            assert_eq!(records.len().to_string(), row[1]);
        } else if label == "newest_after_append" {
            assert_eq!(records.query(1, 0, None, None)[0].id, row[1]);
        } else if label == "oldest_after_append" {
            assert_eq!(records.query(1, 999, None, None)[0].id, row[1]);
        } else {
            panic!("unexpected row {label}");
        }
    }
}

#[test]
fn append_goes_first_and_the_cap_drops_the_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    for i in 0..(MAX_RECORDS + 5) {
        records.append(simple(&format!("r{i}"), "room", i as i64));
    }
    assert_eq!(records.len(), MAX_RECORDS);
    let newest = records.query(1, 0, None, None);
    assert_eq!(newest[0].id, format!("r{}", MAX_RECORDS + 4));
    let oldest = records.query(1, MAX_RECORDS - 1, None, None);
    assert_eq!(oldest[0].id, "r5");
    assert!(records.find("r4").is_none());
    assert!(records.find("r5").is_some());

    records.flush().unwrap();
    let file = read_records(dir.path());
    let first = file.find("\"r5\"").unwrap();
    let last = file.find(&format!("\"r{}\"", MAX_RECORDS + 4)).unwrap();
    assert!(first < last);
    assert_eq!(RoundRecords::load(dir.path()).unwrap().len(), MAX_RECORDS);
}

#[test]
fn query_filters_offsets_and_limits_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    let mut r1 = simple("a", "room-1", 10);
    r1.results = vec![result((1, "p1"), 1, 1, 0.5, 0.5, 100, 100)];
    let mut r2 = simple("b", "room-2", 20);
    r2.results = vec![
        result((1, "p1"), 2, 1, 0.5, 0.5, 75, 175),
        result((2, "p2"), 1, 2, 0.5, 0.5, 100, 100),
    ];
    let mut r3 = simple("c", "room-1", 30);
    r3.results = vec![result((2, "p2"), 1, 2, 0.5, 0.5, 100, 200)];
    let r4 = simple("d", "room-1", 40);
    for r in [r1, r2, r3, r4] {
        records.append(r);
    }
    assert_eq!(ids(&records.query(10, 0, None, None)), "d,c,b,a");
    assert_eq!(ids(&records.query(2, 0, None, None)), "d,c");
    assert_eq!(ids(&records.query(2, 1, None, None)), "c,b");
    assert_eq!(ids(&records.query(10, 3, None, None)), "a");
    assert_eq!(ids(&records.query(10, 4, None, None)), "");
    assert_eq!(ids(&records.query(10, 0, Some("room-1"), None)), "d,c,a");
    assert_eq!(ids(&records.query(10, 1, Some("room-1"), None)), "c,a");
    assert_eq!(ids(&records.query(10, 0, None, Some(1))), "b,a");
    assert_eq!(ids(&records.query(10, 0, None, Some(2))), "c,b");
    assert_eq!(ids(&records.query(10, 0, Some("room-1"), Some(2))), "c");
    assert_eq!(ids(&records.query(10, 1, Some("room-1"), Some(2))), "");
    assert_eq!(ids(&records.query(10, 0, Some("room-9"), None)), "");
    assert_eq!(ids(&records.query(10, 0, Some("ROOM-1"), None)), "");
    assert_eq!(ids(&records.query(0, 0, None, None)), "");
}

#[test]
fn by_player_takes_the_first_result_of_each_record_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    let mut r1 = simple("a", "room-1", 10);
    r1.results = vec![
        result((1, "p1"), 1, 1, 0.5, 0.5, 100, 100),
        result((1, "p1 again"), 2, 1, 0.5, 0.5, 0, 100),
    ];
    let mut r2 = simple("b", "room-2", 20);
    r2.results = vec![result((2, "p2"), 1, 1, 0.5, 0.5, 100, 100)];
    records.append(r1);
    records.append(r2);
    let rounds = records.by_player(1, 10);
    assert_eq!(rounds.len(), 1);
    assert_eq!(rounds[0].record_id, "a");
    assert_eq!(rounds[0].result.player_name, "p1");
    assert_eq!(rounds[0].finished_at, 10);
    assert!(records.by_player(3, 10).is_empty());
}

#[test]
fn ids_count_up_from_one() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    assert_eq!(records.next_id("r", 5), "r-5-1");
    assert_eq!(records.next_id("r", 5), "r-5-2");
    assert_eq!(records.next_id("other", 77), "other-77-3");
    assert_eq!(records.clone().next_id("r", 6), "r-6-4");
    let ids: std::collections::HashSet<String> =
        (0..1000).map(|_| records.next_id("r", 1)).collect();
    assert_eq!(ids.len(), 1000);
}

#[test]
fn ids_are_unique_across_threads() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let records = records.clone();
            std::thread::spawn(move || {
                (0..500)
                    .map(|_| records.next_id("room", 1))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut all = std::collections::HashSet::new();
    for t in threads {
        for id in t.join().unwrap() {
            assert!(all.insert(id));
        }
    }
    assert_eq!(all.len(), 4000);
}

#[test]
fn the_json_the_http_layer_serves_has_the_java_shape() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    let made = java_rounds(&records);
    for r in made.iter().cloned() {
        records.append(r);
    }
    let record = records.find(&made[1].id).unwrap();
    assert_eq!(
        serde_json::to_string(&record.api_view()).unwrap(),
        "{\"id\":\"room-b-1700000100000-2\",\"roomId\":\"room-b\",\"chartId\":102,\"chartName\":null,\
\"startedAt\":1700000050000,\"finishedAt\":1700000100000,\"results\":[{\"playerId\":5,\
\"playerName\":\"Eve\",\"rank\":1,\"score\":1000000,\"accuracy\":1.0,\"std\":0.0,\
\"gainedPoints\":100,\"totalPoints\":300}]}"
    );
    let round = &records.by_player(5, 1)[0];
    assert_eq!(
        serde_json::to_string(&round.api_view()).unwrap(),
        "{\"recordId\":\"room-c-1700000400000-5\",\"roomId\":\"room-c\",\"chartId\":104,\
\"chartName\":\"Long \\\"quoted\\\" name\\nline\",\"finishedAt\":1700000400000,\
\"result\":{\"playerId\":5,\"playerName\":\"Eve\",\"rank\":1,\"score\":600000,\
\"accuracy\":0.8765432238578796,\"std\":1.5,\"gainedPoints\":100,\"totalPoints\":400}}"
    );
}

#[test]
fn floats_are_widened_to_doubles_before_they_are_written() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    let mut r = simple("w", "room", 1);
    r.results = vec![result((1, "p"), 1, 1, 0.9876, 1.0e-4, 0, 0)];
    records.append(r);
    let text = records.export().unwrap();
    assert!(text.contains("\"accuracy\": 0.9876000285148621"), "{text}");
    assert!(text.contains("\"std\": 0.00009999999747378752"), "{text}");
}

#[test]
fn a_non_finite_number_cannot_poison_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    let mut bad = simple("bad", "room", 1);
    bad.results = vec![PlayerResult {
        accuracy: f64::NAN,
        std: f64::INFINITY,
        ..Default::default()
    }];
    let ((), logs) = support::capture_logs(|| records.append(bad));
    assert!(
        logs.iter().any(|l| l.message.contains("non-finite")),
        "{logs:?}"
    );
    records.append(simple("good", "room", 2));
    records.flush().unwrap();
    let back = RoundRecords::load(dir.path()).unwrap();
    assert_eq!(back.find("bad").unwrap().results[0].accuracy, 0.0);
    assert_eq!(back.find("bad").unwrap().results[0].std, 0.0);
}

#[test]
fn a_missing_file_is_empty_and_an_unreadable_one_stops_start_up() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load(dir.path()).is_empty());
    for document in ["[{\"id\":\"a\"", "{}", "[null]", "[{\"chart_id\":7.5}]"] {
        std::fs::write(dir.path().join("round-records.json"), document).unwrap();
        let err = RoundRecords::load(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Failed to load round records",
            "{document}"
        );
        assert_eq!(read_records(dir.path()), document, "left alone");
    }
}

#[test]
fn a_loaded_file_above_the_cap_keeps_the_newest_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let rows: Vec<String> = (0..MAX_RECORDS + 3)
        .map(|i| format!("{{\"id\":\"f{i}\",\"room_id\":\"r\",\"results\":[]}}"))
        .collect();
    std::fs::write(
        dir.path().join("round-records.json"),
        format!("[{}]", rows.join(",")),
    )
    .unwrap();
    let (records, logs) = support::capture_logs(|| load(dir.path()));
    assert_eq!(records.len(), MAX_RECORDS);
    assert!(
        logs.iter()
            .any(|l| l.message.contains("keeping the newest")),
        "{logs:?}"
    );
    assert_eq!(
        records.query(1, 0, None, None)[0].id,
        format!("f{}", MAX_RECORDS + 2)
    );
    assert_eq!(records.query(1, MAX_RECORDS - 1, None, None)[0].id, "f3");
    records.append(simple("new", "r", 1));
    assert_eq!(records.len(), MAX_RECORDS);
    assert_eq!(records.query(1, MAX_RECORDS - 1, None, None)[0].id, "f4");
}

#[test]
fn export_is_memory_not_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let records = load(dir.path());
    records.append(simple("one", "room", 1));
    assert!(!dir.path().join("round-records.json").exists());
    let exported = records.export().unwrap();
    assert!(exported.contains("\"id\": \"one\""));
    records.flush().unwrap();
    assert_eq!(exported, read_records(dir.path()));
    assert_eq!(load(dir.path()).export().unwrap(), exported);
    let dir2 = tempfile::tempdir().unwrap();
    assert_eq!(load(dir2.path()).export().unwrap(), "[]");
}
