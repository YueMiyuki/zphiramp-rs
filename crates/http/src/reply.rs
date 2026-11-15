use std::fmt;

use axum::body::Body;
use bytes::Bytes;
use http::header::{CONTENT_TYPE, HeaderValue};
use http::{Response, StatusCode};
use serde::Serialize;

pub(crate) const APPLICATION_JSON: &str = "application/json";
pub(crate) const TEXT_PLAIN: &str = "text/plain";
pub(crate) const TEXT_HTML_UTF8: &str = "text/html;charset=utf-8";

pub(crate) const INTERNAL_ERROR: &str = "服务器内部错误";
pub(crate) const BAD_FORMAT: &str = "请检查是否传入了错误的格式";
pub(crate) const BAD_PARAM: &str = "参数格式错误";

pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

pub(crate) enum Failure {
    Api { status: StatusCode, reason: String },
    Internal(BoxError),
    Plain(StatusCode, &'static str),
    Aborted(String),
}

impl Failure {
    pub(crate) fn api(status: u16, reason: impl Into<String>) -> Self {
        Self::Api {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            reason: reason.into(),
        }
    }

    pub(crate) fn bad_request(reason: impl Into<String>) -> Self {
        Self::api(400, reason)
    }

    pub(crate) fn bad_format() -> Self {
        Self::bad_request(BAD_FORMAT)
    }

    pub(crate) fn internal(error: impl Into<BoxError>) -> Self {
        Self::Internal(error.into())
    }

    pub(crate) fn internal_msg(message: impl Into<String>) -> Self {
        Self::Internal(message.into().into())
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Api { status, reason } => write!(f, "Api({status}, {reason})"),
            Self::Internal(e) => write!(f, "Internal({e})"),
            Self::Plain(status, text) => write!(f, "Plain({status}, {text})"),
            Self::Aborted(e) => write!(f, "Aborted({e})"),
        }
    }
}

pub(crate) fn text(
    status: StatusCode,
    content_type: &str,
    body: impl Into<Bytes>,
) -> Response<Body> {
    let mut response = Response::new(Body::from(body.into()));
    *response.status_mut() = status;
    if let Ok(value) = HeaderValue::from_str(content_type) {
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    response
}

pub(crate) fn plain(status: StatusCode, body: &'static str) -> Response<Body> {
    text(status, TEXT_PLAIN, body)
}

pub(crate) fn not_found() -> Response<Body> {
    plain(StatusCode::NOT_FOUND, "Not Found")
}

pub(crate) fn json<T: Serialize + ?Sized>(value: &T) -> Result<Response<Body>, Failure> {
    let body = serde_json::to_vec(value).map_err(Failure::internal)?;
    Ok(text(StatusCode::OK, APPLICATION_JSON, body))
}

pub(crate) fn api_error(status: StatusCode, reason: &str) -> Response<Body> {
    let body = serde_json::json!({"ok": false, "reason": reason}).to_string();
    text(status, APPLICATION_JSON, body)
}

pub(crate) fn ok() -> Result<Response<Body>, Failure> {
    Ok(text(StatusCode::OK, APPLICATION_JSON, "{\"ok\":true}"))
}
