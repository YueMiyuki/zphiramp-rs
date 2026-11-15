mod support;

use std::path::Path;

use support::{ManualClock, read_fixture};
use zphira_common::gson;
use zphira_ledger::{
    Admins, ImportMode, PlayerResult, PointAward, Points, RoundRecord, RoundRecords,
};

const T0: i64 = 1_700_000_000_000;

fn put(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap()
}

#[test]
fn both_generators_agree_byte_for_byte() {
    assert_eq!(
        read_fixture("gson_points.json"),
        read_fixture("points_java.json")
    );
    assert_eq!(
        read_fixture("gson_records.json"),
        read_fixture("records_java.json")
    );
    assert_eq!(
        read_fixture("gson_admins.json"),
        read_fixture("admins_java.json")
    );
}

#[test]
fn points_round_trip() {
    let gson_file = read_fixture("gson_points.json");
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "player-points.json", &gson_file);
    let points = Points::load_with_clock(dir.path(), ManualClock::new(T0).clock()).unwrap();
    assert_eq!(points.len(), 8);
    assert_eq!(points.export().unwrap(), gson_file);

    let fresh = tempfile::tempdir().unwrap();
    let rust = Points::load_with_clock(fresh.path(), ManualClock::new(T0).clock()).unwrap();
    let award = |id, name: Option<&str>, gained| PointAward {
        player_id: id,
        name: name.map(str::to_owned),
        gained,
    };
    rust.add_points_batch(&[
        award(42, Some("Newbie"), 10),
        award(20, Some("Zed<>&'"), 250),
        award(9, None, 0),
        award(7, Some("Seven"), 150),
        award(5, Some("Eve2"), 300),
        award(3, Some("Cy3"), 25),
        award(1, Some("Ann2"), 150),
        award(-4, Some("Neg"), 100),
    ]);
    rust.flush().unwrap();
    assert_eq!(read(fresh.path(), "player-points.json"), gson_file);

    assert!(gson_file.contains("\"9\": {\n    \"points\": 0\n  }"));
    assert!(gson_file.contains("Zed\\u003c\\u003e\\u0026\\u0027"));

    let empty = Points::load(fresh.path().join("none")).unwrap();
    assert_eq!(
        empty.export().unwrap(),
        read_fixture("gson_points_empty.json")
    );
}

#[test]
fn round_records_round_trip_with_the_design_shape() {
    let gson_file = read_fixture("gson_records.json");
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "round-records.json", &gson_file);
    let records = RoundRecords::load(dir.path()).unwrap();
    assert_eq!(records.len(), 5);
    assert_eq!(records.export().unwrap(), gson_file);

    let parsed: Vec<RoundRecord> = gson::from_str(&gson_file).unwrap();
    assert_eq!(parsed.len(), 5);
    let first = &parsed[0];
    assert_eq!(first.id, "room-a-1700000000000-1");
    assert_eq!(first.room_id, "room-a");
    assert_eq!(first.chart_id, 101);
    assert_eq!(
        first.chart_name.as_deref(),
        Some("Spasmodic <Lv.15> & 'co' \u{e9}")
    );
    assert_eq!(
        (first.started_at, first.finished_at),
        (1_699_999_990_000, 1_700_000_000_000)
    );
    assert_eq!(
        first.results[0],
        PlayerResult {
            player_id: 3,
            player_name: "Alice".to_owned(),
            rank: 1,
            score: 990_000,
            accuracy: f64::from(0.9876_f32),
            std: f64::from(0.0123_f32),
            gained_points: 100,
            total_points: 100,
        }
    );
    assert_eq!(parsed[1].chart_name, None);
    assert!(parsed[2].results.is_empty());
    assert_eq!(parsed[3].results[2].std, f64::from(12_345_678.0_f32));
    assert_eq!(parsed[3].results[0].accuracy, f64::from(0.999_999_94_f32));

    let fresh = tempfile::tempdir().unwrap();
    let rust = RoundRecords::load(fresh.path()).unwrap();
    for record in parsed {
        rust.append(record);
    }
    rust.flush().unwrap();
    assert_eq!(read(fresh.path(), "round-records.json"), gson_file);

    let empty = RoundRecords::load(fresh.path().join("none")).unwrap();
    assert_eq!(
        empty.export().unwrap(),
        read_fixture("gson_records_empty.json")
    );
}

#[test]
fn admins_round_trip() {
    let gson_file = read_fixture("gson_admins.json");
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "admins.json", &gson_file);
    let admins = Admins::load(dir.path()).unwrap();
    assert_eq!(admins.list(), vec![3, 5, 9]);
    assert_eq!(admins.export().unwrap(), gson_file);

    let fresh = tempfile::tempdir().unwrap();
    let rust = Admins::load(fresh.path()).unwrap();
    assert_eq!(
        read(fresh.path(), "admins.json"),
        read_fixture("gson_admins_empty.json")
    );
    for id in [9, 3, 5] {
        rust.add(id).unwrap();
    }
    assert_eq!(read(fresh.path(), "admins.json"), gson_file);
}

#[test]
fn an_import_of_a_java_file_is_the_same_as_loading_it() {
    let dir = tempfile::tempdir().unwrap();
    let points = Points::load(dir.path()).unwrap();
    let report = points
        .import(&read_fixture("gson_points.json"), ImportMode::Replace)
        .unwrap();
    assert_eq!(report.total, 8);
    assert_eq!(
        read(dir.path(), "player-points.json"),
        read_fixture("gson_points.json")
    );

    let records = RoundRecords::load(dir.path()).unwrap();
    let report = records
        .import(&read_fixture("gson_records.json"), ImportMode::Replace)
        .unwrap();
    assert_eq!(report.total, 5);
    assert_eq!(
        read(dir.path(), "round-records.json"),
        read_fixture("gson_records.json")
    );

    let admins = Admins::load(dir.path()).unwrap();
    admins
        .import(&read_fixture("gson_admins.json"), ImportMode::Replace)
        .unwrap();
    assert_eq!(
        read(dir.path(), "admins.json"),
        read_fixture("gson_admins.json")
    );
}
