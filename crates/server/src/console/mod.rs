pub mod commands;
pub mod editor;
pub mod interactive;

use std::io::{self, Read};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::shutdown::{Cause, Shutdown};
use commands::{Commands, Flow, is_stop, normalize};
use interactive::Interactive;

#[derive(Debug, Default)]
pub struct LineSplitter {
    line: Vec<u8>,
    skip_lf: bool,
}

impl LineSplitter {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &b in bytes {
            if self.skip_lf {
                self.skip_lf = false;
                if b == b'\n' {
                    continue;
                }
            }
            match b {
                b'\n' => lines.push(self.take()),
                b'\r' => {
                    lines.push(self.take());
                    self.skip_lf = true;
                }
                _ => self.line.push(b),
            }
        }
        lines
    }

    pub fn finish(&mut self) -> Option<String> {
        (!self.line.is_empty()).then(|| self.take())
    }

    fn take(&mut self) -> String {
        let line = String::from_utf8_lossy(&self.line).into_owned();
        self.line.clear();
        line
    }
}

pub struct Console {
    queue: mpsc::UnboundedSender<String>,
    shutdown: Shutdown,
}

impl Console {
    pub fn start(
        commands: Commands,
        terminal: Option<Arc<Interactive>>,
        shutdown: Shutdown,
    ) -> Console {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(worker(commands, rx, shutdown.clone()));
        let console = Console {
            queue: tx,
            shutdown,
        };
        let submitter = console.submitter();
        let interactive_started = terminal.as_ref().is_some_and(|terminal| {
            let interrupt = {
                let shutdown = console.shutdown.clone();
                move || {
                    shutdown.request(Cause::Console);
                }
            };
            terminal.start(submitter.clone(), interrupt).is_ok()
        });
        if !interactive_started {
            spawn_plain_reader(submitter);
        }
        console
    }

    fn submitter(&self) -> impl Fn(String) + Clone + Send + 'static {
        let queue = self.queue.clone();
        let shutdown = self.shutdown.clone();
        move |line: String| submit(&queue, &shutdown, &line)
    }
}

// `stop` acts at once instead of waiting behind a running command.
fn submit(queue: &mpsc::UnboundedSender<String>, shutdown: &Shutdown, line: &str) {
    let Some(command) = normalize(line) else {
        return;
    };
    if is_stop(command) {
        shutdown.request(Cause::Console);
    } else {
        let _ = queue.send(command.to_owned());
    }
}

async fn worker(commands: Commands, mut rx: mpsc::UnboundedReceiver<String>, shutdown: Shutdown) {
    while let Some(line) = rx.recv().await {
        if shutdown.is_requested() {
            // Commands still queued when the stop is requested are dropped.
            break;
        }
        let commands = commands.clone();
        match tokio::spawn(async move { commands.run(&line).await }).await {
            Ok(Flow::Continue) => {}
            Ok(Flow::Stop) => {
                shutdown.request(Cause::Console);
            }
            Err(e) => tracing::error!("Console command failed unexpectedly: {}", e),
        }
    }
}

fn spawn_plain_reader(submit: impl Fn(String) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("Console".to_owned())
        .spawn(move || {
            let mut stdin = io::stdin().lock();
            let mut splitter = LineSplitter::default();
            let mut buf = [0u8; 4096];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        for line in splitter.feed(&buf[..n]) {
                            submit(line);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            if let Some(line) = splitter.finish() {
                submit(line);
            }
        });
    if let Err(e) = spawned {
        tracing::error!("Failed to read console input: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_buffered_reader() {
        let mut s = LineSplitter::default();
        assert_eq!(
            s.feed(b"room list\nsay hi\r\nadmin"),
            ["room list", "say hi"]
        );
        assert_eq!(s.feed(b" list\rstop\r"), ["admin list", "stop"]);
        assert_eq!(s.feed(b"\nx\n\n"), ["x", ""]);
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn the_last_unterminated_line_counts() {
        let mut s = LineSplitter::default();
        assert!(s.feed(b"chart index").is_empty());
        assert_eq!(s.finish().as_deref(), Some("chart index"));
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn invalid_utf8_is_replaced() {
        let mut s = LineSplitter::default();
        assert_eq!(s.feed(b"say \xff\n"), ["say \u{fffd}"]);
        assert_eq!(s.feed("say 你好\n".as_bytes()), ["say 你好"]);
    }

    #[test]
    fn submit_trims_skips_blanks_and_stops_at_once() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let shutdown = Shutdown::new();
        submit(&tx, &shutdown, "   ");
        submit(&tx, &shutdown, "  room list \t");
        assert_eq!(rx.try_recv().unwrap(), "room list");
        assert!(rx.try_recv().is_err());
        assert!(!shutdown.is_requested());
        submit(&tx, &shutdown, "  STOP ");
        assert!(shutdown.is_requested());
        assert!(rx.try_recv().is_err());
        let shutdown = Shutdown::new();
        submit(&tx, &shutdown, "Stop-not");
        assert!(!shutdown.is_requested());
        assert_eq!(rx.try_recv().unwrap(), "Stop-not");
    }
}
