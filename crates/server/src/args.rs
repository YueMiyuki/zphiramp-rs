use zphira_common::cli::{self, CliOutcome, ServerArgs};

#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Run {
        args: ServerArgs,
        warnings: Vec<String>,
    },
    Exit {
        code: u8,
        stdout: String,
        stderr: String,
    },
}

pub fn parse(argv: &[String]) -> Next {
    match cli::parse(argv) {
        CliOutcome::Run { args, warnings } => Next::Run { args, warnings },
        CliOutcome::Help(text) => Next::Exit {
            code: 0,
            stdout: text,
            stderr: String::new(),
        },
        CliOutcome::Error(text) => Next::Exit {
            code: 1,
            stdout: String::new(),
            stderr: text,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(argv: &[&str]) -> Next {
        parse(&argv.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }

    fn exit_of(argv: &[&str]) -> (u8, String, String) {
        match parse_str(argv) {
            Next::Exit {
                code,
                stdout,
                stderr,
            } => (code, stdout, stderr),
            Next::Run { .. } => panic!("expected an exit for {argv:?}"),
        }
    }

    fn run_of(argv: &[&str]) -> (ServerArgs, Vec<String>) {
        match parse_str(argv) {
            Next::Run { args, warnings } => (args, warnings),
            Next::Exit { stderr, .. } => panic!("expected Run for {argv:?}: {stderr}"),
        }
    }

    #[test]
    fn help_goes_to_stdout_with_exit_zero() {
        let (code, stdout, stderr) = exit_of(&["--help"]);
        assert_eq!(code, 0);
        assert!(stderr.is_empty());
        for flag in [
            "--port",
            "--host",
            "--http-port",
            "--proxy-protocol",
            "--login-rate-limit",
            "--max-pending-submissions",
        ] {
            assert!(stdout.contains(flag), "{flag} missing from:\n{stdout}");
        }
    }

    #[test]
    fn argument_errors_go_to_stderr_with_exit_one() {
        for argv in [
            vec!["--bogus"],
            vec!["--port"],
            vec!["--port", "abc"],
            vec!["--port", "1", "--port", "2"],
            vec!["--max-rooms", "1.5"],
        ] {
            let (code, stdout, stderr) = exit_of(&argv);
            assert_eq!(code, 1, "{argv:?}");
            assert!(stdout.is_empty(), "{argv:?}");
            assert!(!stderr.trim().is_empty(), "{argv:?}");
        }
    }

    #[test]
    fn abbreviations_and_single_dash_flags_are_not_accepted() {
        assert_eq!(exit_of(&["--po", "1"]).0, 1);
        assert_eq!(exit_of(&["-port", "1"]).0, 1);
    }

    #[test]
    fn defaults_and_values() {
        let (args, warnings) = run_of(&[]);
        assert_eq!(args, ServerArgs::default());
        assert!(warnings.is_empty());
        let (args, _) = run_of(&["--port", "1234", "--http-port=9000", "--language", "en-US"]);
        assert_eq!((args.port, args.http_port), (1234, 9000));
        assert_eq!(args.default_language, "en-US");
    }

    #[test]
    fn clamps_come_with_warnings() {
        let (args, warnings) = run_of(&[
            "--read-timeout",
            "1",
            "--max-connections",
            "-3",
            "--max-rooms",
            "0",
        ]);
        assert_eq!(
            (
                args.read_timeout_seconds,
                args.max_connections,
                args.max_rooms
            ),
            (5, 1, 1)
        );
        assert_eq!(
            warnings,
            [
                "Value 1 is below the minimum 5, using 5",
                "Value -3 is below the minimum 1, using 1",
                "Value 0 is below the minimum 1, using 1",
            ]
        );
        let (args, warnings) = run_of(&["--port", "70000", "--http-port", "0"]);
        assert_eq!((args.port, args.http_port), (65535, 1));
        assert!(warnings.is_empty());
    }

    #[test]
    fn the_two_limits() {
        let (args, warnings) =
            run_of(&["--max-pending-submissions", "0", "--login-rate-limit", "-5"]);
        assert_eq!(args.max_pending_submissions, Some(50));
        assert_eq!(args.login_rate_limit, Some(30));
        assert_eq!(
            warnings,
            [
                "Value 0 is not valid for --max-pending-submissions, using 50",
                "Value -5 is not valid for --login-rate-limit, using 30",
            ]
        );
        let (args, warnings) =
            run_of(&["--max-pending-submissions", "-1", "--login-rate-limit", "7"]);
        assert_eq!(args.max_pending_submissions, None);
        assert_eq!(args.login_rate_limit, Some(7));
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_bare_proxy_protocol_flag_turns_it_on() {
        assert!(run_of(&["--proxy-protocol"]).0.proxy_protocol);
        assert!(run_of(&["--proxy-protocol", "true"]).0.proxy_protocol);
        assert!(!run_of(&["--proxy-protocol", "false"]).0.proxy_protocol);
        assert!(!run_of(&[]).0.proxy_protocol);
    }
}
