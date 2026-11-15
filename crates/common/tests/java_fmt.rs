mod support;

use support::read_tsv;
use zphira_common::java_fmt::{
    double_to_string, float_to_string, format_fixed, parse_boolean, parse_double, parse_int,
    parse_long,
};

#[test]
fn float_to_string_matches_jdk() {
    let rows = read_tsv("float_to_string.tsv");
    assert!(rows.len() > 5000);
    let mut failures = Vec::new();
    for row in &rows {
        let bits = u32::from_str_radix(&row[0], 16).unwrap();
        let got = float_to_string(f32::from_bits(bits));
        if got != row[1] {
            failures.push(format!("{bits:08x}: java={} rust={got}", row[1]));
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
fn double_to_string_matches_jdk() {
    let rows = read_tsv("double_to_string.tsv");
    assert!(rows.len() > 5000);
    let mut failures = Vec::new();
    for row in &rows {
        let bits = u64::from_str_radix(&row[0], 16).unwrap();
        let got = double_to_string(f64::from_bits(bits));
        if got != row[1] {
            failures.push(format!("{bits:016x}: java={} rust={got}", row[1]));
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
fn format_fixed_matches_jdk() {
    let rows = read_tsv("format_fixed.tsv");
    assert!(rows.len() > 3000);
    let mut failures = Vec::new();
    for row in &rows {
        let bits = u64::from_str_radix(&row[0], 16).unwrap();
        let prec: usize = row[1].parse().unwrap();
        let got = format_fixed(f64::from_bits(bits), prec);
        if got != row[2] {
            failures.push(format!(
                "{} %.{prec}f: java={} rust={got}",
                f64::from_bits(bits),
                row[2]
            ));
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
fn parse_double_matches_jdk() {
    let mut failures = Vec::new();
    for row in read_tsv("parse_double.tsv") {
        let got = match parse_double(&row[0]) {
            Ok(v) => format!("{:016x}", v.to_bits()),
            Err(e) => format!("ERR {}", e.message()),
        };
        if got != row[1] {
            failures.push(format!("{:?}: java={} rust={got}", row[0], row[1]));
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
fn parse_int_long_boolean_match_jdk() {
    let mut failures = Vec::new();
    for row in read_tsv("parse_int.tsv") {
        let int = match parse_int(&row[0]) {
            Ok(v) => v.to_string(),
            Err(e) => format!("ERR {}", e.message()),
        };
        let long = match parse_long(&row[0]) {
            Ok(v) => v.to_string(),
            Err(e) => format!("ERR {}", e.message()),
        };
        let boolean = parse_boolean(&row[0]).to_string();
        if int != row[1] || long != row[2] || boolean != row[3] {
            failures.push(format!(
                "{:?}: java={:?} rust={:?}",
                row[0],
                &row[1..],
                [int, long, boolean]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
