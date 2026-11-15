use std::fmt;

use serde_json::Value;

pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Replace,
    Merge,
}

impl ImportMode {
    pub fn from_merge(merge: bool) -> Self {
        if merge { Self::Merge } else { Self::Replace }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: usize,
    pub total: usize,
}

impl ImportReport {
    pub(crate) fn rows(rows: usize) -> Self {
        Self {
            imported: rows,
            total: rows,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportResult {
    pub imported: usize,
    pub total: usize,
    pub backup_file: String,
}

/// Receives the serialized current state under the store lock, right before the file is written.
pub trait BackupFn: FnOnce(&str) -> std::io::Result<String> {}
impl<F: FnOnce(&str) -> std::io::Result<String>> BackupFn for F {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Invalid(String),
    Write(String),
}

impl ImportError {
    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid(_))
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(m) | Self::Write(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ImportError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Admins,
    Points,
    Records,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Admins => "admins",
            Self::Points => "points",
            Self::Records => "records",
        }
    }
}

pub(crate) fn parse_document(kind: Kind, document: &str) -> Result<Value, ImportError> {
    if document.trim().is_empty() {
        return Err(ImportError::invalid("内容为空"));
    }
    if document.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::invalid("内容过大"));
    }
    let root: Value =
        serde_json::from_str(document).map_err(|_| ImportError::invalid("不是有效的 JSON"))?;
    if root.is_null() {
        return Err(ImportError::invalid("不是有效的 JSON"));
    }
    if !root.is_array() && !root.is_object() {
        return Err(ImportError::invalid("JSON 顶层必须是对象或数组"));
    }
    match kind {
        Kind::Admins | Kind::Records => {
            if !root.is_array() {
                return Err(ImportError::invalid(format!("{} 应为数组", kind.key())));
            }
        }
        Kind::Points => {
            if !root.is_object() {
                return Err(ImportError::invalid("玩家积分应为对象（用户 ID 到积分）"));
            }
        }
    }
    Ok(root)
}

pub(crate) fn present<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a Value> {
    object.get(key).filter(|value| !value.is_null())
}

pub(crate) fn check_integer(
    object: &serde_json::Map<String, Value>,
    key: &str,
    wide: bool,
) -> Result<(), String> {
    match object.get(key) {
        None => Ok(()),
        Some(Value::Null) => Err(format!("{key} 不能为 null")),
        Some(value) => match value.as_i64() {
            Some(n) if wide || i32::try_from(n).is_ok() => Ok(()),
            _ => Err(format!("{key} 必须是整数")),
        },
    }
}

pub(crate) fn check_number(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<(), String> {
    match object.get(key) {
        None => Ok(()),
        Some(Value::Null) => Err(format!("{key} 不能为 null")),
        Some(Value::Number(_)) => Ok(()),
        Some(_) => Err(format!("{key} 必须是数字")),
    }
}

pub(crate) fn check_text(
    object: &serde_json::Map<String, Value>,
    key: &str,
    nullable: bool,
) -> Result<(), String> {
    match object.get(key) {
        None | Some(Value::String(_)) => Ok(()),
        Some(Value::Null) if nullable => Ok(()),
        Some(_) => Err(format!("{key} 必须是文本")),
    }
}
