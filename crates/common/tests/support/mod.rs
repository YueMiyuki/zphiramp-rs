#![allow(dead_code)]

use std::path::PathBuf;

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                let cp = u32::from_str_radix(&hex, 16).expect("hex escape");
                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
            }
            other => panic!("bad escape {other:?} in {s:?}"),
        }
    }
    out
}

pub fn read_tsv(name: &str) -> Vec<Vec<String>> {
    read_fixture(name)
        .lines()
        .map(|l| l.split('\t').map(unescape).collect())
        .collect()
}
