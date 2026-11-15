#![cfg(unix)]

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use common::{FakePhira, free_port};
use nix::sys::termios::LocalFlags;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const BIN: &str = env!("CARGO_BIN_EXE_zphiramp");
const WAIT: Duration = Duration::from_secs(30);

struct Terminal {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Arc<Mutex<Vec<u8>>>,
    game_port: u16,
    _dir: tempfile::TempDir,
    _phira: FakePhira,
}

fn start() -> Terminal {
    let phira = FakePhira::start();
    let dir = tempfile::tempdir().unwrap();
    let game_port = free_port();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(BIN);
    cmd.cwd(dir.path());
    cmd.env("TERM", "xterm-256color");
    cmd.env("ZPHIRA_PHIRA_HOST", phira.base());
    for arg in [
        "--host",
        "127.0.0.1",
        "--port",
        &game_port.to_string(),
        "--http-host",
        "127.0.0.1",
        "--http-port",
        &free_port().to_string(),
    ] {
        cmd.arg(arg);
    }
    let child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut reader = pair.master.try_clone_reader().unwrap();
    {
        let output = output.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                output.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
    }
    let writer = pair.master.take_writer().unwrap();
    Terminal {
        master: pair.master,
        writer,
        child,
        output,
        game_port,
        _dir: dir,
        _phira: phira,
    }
}

impl Terminal {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    fn wait_for(&self, needle: &str, from: usize) -> usize {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            let text = self.text();
            if let Some(at) = text[from.min(text.len())..].find(needle) {
                return from + at + needle.len();
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "no {needle:?} after byte {from} within {WAIT:?}; output:\n{:?}",
            self.text()
        );
    }

    fn send(&mut self, bytes: &str) {
        self.writer.write_all(bytes.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    fn wait_exit(&mut self) -> u32 {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.exit_code();
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        panic!("no exit within {WAIT:?}; output:\n{:?}", self.text());
    }

    fn assert_cooked(&self) {
        let termios = self.master.get_termios().expect("termios");
        assert!(
            termios.local_flags.contains(LocalFlags::ICANON),
            "{termios:?}"
        );
        assert!(
            termios.local_flags.contains(LocalFlags::ECHO),
            "{termios:?}"
        );
    }

    fn pid(&self) -> u32 {
        self.child.process_id().unwrap()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn prompt_editing_history_log_lines_above_the_prompt_and_ctrl_c() {
    let mut t = start();
    let at = t.wait_for("Server is running. Type 'stop' to stop.\r\n> ", 0);
    let raw = t.master.get_termios().expect("termios");
    assert!(!raw.local_flags.contains(LocalFlags::ICANON));

    t.send("room li");
    let at = t.wait_for("> room li", at);
    let client = TcpStream::connect(("127.0.0.1", t.game_port)).unwrap();
    let at = t.wait_for("Establishing a connection from", at);
    let before = t.text()[..at].to_owned();
    assert!(
        before.rfind("\r\x1b[J[").unwrap() > before.find("> room li").unwrap(),
        "{before:?}"
    );
    let at = t.wait_for("\r\n> room li", at);
    drop(client);

    t.send("st\r");
    let at = t.wait_for("No rooms", at);
    let at = t.wait_for("> ", at);
    t.send("\x1b[A\r");
    t.wait_for("No rooms", at);
    let at = t.text().len();
    t.send("\x04");
    t.wait_for("\r\n> ", at);
    assert!(t.child.try_wait().unwrap().is_none());
    let at = t.text().len();
    t.send("bogus cmd\r");
    t.wait_for("\x1b[33;1m[", at);
    t.wait_for("Unknown command: bogus cmd", at);

    t.send("abc");
    let at = t.wait_for("> abc", at);
    t.send("\x03");
    t.wait_for("Shutting down...", at);
    t.wait_for("Goodbye!", at);
    assert_eq!(t.wait_exit(), 0);
    t.assert_cooked();
    assert!(
        t.text().contains("\x1b[?2004l"),
        "bracketed paste switched off"
    );
}

#[test]
fn stop_typed_on_the_terminal_stops_and_restores_it() {
    let mut t = start();
    let at = t.wait_for("Server is running. Type 'stop' to stop.\r\n> ", 0);
    t.send("stop\r");
    t.wait_for("Goodbye!", at);
    assert_eq!(t.wait_exit(), 0);
    t.assert_cooked();
}

#[test]
fn sigterm_on_a_terminal_stops_and_restores_it() {
    let mut t = start();
    let at = t.wait_for("Server is running. Type 'stop' to stop.\r\n> ", 0);
    t.send("half a line");
    t.wait_for("> half a line", at);
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(t.pid().to_string())
        .status()
        .unwrap();
    assert!(status.success());
    t.wait_for("Goodbye!", at);
    assert_eq!(t.wait_exit(), 0);
    t.assert_cooked();
}
