#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use axum::body::Body;
use base64::Engine;
use http::Request;
use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

use super::{Env, Reply, fixtures};

pub struct Exchange {
    pub name: String,
    pub request: Value,
    pub response: Value,
    pub log: Vec<String>,
}

pub fn load(scenario: &str) -> Vec<Exchange> {
    let text = std::fs::read_to_string(fixtures().join(format!("{scenario}.json"))).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    doc["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| Exchange {
            name: e["name"].as_str().unwrap().to_owned(),
            request: e["request"].clone(),
            response: e["response"].clone(),
            log: e["log"]
                .as_array()
                .map(|l| l.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
                .unwrap_or_default(),
        })
        .collect()
}

pub fn build_request(env: &Env, request: &Value) -> Option<Request<Body>> {
    let method = request["method"].as_str().unwrap();
    let target = env.resolve(request["target"].as_str().unwrap());
    let mut builder = Request::builder().method(method).uri(target.as_str());
    for pair in request["headers"].as_array().unwrap() {
        let name = pair[0].as_str().unwrap();
        let value = env.resolve(pair[1].as_str().unwrap());
        builder = builder.header(name, value);
    }
    let body: Vec<u8> = if let Some(text) = request["body"].as_str() {
        env.resolve(text).into_bytes()
    } else if let Some(b64) = request["bodyBase64"].as_str() {
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap()
    } else if let Some(fill) = request.get("fill") {
        let prefix = fill["prefix"].as_str().unwrap().as_bytes();
        let mut bytes = vec![b' '; usize::try_from(fill["size"].as_u64().unwrap()).unwrap()];
        bytes[..prefix.len()].copy_from_slice(prefix);
        bytes
    } else {
        Vec::new()
    };
    builder.body(Body::from(body)).ok()
}

pub fn header<'a>(response: &'a Value, name: &str) -> Option<&'a str> {
    response["headers"].as_array().and_then(|headers| {
        headers
            .iter()
            .find(|h| h[0].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
            .and_then(|h| h[1].as_str())
    })
}

pub fn normalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                match key.as_str() {
                    "token" => *v = Value::String("<token>".into()),
                    "backup" if v.as_str().is_some_and(|s| !s.is_empty()) => {
                        *v = Value::String("<backup>".into())
                    }
                    _ => normalize(v),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize),
        Value::String(s) => {
            if s.len() >= 20
                && s.as_bytes()[4] == b'-'
                && s.as_bytes()[10] == b'T'
                && s[..4] >= *"2026"
            {
                *s = "<now>".into();
            } else if let Ok(instant) = zphira_common::time::Timestamp::parse(s) {
                *s = instant.to_string();
            }
            if let Some(dash) = s.find('-')
                && s.ends_with(".json")
                && s[dash + 1..].len() == "20261115-011815-585.json".len()
                && s.as_bytes()[dash + 1 + 8] == b'-'
            {
                *s = format!("{}-<stamp>.json", &s[..dash]);
            }
        }
        _ => {}
    }
}

fn ordered_eq(a: &Value, b: &Value, path: &str, top: bool, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let unordered =
                top || x.contains_key("playerId") && x.contains_key("rounds") && x.len() == 2;
            let xk: Vec<&String> = x.keys().collect();
            let yk: Vec<&String> = y.keys().collect();
            if unordered {
                let mut xs = xk.clone();
                let mut ys = yk.clone();
                xs.sort();
                ys.sort();
                if xs != ys {
                    out.push(format!("{path}: keys {xk:?} vs {yk:?}"));
                    return;
                }
            } else if xk != yk {
                out.push(format!("{path}: key order {xk:?} vs {yk:?}"));
                return;
            }
            for (k, v) in x {
                ordered_eq(v, &y[k], &format!("{path}.{k}"), false, out);
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{path}: {} items vs {}", x.len(), y.len()));
                return;
            }
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                ordered_eq(v, w, &format!("{path}[{i}]"), false, out);
            }
        }
        (Value::Number(x), Value::Number(y)) => {
            if x.to_string() != y.to_string() {
                out.push(format!("{path}: {x} vs {y}"));
            }
        }
        _ => {
            if a != b {
                out.push(format!("{path}: {a} vs {b}"));
            }
        }
    }
}

pub fn compare(method: &str, java: &Value, rust: &Reply) -> Vec<String> {
    compare_patched(method, java, rust, &|_| {})
}

pub fn compare_patched(
    method: &str,
    java: &Value,
    rust: &Reply,
    patch: &dyn Fn(&mut Value),
) -> Vec<String> {
    let mut out = Vec::new();
    let status = java["status"].as_u64().unwrap();
    if u64::from(rust.status.as_u16()) != status {
        out.push(format!("status {status} vs {}", rust.status.as_u16()));
    }
    for name in [
        "Content-Type",
        "Cache-Control",
        "Content-Disposition",
        "Access-Control-Allow-Origin",
        "Access-Control-Allow-Methods",
        "Access-Control-Allow-Headers",
    ] {
        let j = header(java, name);
        let r = rust.header(name);
        if j != r {
            out.push(format!("header {name}: {j:?} vs {r:?}"));
        }
    }
    let gzipped = java["gzipped"].as_bool().unwrap_or(false);
    let rust_gzipped = rust.header("content-encoding") == Some("gzip");
    if gzipped != rust_gzipped {
        out.push(format!("gzip {gzipped} vs {rust_gzipped}"));
    }
    if method == "HEAD" {
        return out;
    }
    let java_body: Vec<u8> = match (java["body"].as_str(), java["bodyBase64"].as_str()) {
        (Some(text), _) => text.as_bytes().to_vec(),
        (None, Some(b64)) => base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap(),
        _ => Vec::new(),
    };
    let json_type = header(java, "Content-Type") == Some("application/json");
    let java_json: Option<Value> = if json_type {
        serde_json::from_slice(&java_body).ok()
    } else {
        None
    };
    match java_json {
        Some(mut j) => match serde_json::from_slice::<Value>(&rust.body) {
            Ok(mut r) => {
                normalize(&mut j);
                patch(&mut j);
                normalize(&mut r);
                let top = header(java, "Content-Disposition").is_none();
                let mut diffs = Vec::new();
                ordered_eq(&j, &r, "$", top, &mut diffs);
                out.extend(diffs.into_iter().take(5).map(|d| format!("body {d}")));
            }
            Err(_) => out.push(format!("body not JSON: {}", rust.text())),
        },
        None => {
            if java_body != rust.body {
                out.push(format!(
                    "body {:?} vs {:?}",
                    String::from_utf8_lossy(&java_body)
                        .chars()
                        .take(200)
                        .collect::<String>(),
                    rust.text().chars().take(200).collect::<String>()
                ));
            }
        }
    }
    out
}

#[derive(Clone, Default)]
pub struct Captured(pub Arc<Mutex<Vec<String>>>);

impl Captured {
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

thread_local! {
    static CURRENT: std::cell::RefCell<Option<Captured>> = const { std::cell::RefCell::new(None) };
}

// One global subscriber routing events by thread: per-thread default subscribers race with
// the callsite interest cache of `tracing` when tests run in parallel.
struct Capture;

#[derive(Default)]
struct Fields {
    message: String,
    error: Option<String>,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "error" => self.error = Some(format!("{value:?}")),
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_owned(),
            "error" => self.error = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        if field.name() == "error" {
            self.error = Some(value.to_string());
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Capture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        *metadata.level() <= tracing::Level::INFO
    }

    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let everywhere = EVERYWHERE.lock().unwrap().clone();
        let here = CURRENT.with(|current| current.borrow().clone());
        if here.is_none() && everywhere.is_empty() {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let meta = event.metadata();
        let logger = zphira_common::logging::logger_name(meta.target());
        let mut line = format!("{} {}: {}", meta.level(), logger, fields.message);
        if let Some(error) = fields.error {
            line.push_str("\n  ");
            line.push_str(&error);
        }
        for captured in here.iter().chain(&everywhere) {
            captured.0.lock().unwrap().push(line.clone());
        }
    }
}

static EVERYWHERE: Mutex<Vec<Captured>> = Mutex::new(Vec::new());

pub struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| *current.borrow_mut() = None);
    }
}

pub fn capture() -> (Captured, CaptureGuard) {
    install();
    let captured = Captured::default();
    CURRENT.with(|current| *current.borrow_mut() = Some(captured.clone()));
    (captured, CaptureGuard)
}

pub fn relative_logs(lines: &[String], data: &std::path::Path) -> Vec<String> {
    let prefix = data.display().to_string();
    lines.iter().map(|l| l.replace(&prefix, "data")).collect()
}

pub fn log_messages(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = lines
        .iter()
        .map(|l| l.lines().next().unwrap_or("").to_owned())
        .filter(|l| l.contains(" Server: ") || l.contains(" io.javalin.Javalin: "))
        .map(|l| mask_stamps(&l))
        .collect();
    out.sort();
    out
}

fn mask_stamps(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(pos) = rest.find(".json") {
        let (head, tail) = rest.split_at(pos);
        let stamp_len = "20261115-011815-585".len();
        if head.len() > stamp_len
            && head.as_bytes()[head.len() - stamp_len - 1] == b'-'
            && head[head.len() - stamp_len..]
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-')
        {
            out.push_str(&head[..head.len() - stamp_len]);
            out.push_str("<stamp>");
        } else {
            out.push_str(head);
        }
        out.push_str(".json");
        rest = &tail[5..];
    }
    out.push_str(rest);
    out
}

fn install() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let _ =
            tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Capture));
    });
}

pub struct EverywhereGuard(Captured);

impl Drop for EverywhereGuard {
    fn drop(&mut self) {
        EVERYWHERE
            .lock()
            .unwrap()
            .retain(|c| !Arc::ptr_eq(&c.0, &(self.0).0));
    }
}

pub fn capture_everywhere() -> (Captured, EverywhereGuard) {
    install();
    let captured = Captured::default();
    EVERYWHERE.lock().unwrap().push(captured.clone());
    (captured.clone(), EverywhereGuard(captured))
}
