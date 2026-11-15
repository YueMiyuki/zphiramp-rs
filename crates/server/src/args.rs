use zphira_common::args::{self, ArgsOutcome, ServerArgs};

#[derive(Debug, PartialEq, Eq)]
pub enum Handled {
    Run(ServerArgs),
    Exit(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Info(String),
    Warn(String),
    Error(String),
}

pub fn plan(outcome: &ArgsOutcome) -> (Vec<Line>, Option<u8>) {
    let usage = || {
        std::iter::once(Line::Info(args::USAGE_HEADER.to_owned()))
            .chain(args::help_lines().map(|l| Line::Info(l.to_owned())))
    };
    match outcome {
        ArgsOutcome::Run { warnings, .. } => (
            warnings.iter().map(|w| Line::Warn(w.clone())).collect(),
            None,
        ),
        ArgsOutcome::Help => (usage().collect(), Some(0)),
        ArgsOutcome::ParseError { message } => {
            let mut lines = vec![Line::Error(args::parse_error_line(message))];
            lines.extend(usage());
            (lines, Some(1))
        }
        ArgsOutcome::Fatal {
            warnings,
            exception,
        } => {
            let message = fatal_message(exception);
            let mut lines: Vec<Line> = warnings.iter().map(|w| Line::Warn(w.clone())).collect();
            lines.push(Line::Error(args::parse_error_line(&message)));
            lines.extend(usage());
            (lines, Some(1))
        }
    }
}

fn fatal_message(exception: &[String]) -> String {
    let first = exception.first().map_or("", String::as_str);
    match first.split_once(": ") {
        Some((_, message)) => message.to_owned(),
        None => first.to_owned(),
    }
}

pub fn handle(argv: &[String]) -> Handled {
    let outcome = args::parse(argv);
    let (lines, exit) = plan(&outcome);
    for line in lines {
        match line {
            Line::Info(m) => tracing::info!("{}", m),
            Line::Warn(m) => tracing::warn!("{}", m),
            Line::Error(m) => tracing::error!("{}", m),
        }
    }
    match (outcome, exit) {
        (ArgsOutcome::Run { args, .. }, None) => Handled::Run(args),
        (_, code) => Handled::Exit(code.unwrap_or(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_of(argv: &[&str]) -> (Vec<Line>, Option<u8>) {
        plan(&args::parse(argv))
    }

    #[test]
    fn help_is_the_usage_header_and_every_help_line_with_exit_zero() {
        let (lines, exit) = plan_of(&["--help"]);
        assert_eq!(exit, Some(0));
        assert_eq!(lines[0], Line::Info("Phira Server Usage:".to_owned()));
        assert_eq!(lines.len(), 1 + args::help_lines().count());
        assert_eq!(args::help_lines().count(), 26);
        assert!(lines.iter().all(|l| matches!(l, Line::Info(_))));
        assert_eq!(
            lines[1],
            Line::Info(
                "Option                               Description                               "
                    .to_owned()
            )
        );
        assert!(
            lines
                .iter()
                .any(|l| matches!(l, Line::Info(t) if t.contains("--login-rate-limit <Integer>")))
        );
        assert!(lines.iter().any(
            |l| matches!(l, Line::Info(t) if t.contains("--max-pending-submissions <Integer>"))
        ));
    }

    #[test]
    fn a_parse_error_is_an_error_line_then_the_usage_and_exit_one() {
        let (lines, exit) = plan_of(&["--bogus"]);
        assert_eq!(exit, Some(1));
        assert_eq!(
            lines[0],
            Line::Error("Failed to parse arguments: bogus is not a recognized option".to_owned())
        );
        assert_eq!(lines[1], Line::Info("Phira Server Usage:".to_owned()));
        assert_eq!(lines.len(), 2 + args::help_lines().count());

        let (lines, exit) = plan_of(&["--port"]);
        assert_eq!(exit, Some(1));
        assert_eq!(
            lines[0],
            Line::Error("Failed to parse arguments: Option port requires an argument".to_owned())
        );
    }

    #[test]
    fn the_fatal_case_is_a_normal_parse_error_n07() {
        let (lines, exit) = plan_of(&["--port", "abc"]);
        assert_eq!(exit, Some(1));
        assert_eq!(
            lines[0],
            Line::Error(
                "Failed to parse arguments: Cannot parse argument 'abc' of option port".to_owned()
            )
        );
        assert_eq!(lines[1], Line::Info("Phira Server Usage:".to_owned()));

        let (lines, exit) = plan_of(&["--port", "1", "--port", "2"]);
        assert_eq!(exit, Some(1));
        assert_eq!(
            lines[0],
            Line::Error(
                "Failed to parse arguments: Found multiple arguments for option port, but you asked for only one"
                    .to_owned()
            )
        );
    }

    #[test]
    fn warnings_logged_before_the_exception_stay_first() {
        let (lines, exit) = plan_of(&["--read-timeout", "1", "--empty-room-ttl", "x"]);
        assert_eq!(exit, Some(1));
        assert_eq!(
            lines[0],
            Line::Warn("Value 1 is below the minimum 5, using 5".to_owned())
        );
        assert_eq!(
            lines[1],
            Line::Error(
                "Failed to parse arguments: Cannot parse argument 'x' of option empty-room-ttl"
                    .to_owned()
            )
        );
    }

    #[test]
    fn clamp_warnings_are_logged_and_the_server_runs() {
        let outcome = args::parse(&["--max-rooms", "0", "--max-connections", "-3"]);
        let (lines, exit) = plan(&outcome);
        assert_eq!(exit, None);
        assert_eq!(
            lines,
            [
                Line::Warn("Value -3 is below the minimum 1, using 1".to_owned()),
                Line::Warn("Value 0 is below the minimum 1, using 1".to_owned()),
            ]
        );
        let ArgsOutcome::Run { args, .. } = outcome else {
            panic!("expected Run")
        };
        assert_eq!((args.max_connections, args.max_rooms), (1, 1));
    }

    #[test]
    fn invalid_limits_warn_and_keep_the_default() {
        let outcome = args::parse(&["--max-pending-submissions", "0", "--login-rate-limit", "-5"]);
        let (lines, exit) = plan(&outcome);
        assert_eq!(exit, None);
        assert_eq!(
            lines,
            [
                Line::Warn(
                    "Value 0 is not valid for --max-pending-submissions, using 50".to_owned()
                ),
                Line::Warn("Value -5 is not valid for --login-rate-limit, using 30".to_owned()),
            ]
        );
        let ArgsOutcome::Run { args, .. } = outcome else {
            panic!("expected Run")
        };
        assert_eq!(args.max_pending_submissions, Some(50));
        assert_eq!(args.login_rate_limit, Some(30));
    }

    #[test]
    fn minus_one_removes_a_limit_without_a_warning() {
        let outcome = args::parse(&["--max-pending-submissions", "-1", "--login-rate-limit", "7"]);
        let (lines, exit) = plan(&outcome);
        assert_eq!((lines, exit), (vec![], None));
        let ArgsOutcome::Run { args, .. } = outcome else {
            panic!("expected Run")
        };
        assert_eq!(args.max_pending_submissions, None);
        assert_eq!(args.login_rate_limit, Some(7));
    }

    #[test]
    fn plain_arguments_log_nothing() {
        let (lines, exit) = plan_of(&["--port", "1234", "--language", "en-US"]);
        assert!(lines.is_empty());
        assert_eq!(exit, None);
    }

    #[test]
    fn handle_returns_the_arguments_or_the_exit_code() {
        assert!(matches!(handle(&[]), Handled::Run(_)));
        assert_eq!(handle(&["--help".to_owned()]), Handled::Exit(0));
        assert_eq!(handle(&["--nope".to_owned()]), Handled::Exit(1));
        assert_eq!(
            handle(&["--port".to_owned(), "x".to_owned()]),
            Handled::Exit(1)
        );
    }
}
