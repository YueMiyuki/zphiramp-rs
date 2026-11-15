use std::fmt;

use axum::body::Body;
use bytes::Bytes;
use http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use http::{HeaderMap, Response, StatusCode};
use serde::Serialize;
use zphira_common::gson;

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
    Javalin(JavalinError),
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
        Self::Internal(Box::new(Message(message.into())))
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Api { status, reason } => write!(f, "Api({status}, {reason})"),
            Self::Internal(e) => write!(f, "Internal({e})"),
            Self::Javalin(e) => write!(f, "Javalin({})", e.title),
            Self::Aborted(e) => write!(f, "Aborted({e})"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Message(pub(crate) String);

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Message {}

pub(crate) struct JavalinError {
    pub(crate) status: StatusCode,
    pub(crate) title: String,
    pub(crate) type_anchor: &'static str,
}

impl JavalinError {
    pub(crate) fn endpoint_not_found(method: &str, path: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            title: format!("Endpoint {method} {path} not found"),
            type_anchor: "endpointnotfound",
        }
    }

    pub(crate) fn content_too_large() -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            title: "Content Too Large".to_owned(),
            type_anchor: "error-responses",
        }
    }

    pub(crate) fn render(&self, headers: &HeaderMap) -> Response<Body> {
        let accept = headers
            .get(http::header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if accept.contains("text/html") {
            text(self.status, "text/html", self.title.clone())
        } else if accept.contains(APPLICATION_JSON) {
            let body = format!(
                "{{\n    \"title\": \"{}\",\n    \"status\": {},\n    \"type\": \"https://javalin.io/documentation#{}\",\n    \"details\": {{}}\n}}",
                json_escape(&self.title),
                self.status.as_u16(),
                self.type_anchor
            );
            text(self.status, APPLICATION_JSON, body)
        } else {
            text(self.status, TEXT_PLAIN, self.title.clone())
        }
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
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

pub(crate) fn empty(status: StatusCode) -> Response<Body> {
    text(status, TEXT_PLAIN, Bytes::new())
}

pub(crate) fn json<T: Serialize + ?Sized>(value: &T) -> Result<Response<Body>, Failure> {
    let body = gson::to_vec_compact(value).map_err(Failure::internal)?;
    Ok(text(StatusCode::OK, APPLICATION_JSON, body))
}

pub(crate) fn api_error(status: StatusCode, reason: &str) -> Response<Body> {
    #[derive(Serialize)]
    struct Body<'a> {
        ok: bool,
        reason: &'a str,
    }
    let body = gson::to_vec_compact(&Body { ok: false, reason })
        .unwrap_or_else(|_| b"{\"ok\":false}".to_vec());
    text(status, APPLICATION_JSON, body)
}

pub(crate) fn ok() -> Result<Response<Body>, Failure> {
    Ok(text(
        StatusCode::OK,
        APPLICATION_JSON,
        &b"{\"ok\":true}"[..],
    ))
}

pub(crate) fn jetty_bad_request() -> Response<Body> {
    const PAGE: &str = "<html>\n<head>\n<meta http-equiv=\"Content-Type\" content=\"text/html;charset=ISO-8859-1\"/>\n<title>Error 400 Bad Request</title>\n</head>\n<body>\n<h2>HTTP ERROR 400 Bad Request</h2>\n<table>\n<tr><th>URI:</th><td>/badMessage</td></tr>\n<tr><th>STATUS:</th><td>400</td></tr>\n<tr><th>MESSAGE:</th><td>Bad Request</td></tr>\n</table>\n\n</body>\n</html>\n";
    let mut response = text(
        StatusCode::BAD_REQUEST,
        "text/html;charset=iso-8859-1",
        PAGE,
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("must-revalidate,no-cache,no-store"),
    );
    response
}

pub(crate) fn jetty_not_found() -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NOT_FOUND;
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("must-revalidate,no-cache,no-store"),
    );
    response
}
