use std::sync::{Arc, Mutex};

use tracing::Level;
use tracing_subscriber::prelude::*;
use zphira_common::logging::{self, ConsoleLine, ConsoleSink, LogConfig};

#[derive(Default)]
struct Capture {
    lines: Mutex<Vec<ConsoleLine>>,
    ansi: bool,
}

impl ConsoleSink for Capture {
    fn write_line(&self, line: &ConsoleLine) {
        self.lines.lock().unwrap().push(line.clone());
    }

    fn supports_ansi(&self) -> bool {
        self.ansi
    }
}

fn strip_time(s: &str, open: &str) -> String {
    let rest = s
        .strip_prefix(open)
        .unwrap_or_else(|| panic!("{s:?} does not start with {open:?}"));
    let (time, rest) = rest.split_at(8);
    assert!(
        time.chars().enumerate().all(|(i, c)| if i == 2 || i == 5 {
            c == ':'
        } else {
            c.is_ascii_digit()
        }),
        "time {time:?}"
    );
    rest.to_owned()
}

#[test]
fn console_and_file_lines() {
    let dir = tempfile::tempdir().unwrap();
    let sink = Arc::new(Capture::default());
    let (layer, handle) = logging::layer(LogConfig {
        dir: Some(dir.path().to_path_buf()),
        console: sink.clone(),
        ..LogConfig::default()
    })
    .unwrap();
    let subscriber = tracing_subscriber::registry().with(layer);
    std::thread::Builder::new()
        .name("main".to_owned())
        .spawn(move || {
            tracing::subscriber::with_default(subscriber, || {
                tracing::info!(target: "zphira_game::room", "Room {} created by {}", "r1", 7);
                tracing::warn!(target: "zphira_common::args", "Value {} is below the minimum {}, using {}", 0, 1, 1);
                tracing::error!(target: "STDERR", "Exception in thread \"main\" x");
                tracing::info!(target: "STDOUT", "plain \u{1b}[31mred\u{1b}[0m text");
                tracing::debug!(target: "zphira_game", "not written at INFO");
                let io = std::io::Error::other("disk full");
                tracing::error!(target: "zphiramp", error = &io as &dyn std::error::Error, "Failed to save rooms");
                tracing::info!("from the test crate");
            });
        })
        .unwrap()
        .join()
        .unwrap();
    handle.flush();

    let console: Vec<(Level, String)> = sink
        .lines
        .lock()
        .unwrap()
        .iter()
        .map(|l| (l.level, strip_time(&l.text, "[")))
        .collect();
    assert_eq!(
        console,
        vec![
            (
                Level::INFO,
                " INFO] [Server]: Room r1 created by 7".to_owned()
            ),
            (
                Level::WARN,
                " WARN] [Server]: Value 0 is below the minimum 1, using 1".to_owned()
            ),
            (
                Level::ERROR,
                " ERROR] [STDERR]: Exception in thread \"main\" x".to_owned()
            ),
            (
                Level::INFO,
                " INFO] [STDOUT]: plain \u{1b}[31mred\u{1b}[0m text".to_owned()
            ),
            (
                Level::ERROR,
                " ERROR] [Server]: Failed to save rooms\ndisk full".to_owned()
            ),
            (
                Level::INFO,
                " INFO] [logging]: from the test crate".to_owned()
            ),
        ]
    );

    handle.shutdown();
    let file = std::fs::read_to_string(dir.path().join("latest.log")).unwrap();
    let lines: Vec<String> = file
        .lines()
        .map(|l| {
            if l.starts_with('[') {
                strip_time(l, "[")
            } else {
                l.to_owned()
            }
        })
        .collect();
    assert_eq!(
        lines,
        vec![
            "] [main/INFO] [Server]: Room r1 created by 7",
            "] [main/WARN] [Server]: Value 0 is below the minimum 1, using 1",
            "] [main/ERROR] [STDERR]: Exception in thread \"main\" x",
            "] [main/INFO] [STDOUT]: plain red text",
            "] [main/ERROR] [Server]: Failed to save rooms",
            "disk full",
            "] [main/INFO] [logging]: from the test crate",
        ]
    );
    assert!(file.ends_with('\n'));

    tracing::subscriber::with_default(tracing_subscriber::registry(), || tracing::info!("ignored"));
    handle.shutdown();
}

#[test]
fn ansi_rendering_matches_terminal_console_appender() {
    let error = ConsoleLine {
        level: Level::ERROR,
        text: "[12:00:00 ERROR] [Server]: boom".to_owned(),
    };
    assert_eq!(
        error.render(true),
        "\u{1b}[31;1m[12:00:00 ERROR] [Server]: boom\n\u{1b}[m"
    );
    assert_eq!(error.render(false), "[12:00:00 ERROR] [Server]: boom\n");
    let warn = ConsoleLine {
        level: Level::WARN,
        text: "[12:00:00 WARN] [Server]: w".to_owned(),
    };
    assert_eq!(
        warn.render(true),
        "\u{1b}[33;1m[12:00:00 WARN] [Server]: w\n\u{1b}[m"
    );
    let info = ConsoleLine {
        level: Level::INFO,
        text: "[12:00:00 INFO] [Server]: i".to_owned(),
    };
    assert_eq!(info.render(true), "[12:00:00 INFO] [Server]: i\n");
}

#[test]
fn sink_can_be_replaced() {
    let first = Arc::new(Capture::default());
    let second = Arc::new(Capture {
        ansi: true,
        ..Capture::default()
    });
    let (layer, handle) = logging::layer(LogConfig {
        dir: None,
        console: first.clone(),
        ..LogConfig::default()
    })
    .unwrap();
    let subscriber = Arc::new(tracing_subscriber::registry().with(layer));
    tracing::subscriber::with_default(
        subscriber.clone(),
        || tracing::info!(target: "zphira", "one"),
    );
    handle.set_console_sink(second.clone());
    tracing::subscriber::with_default(subscriber, || tracing::info!(target: "zphira", "two"));
    handle.shutdown();
    assert_eq!(first.lines.lock().unwrap().len(), 1);
    assert_eq!(second.lines.lock().unwrap().len(), 1);
    assert!(
        second.lines.lock().unwrap()[0]
            .text
            .ends_with("[Server]: two")
    );
}

#[test]
fn helpers() {
    assert_eq!(logging::logger_name("zphira_game::room"), "Server");
    assert_eq!(logging::logger_name("zphiramp"), "Server");
    assert_eq!(logging::logger_name("STDERR"), "STDERR");
    assert_eq!(logging::logger_name("hyper::proto"), "hyper::proto");
    assert_eq!(
        logging::strip_ansi("a\u{1b}[31;1mb\u{1b}[mc\u{1b}[x"),
        "abc\u{1b}[x"
    );
    assert_eq!(logging::strip_ansi("plain"), "plain");
}

#[test]
fn many_lines_from_many_threads_are_all_written() {
    let dir = tempfile::tempdir().unwrap();
    let (layer, handle) = logging::layer(LogConfig {
        dir: Some(dir.path().to_path_buf()),
        console: Arc::new(Capture::default()),
        queue_capacity: 16,
        ..LogConfig::default()
    })
    .unwrap();
    let subscriber = Arc::new(tracing_subscriber::registry().with(layer));
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let s = subscriber.clone();
            std::thread::spawn(move || {
                tracing::subscriber::with_default(s, || {
                    for i in 0..500 {
                        tracing::info!(target: "zphira", "t{t} line {i}");
                    }
                });
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    handle.shutdown();
    let file = std::fs::read_to_string(dir.path().join("latest.log")).unwrap();
    assert_eq!(file.lines().count(), 2000);
}
