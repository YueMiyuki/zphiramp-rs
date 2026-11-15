use std::io::{self, IsTerminal, Write};

use tracing::Level;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleLine {
    pub level: Level,
    pub text: String,
}

const ANSI_ERROR: &str = "\u{1b}[31;1m";
const ANSI_WARN: &str = "\u{1b}[33;1m";
const ANSI_RESET: &str = "\u{1b}[m";

impl ConsoleLine {
    pub fn render(&self, ansi: bool) -> String {
        let style = match self.level {
            Level::ERROR if ansi => Some(ANSI_ERROR),
            Level::WARN if ansi => Some(ANSI_WARN),
            _ => None,
        };
        match style {
            // The colour reset comes after the newline.
            Some(s) => format!("{s}{}\n{ANSI_RESET}", self.text),
            None => format!("{}\n", self.text),
        }
    }
}

/// Called on the logging thread, in order; an implementation must not log through `tracing`.
pub trait ConsoleSink: Send + Sync + 'static {
    fn write_line(&self, line: &ConsoleLine);

    fn supports_ansi(&self) -> bool;

    fn flush(&self) {}
}

#[derive(Debug)]
pub struct StdoutSink {
    ansi: bool,
}

impl StdoutSink {
    pub fn new() -> Self {
        let term_ok = std::env::var("TERM")
            .map(|t| t != "dumb")
            .unwrap_or(cfg!(windows));
        Self {
            ansi: io::stdout().is_terminal() && term_ok,
        }
    }
}

impl Default for StdoutSink {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleSink for StdoutSink {
    fn write_line(&self, line: &ConsoleLine) {
        let _ = io::stdout()
            .lock()
            .write_all(line.render(self.ansi).as_bytes());
    }

    fn supports_ansi(&self) -> bool {
        self.ansi
    }

    fn flush(&self) {
        let _ = io::stdout().lock().flush();
    }
}
