use std::fmt;
use std::io;
use std::sync::Arc;

use zphira_common::gson::GsonError;

use crate::upstream::UpstreamError;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CatalogError {
    IllegalArgument(String),
    IllegalState {
        message: String,
        cause: Option<Arc<io::Error>>,
    },
    Upstream(UpstreamError),
    Io(Arc<io::Error>),
    Json(GsonError),
}

impl CatalogError {
    pub(crate) fn illegal_argument(message: impl Into<String>) -> Self {
        Self::IllegalArgument(message.into())
    }

    pub(crate) fn illegal_state(message: impl Into<String>) -> Self {
        Self::IllegalState {
            message: message.into(),
            cause: None,
        }
    }

    pub(crate) fn save_failed(message: &str, cause: io::Error) -> Self {
        Self::IllegalState {
            message: message.to_owned(),
            cause: Some(Arc::new(cause)),
        }
    }

    pub(crate) fn io(error: io::Error) -> Self {
        Self::Io(Arc::new(error))
    }

    pub fn message(&self) -> String {
        match self {
            Self::IllegalArgument(message) => message.clone(),
            Self::IllegalState { message, .. } => message.clone(),
            Self::Upstream(e) => e.message().to_owned(),
            Self::Io(e) => e.to_string(),
            Self::Json(e) => e.message().to_owned(),
        }
    }

    pub fn java_class(&self) -> &'static str {
        match self {
            Self::IllegalArgument(_) => "java.lang.IllegalArgumentException",
            Self::IllegalState { .. } => "java.lang.IllegalStateException",
            Self::Upstream(_) | Self::Io(_) => "java.io.IOException",
            Self::Json(_) => "com.google.gson.JsonSyntaxException",
        }
    }

    pub fn is_io_exception(&self) -> bool {
        matches!(self, Self::Upstream(_) | Self::Io(_))
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IllegalState {
                cause: Some(cause), ..
            } => Some(cause.as_ref()),
            Self::Upstream(e) => Some(e),
            Self::Io(e) => Some(e.as_ref()),
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<UpstreamError> for CatalogError {
    fn from(e: UpstreamError) -> Self {
        Self::Upstream(e)
    }
}
