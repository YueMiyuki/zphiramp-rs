#![allow(dead_code)]

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Once};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber, span};
use zphira_ledger::Clock;

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

pub fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                let cp = u32::from_str_radix(&hex, 16).expect("hex escape");
                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
            }
            other => panic!("bad escape {other:?} in {s:?}"),
        }
    }
    out
}

pub fn read_tsv(name: &str) -> Vec<Vec<String>> {
    read_fixture(name)
        .lines()
        .map(|l| l.split('\t').map(unescape).collect())
        .collect()
}

pub struct Script {
    rows: std::vec::IntoIter<Vec<String>>,
}

impl Script {
    pub fn new(name: &str) -> Self {
        Self {
            rows: read_tsv(name).into_iter(),
        }
    }

    pub fn expect(&mut self, label: &str) -> Vec<String> {
        let row = self
            .rows
            .next()
            .unwrap_or_else(|| panic!("fixture ended before {label}"));
        assert_eq!(row[0], label, "fixture rows out of step");
        row[1..].to_vec()
    }

    pub fn next_row(&mut self) -> Option<Vec<String>> {
        self.rows.next()
    }
}

#[derive(Clone)]
pub struct ManualClock(pub Arc<AtomicI64>);

impl ManualClock {
    pub fn new(start: i64) -> Self {
        Self(Arc::new(AtomicI64::new(start)))
    }

    pub fn clock(&self) -> Clock {
        let t = Arc::clone(&self.0);
        Arc::new(move || t.load(Ordering::SeqCst))
    }

    pub fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }

    pub fn set(&self, millis: i64) {
        self.0.store(millis, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub level: Level,
    pub message: String,
    pub error: Option<String>,
}

thread_local! {
    static CAPTURED: RefCell<Option<Vec<LogLine>>> = const { RefCell::new(None) };
}

struct LineVisitor {
    message: String,
    error: Option<String>,
}

impl Visit for LineVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let mut text = String::new();
        let _ = write!(text, "{value:?}");
        match field.name() {
            "message" => self.message = text,
            "error" => self.error = Some(text),
            _ => {}
        }
    }
}

// One global subscriber routes each event to the thread that logged it: `with_default` is
// unreliable when tests run in parallel.
struct Router;

impl Subscriber for Router {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut visitor = LineVisitor {
            message: String::new(),
            error: None,
        };
        event.record(&mut visitor);
        CAPTURED.with(|captured| {
            if let Some(lines) = captured.borrow_mut().as_mut() {
                lines.push(LogLine {
                    level: *event.metadata().level(),
                    message: visitor.message,
                    error: visitor.error,
                });
            }
        });
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

static INSTALL: Once = Once::new();

pub fn capture_logs<R>(f: impl FnOnce() -> R) -> (R, Vec<LogLine>) {
    INSTALL.call_once(|| {
        tracing::subscriber::set_global_default(Router).expect("no other global subscriber");
    });
    CAPTURED.with(|captured| *captured.borrow_mut() = Some(Vec::new()));
    let result = f();
    let lines = CAPTURED
        .with(|captured| captured.borrow_mut().take())
        .unwrap_or_default();
    (result, lines)
}
