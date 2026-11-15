mod support;

use serde::Deserialize;
use zphira_common::time::Timestamp;

#[derive(Deserialize)]
struct Stamped {
    label: String,
    at: Timestamp,
    millis: i64,
}

#[test]
fn java_written_timestamps_load() {
    let rows: Vec<Stamped> =
        serde_json::from_str(&support::read_fixture("java_timestamps.json")).unwrap();
    assert!(rows.len() >= 13);
    for row in &rows {
        assert_eq!(row.at.epoch_millis(), row.millis, "{}", row.label);
        let printed = row.at.to_string();
        assert_eq!(Timestamp::parse(&printed).unwrap(), row.at, "{printed}");
    }
    assert!(rows.iter().any(|r| r.label == "now_minute"));
}

#[test]
fn printed_form() {
    let show = |s: &str| Timestamp::parse(s).unwrap().to_string();
    assert_eq!(show("2026-11-14T15:26Z"), "2026-11-14T15:26:00Z");
    assert_eq!(show("2026-11-14T15:26+08:00"), "2026-11-14T15:26:00+08:00");
    assert_eq!(show("2026-11-14T15:26:46.120Z"), "2026-11-14T15:26:46.120Z");
    assert_eq!(
        show("2026-11-14T15:26:46.5+00:00"),
        "2026-11-14T15:26:46.500Z"
    );
    assert_eq!(
        show("2026-11-14T15:26:46.123456-05:30"),
        "2026-11-14T15:26:46.123456-05:30"
    );
    assert_eq!(
        serde_json::to_string(&Timestamp::from_epoch_millis(1_794_670_006_000).unwrap()).unwrap(),
        "\"2026-11-14T15:26:46Z\""
    );
    assert!(Timestamp::parse("2026-11-14 15:26").is_err());
    assert!(serde_json::from_str::<Timestamp>("\"yesterday\"").is_err());
}

#[test]
fn ordered_by_instant() {
    let utc = Timestamp::parse("2026-11-14T15:26:46Z").unwrap();
    let shanghai = Timestamp::parse("2026-11-14T23:26:46+08:00").unwrap();
    assert_eq!(utc, shanghai);
    assert!(Timestamp::parse("2026-11-14T23:26:47+08:00").unwrap() > utc);
    assert_eq!(shanghai.epoch_millis(), utc.epoch_millis());
    let now = Timestamp::now();
    assert!((now.epoch_millis() - zphira_common::time::now_millis()).abs() < 60_000);
}
