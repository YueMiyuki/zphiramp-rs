use std::fmt;
use std::io;
use std::sync::Arc;

use crate::upstream::UpstreamError;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CatalogError {
    Invalid(String),
    NotFound(String),
    Upstream(UpstreamError),
    Storage {
        message: String,
        source: Option<Arc<io::Error>>,
    },
}

impl CatalogError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub(crate) fn storage(error: io::Error) -> Self {
        Self::Storage {
            message: error.to_string(),
            source: Some(Arc::new(error)),
        }
    }

    pub(crate) fn storage_with(message: &str, error: io::Error) -> Self {
        Self::Storage {
            message: message.to_owned(),
            source: Some(Arc::new(error)),
        }
    }

    pub(crate) fn bad_data(message: impl Into<String>) -> Self {
        Self::Storage {
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn json(error: serde_json::Error) -> Self {
        Self::storage(error.into())
    }

    pub fn message(&self) -> String {
        match self {
            Self::Invalid(message) | Self::NotFound(message) => message.clone(),
            Self::Upstream(e) => e.message().to_owned(),
            Self::Storage { message, .. } => message.clone(),
        }
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
            Self::Upstream(e) => Some(e),
            Self::Storage {
                source: Some(source),
                ..
            } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<UpstreamError> for CatalogError {
    fn from(e: UpstreamError) -> Self {
        Self::Upstream(e)
    }
}
