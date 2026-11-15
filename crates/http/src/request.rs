use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use bytes::{Bytes, BytesMut};
use http::{HeaderMap, StatusCode};
use http_body_util::BodyExt;

use crate::reply::{BAD_PARAM, Failure};

pub const DEFAULT_MAX_REQUEST_SIZE: usize = 1_000_000;

const DRAIN_LIMIT: usize = 1024 * 1024;
const DRAIN_TIME: Duration = Duration::from_secs(1);

pub(crate) struct Ctx {
    pub(crate) method: String,
    pub(crate) path: String,
    query: HashMap<String, String>,
    pub(crate) headers: HeaderMap,
    body: std::sync::Mutex<Option<Body>>,
    body_bytes: Option<Bytes>,
    pub(crate) client: Option<SocketAddr>,
    pub(crate) user_id: Option<i32>,
    pub(crate) params: Vec<(&'static str, String)>,
    pub(crate) max_body: usize,
    pub(crate) body_idle_timeout: Duration,
}

impl Ctx {
    pub(crate) fn new(request: http::Request<Body>, client: Option<SocketAddr>) -> Self {
        let (parts, body) = request.into_parts();
        Self {
            method: parts.method.as_str().to_owned(),
            path: parts.uri.path().to_owned(),
            query: parts.uri.query().map(parse_query).unwrap_or_default(),
            headers: parts.headers,
            body: std::sync::Mutex::new(Some(body)),
            body_bytes: None,
            client,
            user_id: None,
            params: Vec::new(),
            max_body: DEFAULT_MAX_REQUEST_SIZE,
            body_idle_timeout: Duration::from_secs(30),
        }
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    }

    pub(crate) fn query_param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(String::as_str)
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
                Ok(Some(Ok(frame))) => frame,
                Ok(None) => break,
                Ok(Some(Err(e))) => return Err(Failure::Aborted(e.to_string())),
                Err(_) => {
                    return Err(Failure::Aborted(
                        "idle timeout while reading the body".into(),
                    ));
                }
            };
            if let Ok(data) = frame.into_data() {
                if buf.len() + data.len() > self.max_body {
                    tracing::warn!("Body size greater than max size ({} bytes)", self.max_body);
                    drain(&mut body).await;
                    return Err(Failure::Plain(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "Content Too Large",
                    ));
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
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(crate) fn path_param(&self, name: &str) -> Result<String, Failure> {
        let raw = self
            .params
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .ok_or_else(|| Failure::internal_msg(format!("no path parameter {name}")))?;
        percent_decode(raw, false)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .ok_or_else(|| Failure::bad_request(BAD_PARAM))
    }
}

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

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let decode = |s| percent_decode(s, true).map(|b| String::from_utf8_lossy(&b).into_owned());
        if let (Some(key), Some(value)) = (decode(key), decode(value)) {
            params.entry(key).or_insert(value);
        }
    }
    params
}

pub(crate) fn percent_decode(s: &str, plus_is_space: bool) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = |b: Option<&u8>| b.and_then(|b| char::from(*b).to_digit(16));
                let (hi, lo) = (hex(bytes.get(i + 1))?, hex(bytes.get(i + 2))?);
                out.push(u8::try_from(hi * 16 + lo).ok()?);
                i += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_decoding() {
        let q = parse_query("a=1&b=x+y%20z&a=2&c&d=%zz&%E4%B8%AD=%E6%96%87");
        assert_eq!(q.get("a").map(String::as_str), Some("1"));
        assert_eq!(q.get("b").map(String::as_str), Some("x y z"));
        assert_eq!(q.get("c").map(String::as_str), Some(""));
        assert_eq!(q.get("d"), None);
        assert_eq!(q.get("中").map(String::as_str), Some("文"));
        assert_eq!(percent_decode("a+b%2B", false), Some(b"a+b+".to_vec()));
        assert_eq!(percent_decode("abc%2", false), None);
    }
}
