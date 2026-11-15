mod support;

use std::path::Path;

use serde_json::Value;
use support::{ManualClock, read_fixture};
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

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn points(dir: &Path) -> Points {
    Points::load_with_clock(dir, ManualClock::new(T0).clock()).unwrap()
}

#[test]
fn points_file_of_the_java_server_loads_and_is_written_back_the_same() {
    let java = read_fixture("points_java.json");
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "player-points.json", &java);
    let loaded = points(dir.path());
    assert_eq!(loaded.len(), 8);
    assert_eq!(value(&loaded.export().unwrap()), value(&java));

    let fresh = tempfile::tempdir().unwrap();
    let rust = points(fresh.path());
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
    let written = read(fresh.path(), "player-points.json");
    assert_eq!(value(&written), value(&java));

    assert!(
        written.starts_with("{\n  \"-4\": {\n    \"name\": \"Neg\",\n    \"points\": 100\n  },")
    );
    assert!(written.contains("\"9\": {\n    \"points\": 0\n  }"));
    let keys: Vec<i64> = value(&written)
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.parse().unwrap())
        .collect();
    assert!(
        keys.windows(2).all(|w| w[0] < w[1]),
        "sorted by player id: {keys:?}"
    );

    assert_eq!(points(&fresh.path().join("none")).export().unwrap(), "{}");
}

#[test]
fn round_records_file_of_the_java_server_loads_and_is_written_back_the_same() {
    let java = read_fixture("records_java.json");
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "round-records.json", &java);
    let loaded = RoundRecords::load(dir.path()).unwrap();
    assert_eq!(loaded.len(), 5);
    assert_eq!(value(&loaded.export().unwrap()), value(&java));

    let parsed: Vec<RoundRecord> = serde_json::from_str(&java).unwrap();
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
    assert_eq!(parsed[3].results[0].std, f64::from(1.0e-4_f32));

    let fresh = tempfile::tempdir().unwrap();
    let rust = RoundRecords::load(fresh.path()).unwrap();
    for record in parsed {
        rust.append(record);
    }
    rust.flush().unwrap();
    let written = read(fresh.path(), "round-records.json");
    assert_eq!(value(&written), value(&java));
    assert!(
        written.starts_with("[\n  {\n    \"id\": \"room-a-1700000000000-1\",\n    \"room_id\"")
    );
    let second = &value(&written)[1];
    assert!(
        second.get("chart_name").is_none(),
        "an absent name is left out"
    );

    assert_eq!(
        RoundRecords::load(fresh.path().join("none"))
            .unwrap()
            .export()
            .unwrap(),
        "[]"
    );
}

#[test]
fn admins_file_of_the_java_server_loads_and_is_written_back_the_same() {
    let java = read_fixture("admins_java.json");
    let dir = tempfile::tempdir().unwrap();
    put(dir.path(), "admins.json", &java);
    let admins = Admins::load(dir.path()).unwrap();
    assert_eq!(admins.list(), vec![3, 5, 9]);
    assert_eq!(admins.export().unwrap(), java);

    let fresh = tempfile::tempdir().unwrap();
    let rust = Admins::load(fresh.path()).unwrap();
    assert_eq!(read(fresh.path(), "admins.json"), "[]");
    for id in [9, 3, 5] {
        rust.add(id).unwrap();
    }
    assert_eq!(read(fresh.path(), "admins.json"), java);
}

#[test]
fn importing_the_files_of_the_java_server_is_loading_them() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    let report = p
        .import(&read_fixture("points_java.json"), ImportMode::Replace)
        .unwrap();
    assert_eq!(report.total, 8);
    assert_eq!(
        value(&read(dir.path(), "player-points.json")),
        value(&read_fixture("points_java.json"))
    );

    let records = RoundRecords::load(dir.path()).unwrap();
    let report = records
        .import(&read_fixture("records_java.json"), ImportMode::Replace)
        .unwrap();
    assert_eq!(report.total, 5);
    assert_eq!(
        value(&read(dir.path(), "round-records.json")),
        value(&read_fixture("records_java.json"))
    );

    let admins = Admins::load(dir.path()).unwrap();
    admins
        .import(&read_fixture("admins_java.json"), ImportMode::Replace)
        .unwrap();
    assert_eq!(
        read(dir.path(), "admins.json"),
        read_fixture("admins_java.json")
    );
}

#[test]
fn files_have_the_java_layout() {
    let dir = tempfile::tempdir().unwrap();
    let p = points(dir.path());
    p.add_points(1, "a<b>", 1);
    p.flush().unwrap();
    assert_eq!(
        read(dir.path(), "player-points.json"),
        "{\n  \"1\": {\n    \"name\": \"a<b>\",\n    \"points\": 1\n  }\n}"
    );
    let records = RoundRecords::load(dir.path()).unwrap();
    records.append(RoundRecord {
        id: "a".to_owned(),
        room_id: "r".to_owned(),
        results: vec![],
        ..Default::default()
    });
    records.flush().unwrap();
    assert_eq!(
        read(dir.path(), "round-records.json"),
        "[\n  {\n    \"id\": \"a\",\n    \"room_id\": \"r\",\n    \"chart_id\": 0,\n    \"started_at\": 0,\n    \"finished_at\": 0,\n    \"results\": []\n  }\n]"
    );
}
