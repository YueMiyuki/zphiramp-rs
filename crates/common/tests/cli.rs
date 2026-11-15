use zphira_common::cli::{self, CliOutcome, ServerArgs};

fn run(argv: &[&str]) -> (ServerArgs, Vec<String>) {
    match cli::parse(argv) {
        CliOutcome::Run { args, warnings } => (args, warnings),
        other => panic!("{argv:?}: {other:?}"),
    }
}

fn is_error(argv: &[&str]) -> bool {
    matches!(cli::parse(argv), CliOutcome::Error(_))
}

#[test]
fn defaults() {
    assert_eq!(run(&[]), (ServerArgs::default(), vec![]));
    let d = ServerArgs::default();
    assert_eq!(
        (
            d.port,
            d.http_port,
            d.host.as_str(),
            d.default_language.as_str()
        ),
        (12346, 8080, "0.0.0.0", "zh-CN")
    );
    assert_eq!(
        (
            d.read_timeout_seconds,
            d.max_connections,
            d.max_rooms,
            d.empty_room_ttl_minutes
        ),
        (3600, 1000, 200, 60)
    );
    assert_eq!(
        (d.max_pending_submissions, d.login_rate_limit),
        (Some(50), Some(30))
    );
    assert_eq!(d.plugins_dir, std::path::PathBuf::from("plugins"));
}

#[test]
fn every_flag_in_both_forms() {
    let argv = [
        "--port",
        "1",
        "--http-port",
        "2",
        "--host",
        "h",
        "--http-host",
        "hh",
        "--plugins",
        "pp",
        "--language",
        "fr",
        "--read-timeout",
        "60",
        "--max-connections",
        "10",
        "--max-rooms",
        "3",
        "--empty-room-ttl",
        "7",
        "--max-pending-submissions",
        "9",
        "--login-rate-limit",
        "8",
        "--proxy-protocol",
        "true",
    ];
    let expected = ServerArgs {
        port: 1,
        host: "h".to_owned(),
        http_host: "hh".to_owned(),
        http_port: 2,
        plugins_dir: "pp".into(),
        proxy_protocol: true,
        default_language: "fr".to_owned(),
        read_timeout_seconds: 60,
        max_connections: 10,
        max_rooms: 3,
        empty_room_ttl_minutes: 7,
        max_pending_submissions: Some(9),
        login_rate_limit: Some(8),
    };
    assert_eq!(run(&argv).0, expected);
    let joined: Vec<String> = argv
        .chunks(2)
        .map(|p| format!("{}={}", p[0], p[1]))
        .collect();
    let joined: Vec<&str> = joined.iter().map(String::as_str).collect();
    assert_eq!(run(&joined).0, expected);
}

#[test]
fn proxy_protocol_forms() {
    let proxy = |argv: &[&str]| run(argv).0.proxy_protocol;
    assert!(proxy(&["--proxy-protocol"]));
    assert!(proxy(&["--proxy-protocol", "--port", "5"]));
    assert!(proxy(&["--proxy-protocol=TRUE"]));
    assert!(!proxy(&["--proxy-protocol", "false"]));
    assert!(!proxy(&["--proxy-protocol=false"]));
    assert!(!proxy(&[]));
}

#[test]
fn clamping_and_warnings() {
    assert_eq!(run(&["--port", "0", "--http-port", "70000"]).0.port, 1);
    assert_eq!(run(&["--http-port", "70000"]).0.http_port, 65535);
    let (args, warnings) = run(&[
        "--read-timeout",
        "4",
        "--max-connections",
        "0",
        "--max-rooms",
        "-5",
        "--empty-room-ttl",
        "-1",
    ]);
    assert_eq!(
        (
            args.read_timeout_seconds,
            args.max_connections,
            args.max_rooms,
            args.empty_room_ttl_minutes
        ),
        (5, 1, 1, 0)
    );
    assert_eq!(
        warnings,
        vec![
            "Value 4 is below the minimum 5, using 5",
            "Value 0 is below the minimum 1, using 1",
            "Value -5 is below the minimum 1, using 1",
        ]
    );
}

#[test]
fn limits() {
    let (args, warnings) = run(&["--max-pending-submissions", "-1", "--login-rate-limit=-1"]);
    assert_eq!(
        (args.max_pending_submissions, args.login_rate_limit),
        (None, None)
    );
    assert!(warnings.is_empty());
    let (args, warnings) = run(&["--max-pending-submissions", "0", "--login-rate-limit", "-2"]);
    assert_eq!(
        (args.max_pending_submissions, args.login_rate_limit),
        (Some(50), Some(30))
    );
    assert_eq!(
        warnings,
        vec![
            "Value 0 is not valid for --max-pending-submissions, using 50",
            "Value -2 is not valid for --login-rate-limit, using 30",
        ]
    );
}

#[test]
fn errors_and_help() {
    assert!(is_error(&["--port", "abc"]));
    assert!(is_error(&["--port"]));
    assert!(is_error(&["--port", "1", "--port", "2"]));
    assert!(is_error(&["--bogus"]));
    assert!(is_error(&["--po", "5"]));
    assert!(is_error(&["-port", "5"]));
    assert!(is_error(&["extra", "--port", "5"]));
    match cli::parse(["--port", "abc"]) {
        CliOutcome::Error(text) => {
            assert!(text.contains("--port") && text.contains("abc"), "{text}")
        }
        other => panic!("{other:?}"),
    }
    match cli::parse(["--help"]) {
        CliOutcome::Help(text) => {
            for flag in [
                "--port",
                "--host",
                "--plugins",
                "--proxy-protocol",
                "--language",
                "--http-host",
                "--http-port",
                "--read-timeout",
                "--max-connections",
                "--max-rooms",
                "--empty-room-ttl",
                "--max-pending-submissions",
                "--login-rate-limit",
            ] {
                assert!(text.contains(flag), "{flag} missing from help");
            }
        }
        other => panic!("{other:?}"),
    }
}
