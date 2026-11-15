mod console;
mod rolling;

use std::borrow::Cow;
use std::fmt::{self, Write as _};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

pub use console::{ConsoleLine, ConsoleSink, StdoutSink};

use rolling::RollingFile;

const WRITER_THREAD: &str = "zphira-log-writer";

pub struct LogConfig {
    pub dir: Option<PathBuf>,
    pub max_level: Level,
    pub console: Arc<dyn ConsoleSink>,
    pub started: SystemTime,
    pub queue_capacity: usize,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            dir: Some(PathBuf::from("logs")),
            max_level: Level::INFO,
            console: Arc::new(StdoutSink::new()),
            started: SystemTime::now(),
            queue_capacity: 65_536,
        }
    }
}

pub fn logger_name(target: &str) -> &str {
    if target.starts_with("zphira") {
        "Server"
    } else {
        target
    }
}

pub fn strip_ansi(s: &str) -> Cow<'_, str> {
    if !s.contains('\u{1b}') {
        return Cow::Borrowed(s);
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < bytes.len() && (bytes[j] == b';' || bytes[j].is_ascii_digit()) {
                j += 1;
            }
            if bytes.get(j) == Some(&b'm') {
                out.push_str(&s[last..i]);
                i = j + 1;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&s[last..]);
    Cow::Owned(out)
}

pub fn init(config: LogConfig) -> io::Result<LogHandle> {
    let (layer, handle) = layer(config)?;
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer)).map_err(
        |_| {
            handle.shutdown();
            io::Error::other("a global tracing subscriber is already installed")
        },
    )?;
    Ok(handle)
}

pub fn layer(config: LogConfig) -> io::Result<(LogLayer, LogHandle)> {
    let file = match &config.dir {
        Some(dir) => Some(RollingFile::open(dir, config.started, Local::now())?),
        None => None,
    };
    let (tx, rx) = sync_channel::<Msg>(config.queue_capacity.max(1));
    let console = config.console;
    let thread = std::thread::Builder::new()
        .name(WRITER_THREAD.to_owned())
        .spawn(move || writer_loop(rx, console, file))?;
    let handle = LogHandle {
        inner: Arc::new(HandleInner {
            tx: tx.clone(),
            thread: Mutex::new(Some(thread)),
        }),
    };
    Ok((
        LogLayer {
            tx,
            max_level: config.max_level,
        },
        handle,
    ))
}

/// Call `flush` or `shutdown` before the process exits, or queued lines are lost.
#[derive(Clone)]
#[must_use = "keep the handle to flush and shut down logging before exit"]
pub struct LogHandle {
    inner: Arc<HandleInner>,
}

struct HandleInner {
    tx: SyncSender<Msg>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LogHandle {
    pub fn flush(&self) {
        let (ack_tx, ack_rx) = sync_channel(1);
        if self.inner.tx.send(Msg::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv();
        }
    }

    pub fn shutdown(&self) {
        let (ack_tx, ack_rx) = sync_channel(1);
        if self.inner.tx.send(Msg::Shutdown(ack_tx)).is_ok() {
            let _ = ack_rx.recv();
        }
        let thread = match self.inner.thread.lock() {
            Ok(mut t) => t.take(),
            Err(p) => p.into_inner().take(),
        };
        if let Some(t) = thread {
            let _ = t.join();
        }
    }

    pub fn set_console_sink(&self, sink: Arc<dyn ConsoleSink>) {
        let _ = self.inner.tx.send(Msg::SetSink(sink));
    }
}

impl fmt::Debug for LogHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LogHandle")
    }
}

enum Msg {
    Line(Box<Line>),
    Flush(SyncSender<()>),
    SetSink(Arc<dyn ConsoleSink>),
    Shutdown(SyncSender<()>),
}

struct Line {
    date: NaiveDate,
    console: ConsoleLine,
    file: String,
}

pub struct LogLayer {
    tx: SyncSender<Msg>,
    max_level: Level,
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() > self.max_level {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let line = format_line(Local::now(), *meta.level(), meta.target(), &fields);
        let msg = Msg::Line(Box::new(line));
        match self.tx.try_send(msg) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(msg)) => {
                // The writer thread must never wait on its own queue.
                if std::thread::current().name() != Some(WRITER_THREAD) {
                    let _ = self.tx.send(msg);
                }
            }
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::from_level(self.max_level))
    }
}

fn format_line(now: DateTime<Local>, level: Level, target: &str, fields: &Fields) -> Line {
    let time = now.format("%H:%M:%S");
    let logger = logger_name(target);
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let mut console = format!("[{time} {level}] [{logger}]: {}", fields.message);
    let mut file = format!(
        "[{time}] [{thread_name}/{level}] [{logger}]: {}\n",
        strip_ansi(&fields.message)
    );
    if let Some(error) = &fields.error {
        console.push('\n');
        console.push_str(error);
        file.push_str(error);
        file.push('\n');
    }
    Line {
        date: now.date_naive(),
        console: ConsoleLine {
            level,
            text: console,
        },
        file,
    }
}

#[derive(Default)]
struct Fields {
    message: String,
    error: Option<String>,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "message" => {
                let _ = write!(self.message, "{value:?}");
            }
            "error" => self.error = Some(format!("{value:?}")),
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message.push_str(value),
            "error" => self.error = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        if field.name() == "error" {
            let mut chain = value.to_string();
            let mut source = value.source();
            while let Some(s) = source {
                chain.push_str(": ");
                chain.push_str(&s.to_string());
                source = s.source();
            }
            self.error = Some(chain);
        } else {
            self.record_debug(field, &format_args!("{value}"));
        }
    }
}

fn writer_loop(
    rx: Receiver<Msg>,
    mut console: Arc<dyn ConsoleSink>,
    mut file: Option<RollingFile>,
) {
    let mut reported = false;
    let mut report = |e: io::Error| {
        if !reported {
            reported = true;
            eprintln!("zphira logging: cannot write the log file: {e}");
        }
    };
    loop {
        let Ok(first) = rx.recv() else {
            break;
        };
        let mut next = Some(first);
        let mut shutdown_ack = None;
        while let Some(msg) = next {
            match msg {
                Msg::Line(line) => {
                    console.write_line(&line.console);
                    if let Some(f) = file.as_mut()
                        && let Err(e) = f.write(line.date, &line.file)
                    {
                        report(e);
                    }
                }
                Msg::SetSink(sink) => {
                    console.flush();
                    console = sink;
                }
                Msg::Flush(ack) => {
                    console.flush();
                    if let Some(f) = file.as_mut()
                        && let Err(e) = f.flush()
                    {
                        report(e);
                    }
                    let _ = ack.send(());
                }
                Msg::Shutdown(ack) => {
                    shutdown_ack = Some(ack);
                    break;
                }
            }
            next = rx.try_recv().ok();
        }
        console.flush();
        if let Some(f) = file.as_mut()
            && let Err(e) = f.flush()
        {
            report(e);
        }
        if let Some(ack) = shutdown_ack {
            if let Some(mut f) = file.take()
                && let Err(e) = f.close()
            {
                report(e);
            }
            let _ = ack.send(());
            return;
        }
    }
    if let Some(mut f) = file.take() {
        let _ = f.close();
    }
}
