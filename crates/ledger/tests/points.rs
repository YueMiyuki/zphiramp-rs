mod support;

use std::collections::BTreeMap;
use std::path::Path;

use support::{ManualClock, Script, read_fixture, read_tsv};
use zphira_ledger::{ImportMode, PointAward, Points, RankEntry};

const T0: i64 = 1_700_000_000_000;

fn load(dir: &Path, clock: &ManualClock) -> Points {
    Points::load_with_clock(dir, clock.clock()).unwrap()
}

fn write_points(dir: &Path, document: &str) {
    std::fs::write(dir.join("player-points.json"), document).unwrap();
}

fn read_points(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("player-points.json")).unwrap()
}

fn snapshot(entries: &[RankEntry]) -> String {
    entries
        .iter()
        .map(|e| {
            format!(
                "{}:{}:{}:{}",
                e.player_id,
                e.name.as_deref().unwrap_or("null"),
                e.points,
                e.rank
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn ranks(points: &Points, ids: &[i32]) -> String {
    ids.iter()
        .map(|id| format!("{id}:{}", points.rank(*id)))
        .collect::<Vec<_>>()
        .join(";")
}

fn award(player_id: i32, name: Option<&str>, gained: i32) -> PointAward {
    PointAward {
        player_id,
        name: name.map(str::to_owned),
        gained,
    }
}

const RANK_IDS: [i32; 11] = [1, 3, 5, 9, 20, 777, -4, 42, 7, 100, 2];

#[test]
fn replays_the_java_scenario() {
    let dir = tempfile::tempdir().unwrap();
    write_points(dir.path(), &read_fixture("points_seed.json"));
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    let mut java = Script::new("ledger_points.tsv");

    assert_eq!(
        snapshot(&points.ranking_snapshot(100)),
        java.expect("snapshot_load")[0]
    );
    assert_eq!(
        snapshot(&points.ranking_snapshot(2)),
        java.expect("snapshot_limit2")[0]
    );
    assert_eq!(ranks(&points, &RANK_IDS), java.expect("ranks_load")[0]);
    assert_eq!(
        points.top_ranking_lines(10).join("\n"),
        java.expect("lines_first")[0]
    );

    let s = points.summary(42, "Newbie");
    assert_eq!(
        format!("{}:{}", s.points, s.rank),
        java.expect("summary_new")[0]
    );
    let s = points.summary(5, "Eve2");
    assert_eq!(
        format!("{}:{}", s.points, s.rank),
        java.expect("summary_rename")[0]
    );
    assert_eq!(
        snapshot(&points.ranking_snapshot(100)),
        format!("{};42:Newbie:0:7", java.expect("snapshot_after_touch")[0])
    );
    assert_eq!(ranks(&points, &[42]), java.expect("rank_new_again")[0]);

    let totals = points.add_points_batch(&[
        award(7, Some("Seven"), 150),
        award(1, Some("Ann2"), 50),
        award(42, None, 10),
    ]);
    let totals: Vec<String> = totals.iter().map(|(id, t)| format!("{id}:{t}")).collect();
    assert_eq!(totals.join(";"), java.expect("batch_totals")[0]);
    assert_eq!(
        snapshot(&points.ranking_snapshot(100)),
        java.expect("snapshot_after_batch")[0]
    );
    assert_eq!(
        ranks(&points, &RANK_IDS),
        java.expect("ranks_after_batch")[0]
    );

    clock.advance(1000);
    assert_eq!(
        points.top_ranking_lines(10).join("\n"),
        java.expect("lines_cached")[0]
    );
    assert_eq!(
        points.top_ranking_lines(3).join("\n"),
        java.expect("lines_cached_other_limit")[0]
    );

    assert_eq!(
        points.add_points(3, "Cy3", -1000).to_string(),
        java.expect("add_points_floor")[0]
    );
    assert_eq!(
        points.add_points(3, "Cy3", 25).to_string(),
        java.expect("add_points_after_floor")[0]
    );
    assert_eq!(
        snapshot(&points.ranking_snapshot(100)),
        java.expect("snapshot_after_add")[0]
    );

    points.flush().unwrap();
    let file = read_points(dir.path());
    assert_eq!(file, java.expect("file_after_flush")[0]);
    assert_eq!(file, read_fixture("points_java.json"));

    let mut imports = 0;
    while let Some(row) = java.next_row() {
        let label = row[0].as_str();
        if label.starts_with("load_") {
            break;
        }
        if let Some(case) = label.strip_prefix("import_") {
            let mode = if row[1].ends_with("merge") {
                ImportMode::Merge
            } else {
                ImportMode::Replace
            };
            assert_eq!(row[3], "OK", "{case}");
            let report = points.import(&row[2], mode).unwrap();
            assert_eq!(report.imported.to_string(), row[4], "{case}: imported");
            assert_eq!(report.total.to_string(), row[5], "{case}: total");
            imports += 1;
        } else if label.starts_with("snapshot_") {
            assert_eq!(snapshot(&points.ranking_snapshot(100)), row[1], "{label}");
        } else if label == "lines_after_import" {
            assert_eq!(points.top_ranking_lines(10).join("\n"), row[1]);
        } else {
            panic!("unexpected row {label}");
        }
    }
    assert_eq!(imports, 5);
}

#[test]
fn loader_agrees_with_gson_on_every_recorded_document() {
    let rows: Vec<_> = read_tsv("ledger_points.tsv")
        .into_iter()
        .filter(|r| r[0].starts_with("load_"))
        .collect();
    assert!(rows.len() > 30);
    for row in rows {
        let (name, document, java) = (&row[0], &row[1], &row[2]);
        let dir = tempfile::tempdir().unwrap();
        write_points(dir.path(), document);
        let loaded = Points::load(dir.path());
        match java.strip_prefix("OK ") {
            Some(expected) => {
                let points = loaded.unwrap_or_else(|e| panic!("{name}: {e} {:?}", e));
                assert_eq!(snapshot(&points.ranking_snapshot(100)), expected, "{name}");
            }
            None => assert!(loaded.is_err(), "{name}: Java refuses it, got {loaded:?}"),
        }
    }
}

#[test]
fn ranking_orders_by_points_then_id_with_consecutive_ranks() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    points.add_points_batch(&[
        award(30, Some("c"), 100),
        award(10, Some("a"), 100),
        award(20, Some("b"), 100),
        award(5, Some("low"), 1),
        award(99, Some("top"), 500),
        award(-3, Some("neg"), 100),
    ]);
    let order: Vec<(i32, usize)> = points
        .ranking_snapshot(10)
        .iter()
        .map(|e| (e.player_id, e.rank))
        .collect();
    assert_eq!(
        order,
        vec![(99, 1), (-3, 2), (10, 3), (20, 4), (30, 5), (5, 6)]
    );
    assert_eq!(points.rank(20), 4);
}

#[test]
fn rank_of_an_unknown_player_is_size_plus_one() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    assert_eq!(points.rank(1), 1, "empty board");
    points.add_points_batch(&[award(1, Some("a"), 10), award(2, Some("b"), 20)]);
    assert_eq!(points.rank(123), 3);
    assert_eq!(points.rank(2), 1);
    assert_eq!(points.rank(1), 2);
}

#[test]
fn a_new_player_is_ranked_consistently() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    points.add_points_batch(&[award(10, Some("ten"), 0), award(20, Some("twenty"), 5)]);
    assert_eq!(points.rank(20), 1);
    assert_eq!(points.rank(10), 2);

    let s = points.summary(1, "one");
    assert_eq!((s.points, s.rank), (0, 2));
    let order: Vec<(i32, usize)> = points
        .ranking_snapshot(10)
        .iter()
        .map(|e| (e.player_id, e.rank))
        .collect();
    assert_eq!(order, vec![(20, 1), (1, 2), (10, 3)]);
    assert_eq!(points.rank(10), 3);
    assert_eq!(
        points.rank(999),
        4,
        "a player without a row is still size + 1"
    );

    points.add_points(20, "twenty", 1);
    assert_eq!(points.rank(1), 2);
    assert_eq!(points.summary(1, "one").rank, 2);

    let s = points.summary(20, "renamed");
    assert_eq!((s.points, s.rank), (6, 1));
    assert_eq!(
        points.ranking_snapshot(1)[0].name.as_deref(),
        Some("renamed")
    );

    points.add_points(1, "one", 100);
    assert_eq!(points.rank(1), 1);
    assert_eq!(points.rank(20), 2);
}

#[test]
fn looking_at_a_player_does_not_mark_the_store_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    let _ = points.summary(1, "seen");
    let _ = points.summary(1, "renamed");
    points.flush().unwrap();
    assert!(!dir.path().join("player-points.json").exists());
    assert_eq!(points.len(), 1);
}

#[test]
fn points_never_go_below_zero_and_overflow_wraps_like_java() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    assert_eq!(points.add_points(1, "a", 50), 50);
    assert_eq!(points.add_points(1, "a", -80), 0);
    assert_eq!(points.add_points(1, "a", i32::MAX), i32::MAX);
    assert_eq!(points.add_points(1, "a", 1), 0);
}

#[test]
fn batch_keeps_the_name_when_none_is_given_and_returns_totals_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    points.add_points_batch(&[award(1, Some("first"), 10)]);
    let totals = points.add_points_batch(&[award(2, None, 5), award(1, None, 7)]);
    assert_eq!(totals, vec![(2, 5), (1, 17)]);
    let snap = points.ranking_snapshot(10);
    assert_eq!(snap[0].name.as_deref(), Some("first"));
    assert_eq!(snap[1].name, None);
}

#[test]
fn top_lines_cache_lasts_sixty_seconds_inclusive() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    points.add_points_batch(&[award(1, Some("Ann"), 100), award(2, Some("Bo"), 50)]);

    let first = points.top_ranking_lines(10);
    assert_eq!(first, vec!["#1 Ann：100 分", "#2 Bo：50 分"]);

    points.add_points(2, "Bo", 100);
    clock.advance(60_000);
    assert_eq!(points.top_ranking_lines(10), first, "<= 60000 is a hit");
    clock.advance(1);
    let second = points.top_ranking_lines(1);
    assert_eq!(
        second,
        vec!["#1 Bo：150 分"],
        "expired: recomputed, with the new limit"
    );
    clock.advance(60_000);
    assert_eq!(points.top_ranking_lines(10), second);
}

#[test]
fn an_empty_board_caches_its_empty_answer_too() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    assert!(points.top_ranking_lines(10).is_empty());
    points.add_points(1, "a", 5);
    assert!(points.top_ranking_lines(10).is_empty());
    clock.advance(60_001);
    assert_eq!(points.top_ranking_lines(10).len(), 1);
}

#[test]
fn a_missing_name_is_printed_as_null() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    points.add_points_batch(&[award(8, None, 3)]);
    assert_eq!(points.top_ranking_lines(10), vec!["#1 null：3 分"]);
}

#[test]
fn a_java_file_loads_and_is_written_back_identically() {
    let dir = tempfile::tempdir().unwrap();
    let java = read_fixture("points_java.json");
    write_points(dir.path(), &java);
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    assert_eq!(points.len(), 8);
    assert_eq!(points.export().unwrap(), java);

    points.add_points(5, "Eve2", 0);
    std::fs::remove_file(dir.path().join("player-points.json")).unwrap();
    points.flush().unwrap();
    assert_eq!(read_points(dir.path()), java);
}

#[test]
fn a_missing_file_is_an_empty_board_and_is_created_by_the_first_flush() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let clock = ManualClock::new(T0);
    let points = Points::load_with_clock(&data, clock.clock()).unwrap();
    assert!(points.is_empty());
    assert!(!data.join("player-points.json").exists());
    points.flush().unwrap();
    assert!(
        !data.join("player-points.json").exists(),
        "nothing dirty, nothing written"
    );
    points.add_points(1, "a", 1);
    points.flush().unwrap();
    assert_eq!(
        std::fs::read_to_string(data.join("player-points.json")).unwrap(),
        "{\n  \"1\": {\n    \"name\": \"a\",\n    \"points\": 1\n  }\n}"
    );
}

#[test]
fn an_unreadable_file_stops_start_up_and_is_left_alone() {
    for document in [
        "{\"1\":",
        "{\"1\":{\"points\":\"x\"}}",
        "[1]",
        "\"text\"",
        "{\"abc\":{}}",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_points(dir.path(), document);
        let err = Points::load(dir.path()).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Failed to load player points",
            "{document}"
        );
        assert!(std::error::Error::source(&err).is_some());
        assert_eq!(read_points(dir.path()), document, "left alone");
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("player-points.json"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(Points::load(dir.path()).is_err(), "invalid UTF-8");
}

#[test]
fn an_empty_file_starts_empty_and_warns() {
    let dir = tempfile::tempdir().unwrap();
    write_points(dir.path(), "");
    let (points, logs) = support::capture_logs(|| Points::load(dir.path()).unwrap());
    assert!(points.is_empty());
    assert!(
        logs.iter().any(|l| l.message.contains("is empty")),
        "{logs:?}"
    );
}

#[test]
fn start_up_removes_temp_files_an_interrupted_write_left() {
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join(".player-points.json.1234-0.tmp");
    std::fs::write(&stale, "half").unwrap();
    let other = dir.path().join(".admins.json.1234-0.tmp");
    std::fs::write(&other, "not ours").unwrap();
    let _ = Points::load(dir.path()).unwrap();
    assert!(!stale.exists());
    assert!(other.exists());
}

#[test]
fn handles_share_one_board() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let a = load(dir.path(), &clock);
    let b = a.clone();
    b.add_points(1, "x", 9);
    assert_eq!(a.ranking_snapshot(5)[0].points, 9);
}

#[test]
fn rank_entry_serializes_as_the_api_object() {
    let entry = RankEntry {
        player_id: 7,
        name: None,
        points: 3,
        rank: 2,
    };
    assert_eq!(
        zphira_common::gson::to_string_compact(&entry).unwrap(),
        r#"{"playerId":7,"name":null,"points":3,"rank":2}"#
    );
}

#[test]
fn concurrent_awards_and_reads_keep_the_board_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(T0);
    let points = load(dir.path(), &clock);
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let points = points.clone();
            std::thread::spawn(move || {
                for i in 0..500 {
                    points.add_points_batch(&[award(t, Some("p"), 1), award(100 + i % 7, None, 1)]);
                    let _ = points.summary(t, "p");
                    let snap = points.ranking_snapshot(5);
                    assert!(snap.windows(2).all(|w| w[0].points >= w[1].points));
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let total: i64 = points
        .ranking_snapshot(1000)
        .iter()
        .map(|e| i64::from(e.points))
        .sum();
    assert_eq!(total, 8 * 500 * 2);
    let by_id: BTreeMap<i32, usize> = points
        .ranking_snapshot(1000)
        .iter()
        .map(|e| (e.player_id, e.rank))
        .collect();
    assert_eq!(by_id.len(), points.len());
}
