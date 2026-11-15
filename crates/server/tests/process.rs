use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

mod common;

use common::{FakePhira, free_port};

const BIN: &str = env!("CARGO_BIN_EXE_zphiramp");
const WAIT: Duration = Duration::from_secs(30);

struct Running {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
    dir: tempfile::TempDir,
    game_port: u16,
    http_port: u16,
}

fn spawn(phira: &FakePhira, game_port: u16, http_port: u16, extra: &[&str]) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(BIN)
        .current_dir(dir.path())
        .env("ZPHIRA_PHIRA_HOST", phira.base())
        .env_remove("TERM")
        .args([
            "--host",
            "127.0.0.1",
            "--port",
            &game_port.to_string(),
            "--http-host",
            "127.0.0.1",
            "--http-port",
            &http_port.to_string(),
        ])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let stdout = child.stdout.take().unwrap();
    {
        let lines = lines.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                lines.lock().unwrap().push(line);
            }
        });
    }
    Running {
        child,
        lines,
        dir,
        game_port,
        http_port,
    }
}

fn start(phira: &FakePhira) -> Running {
    spawn(phira, free_port(), free_port(), &[])
}

impl Running {
    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.lines().iter().any(|l| l.contains(needle)) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "no line containing {needle:?} within {WAIT:?}; got:\n{}",
            self.lines().join("\n")
        );
    }

    fn count(&self, needle: &str) -> usize {
        self.lines().iter().filter(|l| l.contains(needle)).count()
    }

    fn send(&mut self, text: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(text.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    fn signal(&self, name: &str) {
        let status = Command::new("kill")
            .arg(format!("-{name}"))
            .arg(self.child.id().to_string())
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn wait_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                thread::sleep(Duration::from_millis(100));
                return status;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        panic!(
            "the server did not exit within {WAIT:?}; got:\n{}",
            self.lines().join("\n")
        );
    }

    fn log_file(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("logs").join("latest.log")).unwrap_or_default()
    }

    fn game_port_open(&self) -> bool {
        TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", self.game_port).parse().unwrap(),
            Duration::from_secs(1),
        )
        .is_ok()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn ordered(lines: &[String], expected: &[&str]) {
    let mut from = 0;
    for needle in expected {
        match lines[from..].iter().position(|l| l.contains(needle)) {
            Some(i) => from += i + 1,
            None => panic!("{needle:?} not found in order in:\n{}", lines.join("\n")),
        }
    }
}

#[test]
fn starts_in_java_order_takes_console_commands_and_stops_on_stop() {
    let phira = FakePhira::start();
    let mut server = start(&phira);
    server.wait_for("Server is running. Type 'stop' to stop.");
    ordered(
        &server.lines(),
        &[
            "[Server]: Booting up Phira Server...",
            "[Server]: Default language: zh-CN",
            "[Server]: Preloading chart pool...",
            "[Server]: Chart pool preloaded.",
            "[Server]: Chart index loaded: 0 charts",
            "[Server]: Round records loaded: 0",
            "[Server]: Initializing network...",
            "[Server]: Listening on 127.0.0.1:",
            "[Server]: Done (",
            "[Server]: Server is running. Type 'stop' to stop.",
        ],
    );
    let done = server
        .lines()
        .into_iter()
        .find(|l| l.contains("Done ("))
        .unwrap();
    let seconds = done
        .split("Done (")
        .nth(1)
        .unwrap()
        .split("s)!")
        .next()
        .unwrap();
    assert!(
        seconds.contains('.') && seconds.split('.').nth(1).unwrap().len() == 3,
        "{done}"
    );

    let paths = phira.paths();
    assert!(paths.iter().any(|p| p == "/up/chart/30474"), "{paths:?}");

    server.send("  \n room list \nfoo bar\nSTOP-not\n");
    server.wait_for("No rooms");
    server.wait_for("Unknown command: foo bar");
    server.wait_for("Unknown command: STOP-not");
    server.send("  Stop \n");
    let status = server.wait_exit();
    assert_eq!(status.code(), Some(0));
    ordered(
        &server.lines(),
        &[
            "[Server]: Shutting down...",
            "[Server]: Closing 1 channel(s)...",
            "[Server]: Channels closed.",
            "[Server]: Uptime: 0m ",
            "[Server]: Shutdown completed in ",
        ],
    );
    assert!(server.lines().last().unwrap().ends_with("ms. Goodbye!"));
    assert!(server.log_file().contains("ms. Goodbye!"));
    assert!(
        server
            .dir
            .path()
            .join("data")
            .join("chart-pools.json")
            .exists()
    );
}

#[test]
fn sigterm_and_sigint_run_the_same_graceful_stop_once() {
    for signal in ["TERM", "INT"] {
        let phira = FakePhira::start();
        let mut server = start(&phira);
        server.wait_for("Server is running.");
        server.signal(signal);
        server.signal(signal);
        server.signal("TERM");
        let status = server.wait_exit();
        assert_eq!(status.code(), Some(0), "SIG{signal}");
        assert_eq!(server.count("Shutting down..."), 1, "SIG{signal}");
        assert_eq!(server.count("Goodbye!"), 1, "SIG{signal}");
        assert!(server.log_file().contains("Goodbye!"));
        assert!(!server.game_port_open());
    }
}

#[test]
fn end_of_input_does_not_stop_the_server() {
    let phira = FakePhira::start();
    let mut server = start(&phira);
    server.wait_for("Server is running.");
    server.send("room list\n");
    server.wait_for("No rooms");
    drop(server.child.stdin.take());
    thread::sleep(Duration::from_millis(700));
    assert!(server.child.try_wait().unwrap().is_none());
    assert!(server.game_port_open());
    server.signal("TERM");
    assert_eq!(server.wait_exit().code(), Some(0));
}

#[test]
fn an_unterminated_last_line_runs_when_input_ends() {
    let phira = FakePhira::start();
    let mut server = start(&phira);
    server.wait_for("Server is running.");
    server.send("admin list");
    drop(server.child.stdin.take());
    server.wait_for("No admins");
    server.signal("TERM");
    assert_eq!(server.wait_exit().code(), Some(0));
}

#[test]
fn help_and_argument_errors_exit_with_javas_codes() {
    for (args, code, first) in [
        (vec!["--help"], 0, "[Server]: Phira Server Usage:"),
        (
            vec!["--bogus"],
            1,
            "ERROR] [Server]: Failed to parse arguments: bogus is not a recognized option",
        ),
        (
            vec!["--port", "abc"],
            1,
            "ERROR] [Server]: Failed to parse arguments: Cannot parse argument 'abc' of option port",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = Command::new(BIN)
            .current_dir(dir.path())
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{args:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        assert!(
            lines.iter().any(|l| l.contains(first)),
            "{args:?}: {stdout}"
        );
        if code == 1 {
            assert!(stdout.contains("[Server]: Phira Server Usage:"));
        }
        let file = std::fs::read_to_string(dir.path().join("logs").join("latest.log")).unwrap();
        assert!(file.contains("Phira Server Usage:"), "{args:?}");
        assert!(!dir.path().join("data").exists());
    }
}

#[test]
fn clamp_warnings_are_logged_before_booting() {
    let phira = FakePhira::start();
    let mut server = spawn(&phira, free_port(), free_port(), &["--max-rooms", "0"]);
    server.wait_for("Server is running.");
    ordered(
        &server.lines(),
        &[
            "WARN] [Server]: Value 0 is below the minimum 1, using 1",
            "Booting up Phira Server...",
        ],
    );
    server.send("stop\n");
    assert_eq!(server.wait_exit().code(), Some(0));
}

#[test]
fn a_busy_http_port_fails_the_start_and_leaves_no_game_port_open() {
    let phira = FakePhira::start();
    let blocker = TcpListener::bind("127.0.0.1:0").unwrap();
    let http_port = blocker.local_addr().unwrap().port();
    let mut server = spawn(&phira, free_port(), http_port, &[]);
    let status = server.wait_exit();
    assert_eq!(status.code(), Some(1));
    ordered(
        &server.lines(),
        &[
            "[Server]: Listening on 127.0.0.1:",
            "ERROR] [Server]: HTTP API server failed to bind",
            "ERROR] [Server]: Failed to start server",
        ],
    );
    assert_eq!(server.count("Server is running."), 0);
    assert!(!server.game_port_open());
    let lines = server.lines();
    let at = lines
        .iter()
        .position(|l| l.contains("Failed to start server"))
        .unwrap();
    assert!(
        lines[at + 1].contains("HTTP API server failed to bind"),
        "{lines:?}"
    );
    assert!(server.log_file().contains("Failed to start server"));
    drop(blocker);
}

#[test]
fn a_busy_game_port_fails_the_start() {
    let phira = FakePhira::start();
    let blocker = TcpListener::bind("127.0.0.1:0").unwrap();
    let game_port = blocker.local_addr().unwrap().port();
    let mut server = spawn(&phira, game_port, free_port(), &[]);
    let status = server.wait_exit();
    assert_eq!(status.code(), Some(1));
    ordered(
        &server.lines(),
        &[
            "Initializing network...",
            "ERROR] [Server]: Failed to start server",
        ],
    );
    assert!(
        server
            .lines()
            .iter()
            .any(|l| l.contains("Address already in use"))
    );
    assert_eq!(server.count("Done ("), 0);
    assert!(
        TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", server.http_port).parse().unwrap(),
            Duration::from_secs(1)
        )
        .is_err()
    );
    drop(blocker);
}

#[test]
fn an_unreachable_upstream_fails_the_start() {
    let dead = free_port();
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(BIN)
        .current_dir(dir.path())
        .env("ZPHIRA_PHIRA_HOST", format!("http://127.0.0.1:{dead}/"))
        .args(["--host", "127.0.0.1", "--port", &free_port().to_string()])
        .args([
            "--http-host",
            "127.0.0.1",
            "--http-port",
            &free_port().to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(1), "{out}");
    assert!(
        out.contains("ERROR] [Server]: Failed to start server"),
        "{out}"
    );
}

fn post_login(port: u16) -> u16 {
    let body = r#"{"email":"a@b.c","password":"pw"}"#;
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "POST /api/v1/login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse().ok())
        .unwrap_or_else(|| panic!("no status in {answer:?}"))
}

#[test]
fn the_login_rate_limit_flag_sets_the_attempts_per_minute() {
    let phira = FakePhira::start();
    let mut server = spawn(
        &phira,
        free_port(),
        free_port(),
        &["--login-rate-limit", "2"],
    );
    server.wait_for("Server is running.");
    let statuses: Vec<u16> = (0..3).map(|_| post_login(server.http_port)).collect();
    assert!(statuses[..2].iter().all(|s| *s != 429), "{statuses:?}");
    assert_eq!(statuses[2], 429, "{statuses:?}");
    server.send("stop\n");
    assert_eq!(server.wait_exit().code(), Some(0));
}

#[test]
fn minus_one_turns_the_login_limit_off() {
    let phira = FakePhira::start();
    let mut server = spawn(
        &phira,
        free_port(),
        free_port(),
        &["--login-rate-limit", "-1"],
    );
    server.wait_for("Server is running.");
    for attempt in 0..35 {
        assert_ne!(post_login(server.http_port), 429, "attempt {attempt}");
    }
    server.send("stop\n");
    assert_eq!(server.wait_exit().code(), Some(0));
}

#[test]
fn the_default_login_limit_is_thirty_per_minute() {
    let phira = FakePhira::start();
    let mut server = start(&phira);
    server.wait_for("Server is running.");
    let statuses: Vec<u16> = (0..31).map(|_| post_login(server.http_port)).collect();
    assert!(statuses[..30].iter().all(|s| *s != 429), "{statuses:?}");
    assert_eq!(statuses[30], 429);
    server.send("stop\n");
    assert_eq!(server.wait_exit().code(), Some(0));
}

#[test]
fn invalid_limit_flags_warn_and_keep_the_defaults() {
    let phira = FakePhira::start();
    let mut server = spawn(
        &phira,
        free_port(),
        free_port(),
        &["--max-pending-submissions", "0", "--login-rate-limit", "-7"],
    );
    server.wait_for("Server is running.");
    ordered(
        &server.lines(),
        &[
            "WARN] [Server]: Value 0 is not valid for --max-pending-submissions, using 50",
            "WARN] [Server]: Value -7 is not valid for --login-rate-limit, using 30",
            "Booting up Phira Server...",
        ],
    );
    server.send("stop\n");
    assert_eq!(server.wait_exit().code(), Some(0));
}
