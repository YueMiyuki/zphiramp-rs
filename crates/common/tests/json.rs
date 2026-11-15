mod support;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use zphira_common::{fs, json};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Category {
    Regular,
    Final,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct Inner {
    id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct Entry {
    chart_id: i32,
    chart_name: String,
    submission_open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_limit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rounds_per_stay: Option<i32>,
    chart_ids: Vec<i32>,
    votes: IndexMap<String, i32>,
    inner: Option<Inner>,
    history: Vec<Inner>,
    empty: Vec<i32>,
    empty_map: IndexMap<String, i32>,
    #[serde(rename = "ratingCount")]
    rating_count: i32,
    #[serde(
        deserialize_with = "json::enum_or_none",
        skip_serializing_if = "Option::is_none"
    )]
    category: Option<Category>,
}

#[test]
fn pretty_output_matches_the_java_data_file_layout() {
    let java = support::read_fixture("java_pretty_entry.json");
    let entry: Entry = serde_json::from_str(&java).unwrap();
    assert_eq!(entry.category, Some(Category::Regular));
    assert_eq!(entry.history[0].name, None);
    assert_eq!(serde_json::to_string_pretty(&entry).unwrap(), java);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data").join("entry.json");
    fs::write_json_pretty(&path, &entry).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), java);
}

#[test]
fn blank_documents_are_absent() {
    assert_eq!(json::from_str_opt::<Entry>("").unwrap(), None);
    assert_eq!(json::from_str_opt::<Entry>(" \n\t").unwrap(), None);
    assert_eq!(json::from_str_opt::<Entry>("null").unwrap(), None);
    assert_eq!(
        json::from_str_opt::<Entry>("{}").unwrap(),
        Some(Entry::default())
    );
    assert!(json::from_str_opt::<Entry>("{").is_err());
}

#[test]
fn unknown_or_null_enum_names_are_none() {
    let category = |text: &str| serde_json::from_str::<Entry>(text).map(|e| e.category);
    assert_eq!(
        category(r#"{"category":"FINAL"}"#).unwrap(),
        Some(Category::Final)
    );
    assert_eq!(category(r#"{"category":"BOGUS"}"#).unwrap(), None);
    assert_eq!(category(r#"{"category":null}"#).unwrap(), None);
    assert_eq!(category(r#"{"category":3}"#).unwrap(), None);
    assert_eq!(category("{}").unwrap(), None);
    assert!(category(r#"{"category":true}"#).is_err());
}
