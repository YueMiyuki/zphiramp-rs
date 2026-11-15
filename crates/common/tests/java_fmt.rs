mod support;

use support::read_tsv;
use zphira_common::java_fmt::{double_to_string, float_to_string};

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
