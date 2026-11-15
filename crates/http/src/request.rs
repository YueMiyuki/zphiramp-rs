use std::collections::HashMap;
use std::net::SocketAddr;

use axum::body::Body;
use bytes::{Bytes, BytesMut};
use http::HeaderMap;
use http_body_util::BodyExt;

use crate::java::{self, Charset};
use crate::reply::{Failure, JavalinError};

pub const DEFAULT_MAX_REQUEST_SIZE: usize = 1_000_000;

async fn drain(body: &mut Body) {
    let drained = async {
        let mut left = DRAIN_LIMIT;
        while let Some(Ok(frame)) = body.frame().await {
            let len = frame.data_ref().map_or(0, bytes::Buf::remaining);
            if len >= left {
                break;
            }
            left -= len;
        }
    };
    let _ = tokio::time::timeout(DRAIN_TIME, drained).await;
}

const DRAIN_LIMIT: usize = 1024 * 1024;
const DRAIN_TIME: std::time::Duration = std::time::Duration::from_secs(1);

pub(crate) struct Ctx {
    pub(crate) method: String,
    pub(crate) path: String,
    query: Option<String>,
    pub(crate) headers: HeaderMap,
    body: std::sync::Mutex<Option<Body>>,
    body_bytes: Option<Bytes>,
    query_params: Option<HashMap<String, Vec<String>>>,
    pub(crate) client: Option<SocketAddr>,
    pub(crate) user_id: Option<i32>,
    pub(crate) raw_params: Vec<(&'static str, String)>,
    pub(crate) max_body: usize,
    pub(crate) body_idle_timeout: std::time::Duration,
}

impl Ctx {
    pub(crate) fn new(request: http::Request<Body>, client: Option<SocketAddr>) -> Self {
        let (parts, body) = request.into_parts();
        let method = parts.method.as_str().to_owned();
        Self {
            method,
            path: parts.uri.path().to_owned(),
            query: parts.uri.query().map(str::to_owned),
            headers: parts.headers,
            body: std::sync::Mutex::new(Some(body)),
            body_bytes: None,
            query_params: None,
            client,
            user_id: None,
            raw_params: Vec::new(),
            max_body: DEFAULT_MAX_REQUEST_SIZE,
            body_idle_timeout: std::time::Duration::from_secs(30),
        }
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim_matches([' ', '\t']))
    }

    fn request_charset(&self) -> Result<Option<Charset>, String> {
        let Some(content_type) = self.headers.get(http::header::CONTENT_TYPE) else {
            return Ok(None);
        };
        let value = String::from_utf8_lossy(content_type.as_bytes());
        let Some(part) = value
            .split(';')
            .find(|p| java::trim(p).to_ascii_lowercase().starts_with("charset"))
        else {
            return Ok(None);
        };
        let Some(name) = part.split('=').nth(1) else {
            return Err("Index 1 out of bounds for length 1".to_owned());
        };
        let name = java::trim(name);
        let name = name
            .strip_prefix('"')
            .and_then(|n| n.strip_suffix('"'))
            .unwrap_or(name);
        Charset::for_name(name)
            .map(Some)
            .ok_or_else(|| name.to_owned())
    }

    pub(crate) fn query_param(&mut self, name: &str) -> Option<&str> {
        if self.query_params.is_none() {
            self.query_params = Some(self.parse_query());
        }
        self.query_params
            .as_ref()
            .and_then(|m| m.get(name))
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    fn parse_query(&self) -> HashMap<String, Vec<String>> {
        let mut params: HashMap<String, Vec<String>> = HashMap::new();
        let query = match self.query.as_deref() {
            Some(q) if !q.is_empty() => q,
            _ => return params,
        };
        let Ok(charset) = self.request_charset() else {
            return params;
        };
        let charset = charset.unwrap_or(Charset::Utf8);
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let Ok(key) = java::url_decode(key, charset) else {
                continue;
            };
            let entry = params.entry(key).or_default();
            if let Ok(value) = java::url_decode(value, charset) {
                entry.push(value);
            }
        }
        params
    }

    pub(crate) async fn body_bytes(&mut self) -> Result<Bytes, Failure> {
        if let Some(bytes) = &self.body_bytes {
            return Ok(bytes.clone());
        }
        let taken = self
            .body
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(mut body) = taken else {
            return Ok(Bytes::new());
        };
        let mut buf = BytesMut::new();
        loop {
            let frame = match tokio::time::timeout(self.body_idle_timeout, body.frame()).await {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(_) => {
                    return Err(Failure::Aborted(
                        "idle timeout while reading the body".into(),
                    ));
                }
            };
            let frame = match frame {
                Ok(frame) => frame,
                Err(e) => return Err(Failure::Aborted(e.to_string())),
            };
            if let Ok(data) = frame.into_data() {
                if buf.len() + data.len() > self.max_body {
                    tracing::warn!(
                        target: "io.javalin.Javalin",
                        "Body size greater than max size ({} bytes)",
                        self.max_body
                    );
                    drain(&mut body).await;
                    return Err(Failure::Javalin(JavalinError::content_too_large()));
                }
                buf.extend_from_slice(&data);
            }
        }
        let bytes = buf.freeze();
        self.body_bytes = Some(bytes.clone());
        Ok(bytes)
    }

    pub(crate) async fn body_string(&mut self) -> Result<String, Failure> {
        let bytes = self.body_bytes().await?;
        let charset = self
            .request_charset()
            .map_err(|_| Failure::bad_format())?
            .unwrap_or(Charset::Utf8);
        Ok(charset.decode(&bytes))
    }

    pub(crate) fn path_param(&self, name: &str) -> Result<String, Failure> {
        let raw = self
            .raw_params
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .ok_or_else(|| Failure::internal_msg(format!("'{name}' is not a valid path-param")))?;
        java::path_param_decode(raw).map_err(|_| Failure::bad_request(crate::reply::BAD_PARAM))
    }
}
