use std::collections::BTreeSet;

use zphira_catalog::{PoolCategory, ScreeningRule};
use zphira_phira::ChartInfo;

fn chart(
    tags: &str,
    rating: &str,
    votes: i32,
    difficulty: &str,
    duration: Option<i32>,
) -> ChartInfo {
    let json = format!(
        "{{\"tags\":[{tags}],\"rating\":{rating},\"ratingCount\":{votes},\"difficulty\":{difficulty}{}}}",
        duration.map_or(String::new(), |d| format!(",\"duration_seconds\":{d}"))
    );
    serde_json::from_str(&json).unwrap()
}

fn tags(rule: &ScreeningRule) -> Vec<&str> {
    rule.any_tags.iter().map(String::as_str).collect()
}

#[test]
fn presets_match_the_activity_rules() {
    let regular = ScreeningRule::of(PoolCategory::Regular).unwrap();
    assert_eq!(regular.min_rating, 0.84f32);
    assert_eq!(regular.min_difficulty, Some(17.0));
    assert_eq!(tags(&regular), ["regular"]);
    assert_eq!(regular.min_rating_count, 5);
    assert_eq!(regular.category, PoolCategory::Regular);

    let configured = ScreeningRule::of(PoolCategory::Configured).unwrap();
    assert_eq!(configured.min_rating, 0.80f32);
    assert_eq!(configured.min_difficulty, None);
    assert_eq!(tags(&configured), ["plain"]);

    let tb = ScreeningRule::of(PoolCategory::Tb).unwrap();
    assert_eq!(tb.min_rating, 0.84f32);
    assert_eq!(tb.min_duration_seconds, Some(360));
    assert_eq!(
        tb.any_tags,
        BTreeSet::from(["regular".to_owned(), "plain".to_owned()])
    );

    assert!(ScreeningRule::of(PoolCategory::Manual).is_none());
}

#[test]
fn regular_needs_both_rating_and_difficulty() {
    let rule = ScreeningRule::of(PoolCategory::Regular).unwrap();
    assert!(rule.matches(&chart("\"regular\"", "0.9", 10, "17.5", None)));
    assert!(!rule.matches(&chart("\"regular\"", "0.9", 10, "16.9", None)));
    assert!(!rule.matches(&chart("\"regular\"", "0.8", 10, "17.5", None)));
    assert!(!rule.matches(&chart("\"plain\"", "0.9", 10, "17.5", None)));
}

#[test]
fn rating_boundary_is_exclusive() {
    let rule = ScreeningRule::of(PoolCategory::Configured).unwrap();
    assert!(!rule.matches(&chart("\"plain\"", "0.8", 10, "0.0", None)));
    assert!(rule.matches(&chart("\"plain\"", "0.81", 10, "0.0", None)));
}

#[test]
fn difficulty_boundary_is_inclusive() {
    let rule = ScreeningRule::of(PoolCategory::Regular).unwrap();
    assert!(rule.matches(&chart("\"regular\"", "0.9", 10, "17.0", None)));
    assert!(!rule.matches(&chart("\"regular\"", "0.9", 10, "16.9", None)));
}

#[test]
fn a_single_vote_cannot_qualify() {
    let rule = ScreeningRule::of(PoolCategory::Configured).unwrap();
    assert!(!rule.matches(&chart("\"plain\"", "0.99", 4, "0.0", None)));
    assert!(rule.matches(&chart("\"plain\"", "0.99", 5, "0.0", None)));
}

#[test]
fn tb_accepts_both_kinds_but_needs_a_probed_duration() {
    let rule = ScreeningRule::of(PoolCategory::Tb).unwrap();
    assert!(rule.matches(&chart("\"regular\"", "0.9", 10, "0.0", Some(400))));
    assert!(rule.matches(&chart("\"plain\"", "0.9", 10, "0.0", Some(400))));
    assert!(!rule.matches(&chart("\"regular\"", "0.9", 10, "0.0", Some(300))));
    assert!(!rule.matches(&chart("\"regular\"", "0.9", 10, "0.0", None)));
    assert!(!rule.matches(&chart("\"other\"", "0.9", 10, "0.0", Some(400))));
}

#[test]
fn rating_is_compared_in_f32_and_difficulty_in_widened_f64() {
    let rule = ScreeningRule::regular();
    assert!(!rule.matches(&chart("\"regular\"", "0.84", 10, "17.5", None)));
    assert!(rule.matches(&chart("\"regular\"", "0.84000003", 10, "17.5", None)));
    let custom = ScreeningRule::new(
        PoolCategory::Manual,
        Vec::<String>::new(),
        0.0,
        0,
        Some(16.9),
        None,
    );
    assert!(!custom.matches(&chart("\"regular\"", "0.9", 10, "16.9", None)));
    assert!(custom.matches(&chart("\"regular\"", "0.9", 10, "16.900002", None)));
    let tb = ScreeningRule::tb();
    assert!(tb.matches(&chart("\"plain\"", "0.9", 10, "0.0", Some(360))));
    assert!(!tb.matches(&chart("\"plain\"", "0.9", 10, "0.0", Some(359))));
}

#[test]
fn empty_tags_and_absent_bounds_are_not_checked() {
    let rule = ScreeningRule::new(
        PoolCategory::Manual,
        Vec::<String>::new(),
        0.0,
        0,
        None,
        None,
    );
    assert!(rule.matches(&chart("", "0.5", 0, "1.0", None)));
    assert!(
        !rule.matches(&chart("", "0.0", 0, "1.0", None)),
        "rating 0 is not above 0"
    );
    let no_tags: ChartInfo = serde_json::from_str("{\"rating\":0.9}").unwrap();
    assert!(rule.matches(&no_tags));
    assert!(
        !ScreeningRule::regular().matches(&no_tags),
        "a chart without tags has no tag"
    );
}

#[test]
fn without_duration_drops_only_the_duration_bound() {
    let relaxed = ScreeningRule::tb().without_duration();
    assert_eq!(relaxed.min_duration_seconds, None);
    assert_eq!(relaxed.category, PoolCategory::Tb);
    assert_eq!(relaxed.min_rating, 0.84f32);
    assert!(relaxed.matches(&chart("\"plain\"", "0.9", 10, "0.0", None)));
}

#[test]
fn categories_parse_like_value_of() {
    assert_eq!(PoolCategory::from_name("TB"), Some(PoolCategory::Tb));
    assert_eq!(PoolCategory::from_name("tb"), None);
    assert_eq!(
        PoolCategory::parse_ignore_case(" configured\t"),
        Some(PoolCategory::Configured)
    );
    assert_eq!(
        PoolCategory::parse_ignore_case("Manual"),
        Some(PoolCategory::Manual)
    );
    assert_eq!(PoolCategory::parse_ignore_case("plain"), None);
    assert_eq!(
        PoolCategory::ALL.map(PoolCategory::name),
        ["REGULAR", "CONFIGURED", "TB", "MANUAL"]
    );
    assert_eq!(serde_json::to_string(&PoolCategory::Tb).unwrap(), "\"TB\"");
}
