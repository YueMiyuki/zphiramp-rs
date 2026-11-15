mod support;

use std::cmp::Ordering;

use support::read_tsv;
use zphira_common::time::JavaOffsetDateTime;

#[test]
fn parse_matches_jdk() {
    let rows = read_tsv("offset_date_time_parse.tsv");
    assert!(rows.len() > 100);
    let mut failures = Vec::new();
    for row in &rows {
        let input = &row[0];
        let expected = &row[1];
        let got = match JavaOffsetDateTime::parse(input) {
            Ok(t) => {
                let millis = t
                    .to_epoch_milli()
                    .map_or("OVERFLOW".to_owned(), |m| m.to_string());
                format!(
                    "{t}\t{millis}\t{}\t{}\t{}",
                    t.offset_seconds(),
                    t.epoch_second(),
                    t.nano()
                )
            }
            Err(e) => format!("ERR DateTimeParseException: {}", e.message()),
        };
        if &got != expected {
            failures.push(format!("{input:?}\n  java: {expected}\n  rust: {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn format_matches_jdk() {
    let rows = read_tsv("offset_date_time_format.tsv");
    assert!(rows.len() > 1000);
    let mut failures = Vec::new();
    for row in &rows {
        let sec: i64 = row[0].parse().unwrap();
        let nano: u32 = row[1].parse().unwrap();
        let off: i32 = row[2].parse().unwrap();
        let t = JavaOffsetDateTime::from_instant(sec, nano, off).expect("in range");
        let millis = t
            .to_epoch_milli()
            .map_or("OVERFLOW".to_owned(), |m| m.to_string());
        let got = format!("{t}\t{millis}");
        let expected = format!("{}\t{}", row[3], row[4]);
        if got != expected {
            failures.push(format!("{sec} {nano} {off}: java={expected} rust={got}"));
        }
        let back = JavaOffsetDateTime::parse(&row[3]).expect("toString output parses");
        if back != t || back.epoch_second() != sec {
            failures.push(format!("round trip of {}", row[3]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn compare_matches_jdk() {
    let rows = read_tsv("offset_date_time_compare.tsv");
    assert!(rows.len() > 100);
    for row in &rows {
        let a = JavaOffsetDateTime::parse(&row[0]).unwrap();
        let b = JavaOffsetDateTime::parse(&row[1]).unwrap();
        let expected_cmp = match row[2].as_str() {
            "-1" => Ordering::Less,
            "0" => Ordering::Equal,
            _ => Ordering::Greater,
        };
        assert_eq!(a.cmp(&b), expected_cmp, "{} vs {}", row[0], row[1]);
        assert_eq!((a == b).to_string(), row[3], "equals {} {}", row[0], row[1]);
        assert_eq!(
            a.is_same_instant(&b).to_string(),
            row[4],
            "isEqual {} {}",
            row[0],
            row[1]
        );
    }
}

#[test]
fn epoch_millis_constructor() {
    let t = JavaOffsetDateTime::from_epoch_millis(-1, 0).unwrap();
    assert_eq!(t.to_string(), "1969-12-31T23:59:59.999Z");
    assert_eq!(t.to_epoch_milli(), Some(-1));
    let t = JavaOffsetDateTime::from_epoch_millis(1_794_670_006_000, 8 * 3600).unwrap();
    assert_eq!(t.to_string(), "2026-11-14T23:26:46+08:00");
    assert!(JavaOffsetDateTime::from_epoch_millis(0, 18 * 3600 + 1).is_none());
}
