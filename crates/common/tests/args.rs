mod support;

use support::{read_fixture, read_tsv};
use zphira_common::args::{self, ArgsOutcome, HELP_TEXT};

fn render(outcome: &ArgsOutcome) -> Vec<String> {
    match outcome {
        ArgsOutcome::Help => vec!["HELP".to_owned()],
        ArgsOutcome::ParseError { message } => vec!["PARSE_ERROR".to_owned(), message.clone()],
        ArgsOutcome::Fatal {
            warnings,
            exception,
        } => {
            vec![
                "FATAL".to_owned(),
                warnings.join(" | "),
                exception.join(" | "),
            ]
        }
        ArgsOutcome::Run { args, warnings } => vec![
            "OK".to_owned(),
            args.port.to_string(),
            args.host.clone(),
            args.http_host.clone(),
            args.http_port.to_string(),
            args.plugins_dir.display().to_string(),
            args.proxy_protocol.to_string(),
            args.default_language.clone(),
            args.read_timeout_seconds.to_string(),
            args.max_connections.to_string(),
            args.max_rooms.to_string(),
            args.empty_room_ttl_minutes.to_string(),
            args.max_pending_submissions
                .map_or("none".to_owned(), |v| v.to_string()),
            args.login_rate_limit
                .map_or("none".to_owned(), |v| v.to_string()),
            warnings.join(" | "),
        ],
    }
}

// The fixture records false for a bare --proxy-protocol; this server enables it.
fn g17_bare_flag(argv: &[String]) -> bool {
    let bare = |i: usize| {
        let a = &argv[i];
        let is_proxy = a == "--proxy-protocol" || a == "--pr" || a == "-pr" || a == "--proxy";
        let next_consumed = argv
            .get(i + 1)
            .is_some_and(|n| !(n.starts_with('-') && n != "-"));
        is_proxy && !next_consumed
    };
    let explicit = argv.iter().enumerate().any(|(i, a)| {
        a.starts_with("--proxy-protocol=")
            || ((a == "--proxy-protocol" || a == "--pr")
                && argv
                    .get(i + 1)
                    .is_some_and(|n| !(n.starts_with('-') && n != "-")))
    });
    (0..argv.len()).any(bare) && !explicit
}

#[test]
fn parser_matches_jopt() {
    let rows = read_tsv("args.tsv");
    assert!(rows.len() > 100);
    let mut failures = Vec::new();
    let mut g17 = 0;
    for row in &rows {
        let n: usize = row[0].parse().unwrap();
        let argv: Vec<String> = row[1..=n].to_vec();
        let mut expected: Vec<String> = row[n + 1..].to_vec();
        let got = args::parse(&argv);
        if g17_bare_flag(&argv) && expected[0] == "OK" {
            assert_eq!(
                expected[6], "false",
                "fixture records the Java behaviour for {argv:?}"
            );
            expected[6] = "true".to_owned();
            g17 += 1;
        }
        if render(&got) != expected {
            failures.push(format!(
                "{argv:?}\n  java: {expected:?}\n  rust: {:?}",
                render(&got)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(g17, 7, "number of bare --proxy-protocol vectors");
}

#[test]
fn help_text_is_jopts() {
    assert_eq!(HELP_TEXT, read_fixture("args_help.txt"));
}

#[test]
fn exit_codes_and_lines() {
    assert_eq!(args::parse(&["--help"]).exit_code(), Some(0));
    let e = args::parse(&["--bogus"]);
    assert_eq!(e.exit_code(), Some(1));
    let ArgsOutcome::ParseError { message } = &e else {
        panic!()
    };
    assert_eq!(
        args::parse_error_line(message),
        "Failed to parse arguments: bogus is not a recognized option"
    );
    let f = args::parse(&["--port", "1", "--port", "2"]);
    assert_eq!(f.exit_code(), Some(1));
    assert_eq!(
        f.fatal_line().unwrap(),
        "Exception in thread \"main\" joptsimple.MultipleArgumentsForOptionException: Found multiple arguments for option port, but you asked for only one"
    );
    assert_eq!(args::parse(&["--port", "5"]).exit_code(), None);
    assert_eq!(
        args::clamp_warning(0, 1),
        "Value 0 is below the minimum 1, using 1"
    );
    assert_eq!(
        args::invalid_limit_warning(0, "--login-rate-limit", 30),
        "Value 0 is not valid for --login-rate-limit, using 30"
    );
}

#[test]
fn g17_explicit_values_keep_jopt_semantics() {
    let proxy = |argv: &[&str]| match args::parse(argv) {
        ArgsOutcome::Run { args, .. } => args.proxy_protocol,
        other => panic!("{other:?}"),
    };
    assert!(proxy(&["--proxy-protocol"]));
    assert!(proxy(&["--proxy-protocol", "TRUE"]));
    assert!(proxy(&["--proxy-protocol=true"]));
    assert!(!proxy(&["--proxy-protocol", "false"]));
    assert!(!proxy(&["--proxy-protocol", "yes"]));
    assert!(!proxy(&["--proxy-protocol="]));
    assert!(!proxy(&[]));
    assert!(!proxy(&["--proxy-protocol", "false", "--proxy-protocol"]));
}

#[test]
fn limit_flags() {
    let run = |argv: &[&str]| match args::parse(argv) {
        ArgsOutcome::Run { args, warnings } => (
            args.max_pending_submissions,
            args.login_rate_limit,
            warnings,
        ),
        other => panic!("{other:?}"),
    };
    assert_eq!(run(&[]), (Some(50), Some(30), vec![]));
    assert_eq!(
        run(&[
            "--max-pending-submissions",
            "-1",
            "--login-rate-limit",
            "-1"
        ]),
        (None, None, vec![])
    );
    assert_eq!(
        run(&[
            "--max-pending-submissions",
            "1",
            "--login-rate-limit",
            "120"
        ]),
        (Some(1), Some(120), vec![])
    );
    assert_eq!(
        run(&["--max-pending-submissions", "0", "--login-rate-limit", "-2"]),
        (
            Some(50),
            Some(30),
            vec![
                "Value 0 is not valid for --max-pending-submissions, using 50".to_owned(),
                "Value -2 is not valid for --login-rate-limit, using 30".to_owned(),
            ]
        )
    );
    assert!(matches!(
        args::parse(&["--login-rate-limit", "x"]),
        ArgsOutcome::Fatal { .. }
    ));
    assert!(matches!(
        args::parse(&["--l", "en-US"]),
        ArgsOutcome::ParseError { .. }
    ));
    assert_eq!(run(&["--lo", "5"]).1, Some(5));
    assert_eq!(run(&["--max-p", "5"]).0, Some(5));
}
