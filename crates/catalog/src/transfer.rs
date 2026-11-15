use std::io;
use std::path::Path;

use zphira_common::gson::{JsonElement, JsonObject};

use crate::error::CatalogError;

pub const MAX_IMPORT_LENGTH: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Replace,
    Merge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// `imported` and `total` both count the rows held after the import.
pub struct ImportResult {
    pub imported: i32,
    pub total: i32,
    pub backup_file: String,
}

/// Called with the live file path under the store's lock, just before the file is replaced.
pub trait BackupFn: FnOnce(&Path) -> io::Result<String> + Send + 'static {}
impl<F: FnOnce(&Path) -> io::Result<String> + Send + 'static> BackupFn for F {}

pub(crate) fn document_rows(document: &str, field: &str) -> Result<Vec<JsonElement>, CatalogError> {
    if is_java_blank(document) {
        return Err(CatalogError::illegal_argument("内容为空"));
    }
    if document.encode_utf16().count() > MAX_IMPORT_LENGTH {
        return Err(CatalogError::illegal_argument("内容过大"));
    }
    let root = JsonElement::parse(document)
        .map_err(|_| CatalogError::illegal_argument("不是有效的 JSON"))?;
    if root.is_null() {
        return Err(CatalogError::illegal_argument("不是有效的 JSON"));
    }
    if !root.is_array() && !root.is_object() {
        return Err(CatalogError::illegal_argument("JSON 顶层必须是对象或数组"));
    }
    match root
        .as_object()
        .and_then(|o| o.get(field))
        .and_then(JsonElement::as_array)
    {
        Some(rows) => Ok(rows.clone()),
        None => Err(CatalogError::illegal_argument(format!(
            "缺少数组字段 {field}"
        ))),
    }
}

pub(crate) fn bad_row(field: &str, index: usize, reason: impl std::fmt::Display) -> CatalogError {
    CatalogError::illegal_argument(format!("{field}[{index}] 无效：{reason}"))
}

pub(crate) fn row_object<'a>(
    field: &str,
    index: usize,
    row: &'a JsonElement,
) -> Result<&'a JsonObject, CatalogError> {
    row.as_object()
        .ok_or_else(|| bad_row(field, index, "不是对象"))
}

pub(crate) fn has_value(object: &JsonObject, key: &str) -> bool {
    object.get(key).is_some_and(|v| !v.is_null())
}

pub(crate) fn check_enum(
    object: &JsonObject,
    key: &str,
    allowed: &[&str],
    null_allowed: bool,
    describe: impl FnOnce(String) -> String,
) -> Result<(), String> {
    let Some(value) = object.get(key) else {
        return Ok(());
    };
    if null_allowed && value.is_null() {
        return Ok(());
    }
    let name = match value {
        JsonElement::String(s) => Some(s.clone()),
        JsonElement::Number(n) => Some(n.as_str().to_owned()),
        _ => None,
    };
    match name {
        Some(name) if allowed.contains(&name.as_str()) => Ok(()),
        _ => Err(describe(value.to_string())),
    }
}

fn is_java_blank(s: &str) -> bool {
    s.chars().all(|c| {
        matches!(c, '\u{1c}'..='\u{1f}')
            || (c.is_whitespace() && !matches!(c, '\u{85}' | '\u{a0}' | '\u{2007}' | '\u{202f}'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_blank() {
        assert!(is_java_blank(""));
        assert!(is_java_blank(" \t\r\n\u{1c}\u{3000}"));
        assert!(!is_java_blank("\u{a0}"));
        assert!(!is_java_blank("\u{85}"));
        assert!(!is_java_blank(" x "));
    }

    #[test]
    fn shape_checks_use_the_java_messages() {
        let msg = |doc: &str| document_rows(doc, "pools").unwrap_err().message();
        assert_eq!(msg("  "), "内容为空");
        assert_eq!(msg("{"), "不是有效的 JSON");
        assert_eq!(msg("null"), "不是有效的 JSON");
        assert_eq!(msg("not-json"), "JSON 顶层必须是对象或数组");
        assert_eq!(msg("5"), "JSON 顶层必须是对象或数组");
        assert_eq!(msg("[]"), "缺少数组字段 pools");
        assert_eq!(msg("{\"pools\":{}}"), "缺少数组字段 pools");
        assert_eq!(msg("{\"pool\":[]}"), "缺少数组字段 pools");
        assert_eq!(
            document_rows("{\"pools\":[1,null]}", "pools")
                .unwrap()
                .len(),
            2
        );
    }
}
