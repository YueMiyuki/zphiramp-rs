use std::fmt;

use zphira_common::gson::JsonElement;

pub const MAX_IMPORT_CHARS: usize = 64 * 1024 * 1024;

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

pub(crate) fn gson_reason(error: &zphira_common::gson::GsonError) -> String {
    use zphira_common::gson::GsonErrorKind;
    match error.kind() {
        GsonErrorKind::NumberFormat => format!("数值格式错误：{}", error.message()),
        _ => error.message().to_owned(),
    }
}

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

pub(crate) fn parse_document(kind: Kind, document: &str) -> Result<JsonElement, ImportError> {
    if document.trim().is_empty() {
        return Err(ImportError::invalid("内容为空"));
    }
    if document.encode_utf16().count() > MAX_IMPORT_CHARS {
        return Err(ImportError::invalid("内容过大"));
    }
    let root = JsonElement::parse(document).map_err(|_| ImportError::invalid("不是有效的 JSON"))?;
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
