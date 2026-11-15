use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GsonErrorKind {
    Syntax,
    Data,
    NumberFormat,
    Custom,
    Null,
    Serialize,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GsonError {
    kind: GsonErrorKind,
    message: String,
    pub(crate) null_field: Option<NullField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct NullField {
    pub(crate) struct_name: &'static str,
    pub(crate) fields: usize,
    pub(crate) key: String,
}

impl GsonError {
    pub(crate) fn new(kind: GsonErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            null_field: None,
        }
    }

    pub(crate) fn syntax(message: impl Into<String>) -> Self {
        Self::new(GsonErrorKind::Syntax, message)
    }

    pub(crate) fn data(message: impl Into<String>) -> Self {
        Self::new(GsonErrorKind::Data, message)
    }

    pub fn kind(&self) -> GsonErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GsonError {}

impl serde::ser::Error for GsonError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::new(GsonErrorKind::Serialize, msg.to_string())
    }
}

impl serde::de::Error for GsonError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::new(GsonErrorKind::Custom, msg.to_string())
    }
}

impl From<std::io::Error> for GsonError {
    fn from(e: std::io::Error) -> Self {
        Self::new(GsonErrorKind::Io, e.to_string())
    }
}
