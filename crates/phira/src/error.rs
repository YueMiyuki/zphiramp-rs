use std::fmt;

#[derive(Debug)]
pub struct PhiraError {
    kind: Kind,
}

#[derive(Debug)]
pub(crate) enum Kind {
    Status { status: u16, body: String },
    Transport(String),
    Decode(zphira_common::gson::GsonError),
    EmptyBody,
    InvalidUrl(String),
    InvalidRequest(String),
    TooLarge(usize),
    Probe(String),
}

impl PhiraError {
    pub(crate) fn new(kind: Kind) -> Self {
        Self { kind }
    }

    pub(crate) fn status_error(status: u16, body: String) -> Self {
        Self::new(Kind::Status { status, body })
    }

    pub(crate) fn transport(error: &reqwest::Error) -> Self {
        let headline = if error.is_timeout() {
            "HTTP request timed out"
        } else if error.is_connect() {
            "HTTP connect failed"
        } else {
            "HTTP request failed"
        };
        Self::transport_with_cause(headline, Some(error))
    }

    pub(crate) fn transport_with_cause(
        headline: &str,
        cause: Option<&dyn std::error::Error>,
    ) -> Self {
        Self::new(Kind::Transport(describe_transport(headline, cause)))
    }

    pub(crate) fn probe(message: impl Into<String>) -> Self {
        Self::new(Kind::Probe(message.into()))
    }

    pub fn status(&self) -> Option<u16> {
        match self.kind {
            Kind::Status { status, .. } => Some(status),
            _ => None,
        }
    }

    pub fn body(&self) -> Option<&str> {
        match &self.kind {
            Kind::Status { body, .. } => Some(body),
            _ => None,
        }
    }

    /// No complete answer arrived: connect, TLS, timeout or reset.
    pub fn is_transport(&self) -> bool {
        matches!(self.kind, Kind::Transport(_))
    }

    pub(crate) fn is_retryable(&self) -> bool {
        match &self.kind {
            Kind::Status { status, .. } => *status == 429 || (500..600).contains(status),
            Kind::Transport(_) => true,
            _ => false,
        }
    }
}

impl fmt::Display for PhiraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::Status { status, body } => write!(f, "HTTP {status}: {body}"),
            Kind::Transport(text) => f.write_str(text),
            Kind::Decode(error) => fmt::Display::fmt(error, f),
            Kind::EmptyBody => f.write_str("Empty response from Phira"),
            Kind::InvalidUrl(text) => write!(f, "Invalid URL: {text}"),
            Kind::InvalidRequest(text) => write!(f, "Invalid request: {text}"),
            Kind::TooLarge(limit) => write!(f, "Response larger than {limit} bytes"),
            Kind::Probe(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for PhiraError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            Kind::Decode(error) => Some(error),
            _ => None,
        }
    }
}

impl From<zphira_common::gson::GsonError> for PhiraError {
    fn from(error: zphira_common::gson::GsonError) -> Self {
        Self::new(Kind::Decode(error))
    }
}

fn describe_transport(headline: &str, error: Option<&dyn std::error::Error>) -> String {
    let mut text = headline.to_owned();
    let mut source = error.and_then(std::error::Error::source);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
