use std::io;
use std::path::Path;

use serde_json::{Map, Value};

use crate::error::CatalogError;

pub const MAX_IMPORT_LENGTH: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Replace,
    Merge,
}

/// `imported` and `total` both count the rows held after the import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportResult {
    pub imported: i32,
    pub total: i32,
    pub backup_file: String,
}

/// Called with the live file path under the store's lock, just before the file is replaced.
pub trait BackupFn: FnOnce(&Path) -> io::Result<String> + Send + 'static {}
impl<F: FnOnce(&Path) -> io::Result<String> + Send + 'static> BackupFn for F {}

pub(crate) fn document_rows(document: &str, field: &str) -> Result<Vec<Value>, CatalogError> {
    if document.trim().is_empty() {
        return Err(CatalogError::invalid("内容为空"));
    }
    if document.len() > MAX_IMPORT_LENGTH {
        return Err(CatalogError::invalid("内容过大"));
    }
    let root: Value =
        serde_json::from_str(document).map_err(|_| CatalogError::invalid("不是有效的 JSON"))?;
    if root.is_null() {
        return Err(CatalogError::invalid("不是有效的 JSON"));
    }
    if !root.is_array() && !root.is_object() {
        return Err(CatalogError::invalid("JSON 顶层必须是对象或数组"));
    }
    match root.get(field).and_then(Value::as_array) {
        Some(rows) => Ok(rows.clone()),
        None => Err(CatalogError::invalid(format!("缺少数组字段 {field}"))),
    }
}

pub(crate) fn bad_row(field: &str, index: usize, reason: impl std::fmt::Display) -> CatalogError {
    CatalogError::invalid(format!("{field}[{index}] 无效：{reason}"))
}

pub(crate) fn row_object<'a>(
    field: &str,
    index: usize,
    row: &'a Value,
) -> Result<&'a Map<String, Value>, CatalogError> {
    row.as_object()
        .ok_or_else(|| bad_row(field, index, "不是对象"))
}

pub(crate) fn has_value(object: &Map<String, Value>, key: &str) -> bool {
    object.get(key).is_some_and(|v| !v.is_null())
}

pub(crate) fn check_enum(
    object: &Map<String, Value>,
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
    match value.as_str() {
        Some(name) if allowed.contains(&name) => Ok(()),
        _ => Err(describe(value.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_checks_and_their_messages() {
        let msg = |doc: &str| document_rows(doc, "pools").unwrap_err().message();
        assert_eq!(msg("  "), "内容为空");
        assert_eq!(msg("{"), "不是有效的 JSON");
        assert_eq!(msg("null"), "不是有效的 JSON");
        assert_eq!(msg("not-json"), "不是有效的 JSON");
        assert_eq!(msg("\"text\""), "JSON 顶层必须是对象或数组");
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
