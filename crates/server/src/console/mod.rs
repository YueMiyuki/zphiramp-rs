pub mod commands;
pub mod editor;
pub mod interactive;

use std::io::{self, BufRead};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::shutdown::{Cause, Shutdown};
use commands::{Commands, Flow, is_stop, normalize};
use interactive::Interactive;

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

fn read_lines(mut reader: impl BufRead, mut submit: impl FnMut(String)) {
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                while matches!(line.last(), Some(b'\n' | b'\r')) {
                    line.pop();
                }
                submit(String::from_utf8_lossy(&line).into_owned());
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

fn spawn_plain_reader(submit: impl Fn(String) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("Console".to_owned())
        .spawn(move || read_lines(io::stdin().lock(), submit));
    if let Err(e) = spawned {
        tracing::error!("Failed to read console input: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(input: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        read_lines(io::Cursor::new(input.to_vec()), |l| lines.push(l));
        lines
    }

    #[test]
    fn lines_end_at_line_feed_or_carriage_return_and_line_feed() {
        assert_eq!(
            lines_of(b"room list\nsay hi\r\nadmin list\n\nx\n"),
            ["room list", "say hi", "admin list", "", "x"]
        );
    }

    #[test]
    fn the_last_unterminated_line_counts() {
        assert_eq!(lines_of(b"chart index"), ["chart index"]);
        assert_eq!(lines_of(b"a\nb"), ["a", "b"]);
        assert!(lines_of(b"").is_empty());
    }

    #[test]
    fn invalid_utf8_is_replaced() {
        assert_eq!(lines_of(b"say \xff\n"), ["say \u{fffd}"]);
        assert_eq!(lines_of("say 你好\n".as_bytes()), ["say 你好"]);
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
