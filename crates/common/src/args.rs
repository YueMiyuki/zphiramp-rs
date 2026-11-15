use std::collections::HashMap;
use std::path::PathBuf;

use crate::java_fmt;

/// Exact help output; trailing spaces are significant.
pub const HELP_TEXT: &str = include_str!("args_help.txt");

pub const USAGE_HEADER: &str = "Phira Server Usage:";

pub fn help_lines() -> impl Iterator<Item = &'static str> {
    HELP_TEXT.lines()
}

pub fn parse_error_line(message: &str) -> String {
    format!("Failed to parse arguments: {message}")
}

pub fn clamp_warning(value: i32, minimum: i32) -> String {
    format!("Value {value} is below the minimum {minimum}, using {minimum}")
}

pub fn invalid_limit_warning(value: i32, flag: &str, default: u32) -> String {
    format!("Value {value} is not valid for {flag}, using {default}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerArgs {
    pub port: u16,
    pub host: String,
    pub http_host: String,
    pub http_port: u16,
    pub plugins_dir: PathBuf,
    pub proxy_protocol: bool,
    pub default_language: String,
    pub read_timeout_seconds: u32,
    pub max_connections: u32,
    pub max_rooms: u32,
    pub empty_room_ttl_minutes: u32,
    pub max_pending_submissions: Option<u32>,
    pub login_rate_limit: Option<u32>,
}

impl Default for ServerArgs {
    fn default() -> Self {
        Self {
            port: 12346,
            host: "0.0.0.0".to_owned(),
            http_host: "0.0.0.0".to_owned(),
            http_port: 8080,
            plugins_dir: PathBuf::from("plugins"),
            proxy_protocol: false,
            default_language: "zh-CN".to_owned(),
            read_timeout_seconds: 3600,
            max_connections: 1000,
            max_rooms: 200,
            empty_room_ttl_minutes: 60,
            max_pending_submissions: Some(50),
            login_rate_limit: Some(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgsOutcome {
    Run {
        args: ServerArgs,
        warnings: Vec<String>,
    },
    Help,
    ParseError {
        message: String,
    },
    Fatal {
        warnings: Vec<String>,
        exception: Vec<String>,
    },
}

impl ArgsOutcome {
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            ArgsOutcome::Run { .. } => None,
            ArgsOutcome::Help => Some(0),
            ArgsOutcome::ParseError { .. } | ArgsOutcome::Fatal { .. } => Some(1),
        }
    }

    pub fn fatal_line(&self) -> Option<String> {
        match self {
            ArgsOutcome::Fatal { exception, .. } => Some(format!(
                "Exception in thread \"main\" {}",
                exception.first().map_or("", String::as_str)
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    RequiredArg,
    OptionalArg,
    NoArg,
    NonOption,
}

const SPECS: [(&str, Kind); 15] = [
    ("[arguments]", Kind::NonOption),
    ("empty-room-ttl", Kind::RequiredArg),
    ("help", Kind::NoArg),
    ("host", Kind::RequiredArg),
    ("http-host", Kind::RequiredArg),
    ("http-port", Kind::RequiredArg),
    ("language", Kind::RequiredArg),
    ("login-rate-limit", Kind::RequiredArg),
    ("max-connections", Kind::RequiredArg),
    ("max-pending-submissions", Kind::RequiredArg),
    ("max-rooms", Kind::RequiredArg),
    ("plugins", Kind::RequiredArg),
    ("port", Kind::RequiredArg),
    ("proxy-protocol", Kind::OptionalArg),
    ("read-timeout", Kind::RequiredArg),
];

fn lookup(key: &str) -> Option<(&'static str, Kind)> {
    if let Some(&spec) = SPECS.iter().find(|(name, _)| *name == key) {
        return Some(spec);
    }
    let mut matches = SPECS.iter().filter(|(name, _)| name.starts_with(key));
    match (matches.next(), matches.next()) {
        (Some(&spec), None) => Some(spec),
        _ => None,
    }
}

fn unrecognized(option: &str) -> String {
    format!("{option} is not a recognized option")
}

fn is_option_terminator(arg: &str) -> bool {
    arg == "--"
}

fn is_long_option_token(arg: &str) -> bool {
    arg.starts_with("--") && !is_option_terminator(arg)
}

fn is_short_option_token(arg: &str) -> bool {
    arg.starts_with('-') && arg != "-" && !is_long_option_token(arg)
}

fn looks_like_an_option(arg: &str) -> bool {
    is_short_option_token(arg) || is_long_option_token(arg)
}

fn key_value(s: &str) -> (&str, Option<&str>) {
    match s.find('=') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    }
}

#[derive(Default)]
struct Detected {
    values: HashMap<&'static str, Vec<String>>,
}

impl Detected {
    fn add(&mut self, name: &'static str) {
        self.values.entry(name).or_default();
    }

    fn add_arguments(&mut self, name: &'static str, argument: &str) {
        let list = self.values.entry(name).or_default();
        let tokens: Vec<&str> = argument.split('\0').filter(|t| !t.is_empty()).collect();
        if tokens.is_empty() {
            list.push(argument.to_owned());
        } else {
            list.extend(tokens.into_iter().map(str::to_owned));
        }
    }
}

struct Parser<'a> {
    args: &'a [String],
    next: usize,
    detected: Detected,
}

impl Parser<'_> {
    fn run(&mut self) -> Result<(), String> {
        let mut more_options = true;
        while self.next < self.args.len() {
            let candidate = &self.args[self.next];
            self.next += 1;
            if !more_options {
                continue;
            }
            if is_option_terminator(candidate) {
                more_options = false;
            } else if is_long_option_token(candidate) {
                let (key, value) = key_value(&candidate[2..]);
                let spec = lookup(key).ok_or_else(|| unrecognized(key))?;
                self.handle(spec, value.map(str::to_owned))?;
            } else if is_short_option_token(candidate) {
                let (key, value) = key_value(&candidate[1..]);
                match lookup(key) {
                    Some(spec) => self.handle(spec, value.map(str::to_owned))?,
                    None => self.short_cluster(&candidate[1..])?,
                }
            }
        }
        Ok(())
    }

    fn short_cluster(&mut self, cluster: &str) -> Result<(), String> {
        let chars: Vec<char> = cluster.chars().collect();
        for &c in &chars {
            let name = c.to_string();
            let (_, kind) = lookup(&name).ok_or_else(|| unrecognized(&name))?;
            if accepts_arguments(kind) {
                break;
            }
        }
        for (i, &c) in chars.iter().enumerate() {
            let spec = lookup(&c.to_string()).expect("validated above");
            if accepts_arguments(spec.1) && chars.len() > i + 1 {
                let argument: String = chars[i + 1..].iter().collect();
                self.handle(spec, Some(argument))?;
                break;
            }
            self.handle(spec, None)?;
        }
        Ok(())
    }

    fn handle(
        &mut self,
        (name, kind): (&'static str, Kind),
        argument: Option<String>,
    ) -> Result<(), String> {
        match kind {
            Kind::NoArg => self.detected.add(name),
            Kind::NonOption => {}
            Kind::RequiredArg | Kind::OptionalArg => match argument {
                Some(a) => self.detected.add_arguments(name, &a),
                None if kind == Kind::RequiredArg => {
                    let Some(next) = self.args.get(self.next) else {
                        return Err(format!("Option {name} requires an argument"));
                    };
                    self.next += 1;
                    self.detected.add_arguments(name, next);
                }
                None => match self.args.get(self.next) {
                    Some(next) if !looks_like_an_option(next) => {
                        self.next += 1;
                        self.detected.add_arguments(name, next);
                    }
                    _ => self.detected.add(name),
                },
            },
        }
        Ok(())
    }
}

fn accepts_arguments(kind: Kind) -> bool {
    matches!(kind, Kind::RequiredArg | Kind::OptionalArg)
}

type Thrown = Vec<String>;

impl Detected {
    fn string(&self, name: &str, default: &str) -> Result<String, Thrown> {
        match self.values.get(name).map(Vec::as_slice) {
            None | Some([]) => Ok(default.to_owned()),
            Some([one]) => Ok(one.clone()),
            Some(_) => Err(multiple(name)),
        }
    }

    fn int(&self, name: &str, default: i32) -> Result<i32, Thrown> {
        let raw = match self.values.get(name).map(Vec::as_slice) {
            None | Some([]) => return Ok(default),
            Some(values) => values,
        };
        // Every value is converted before the count is checked: a bad value wins over a repeat.
        let mut converted = Vec::with_capacity(raw.len());
        for value in raw {
            match java_fmt::parse_int(value) {
                Ok(v) => converted.push(v),
                Err(e) => {
                    return Err(vec![
                        format!(
                            "joptsimple.OptionArgumentConversionException: Cannot parse argument '{value}' of option {name}"
                        ),
                        format!(
                            "joptsimple.internal.ReflectionException: java.lang.NumberFormatException: {}",
                            e.message()
                        ),
                        format!("java.lang.NumberFormatException: {}", e.message()),
                    ]);
                }
            }
        }
        match converted.as_slice() {
            [one] => Ok(*one),
            _ => Err(multiple(name)),
        }
    }

    fn proxy_protocol(&self) -> Result<bool, Thrown> {
        match self.values.get("proxy-protocol").map(Vec::as_slice) {
            None => Ok(false),
            // A bare flag enables proxy protocol.
            Some([]) => Ok(true),
            Some([one]) => Ok(java_fmt::parse_boolean(one)),
            Some(_) => Err(multiple("proxy-protocol")),
        }
    }
}

fn multiple(name: &str) -> Thrown {
    vec![format!(
        "joptsimple.MultipleArgumentsForOptionException: Found multiple arguments for option {name}, but you asked for only one"
    )]
}

fn clamp_port(value: i32) -> u16 {
    value.clamp(1, 65535) as u16
}

fn clamp_positive(value: i32, minimum: i32, warnings: &mut Vec<String>) -> u32 {
    if value < minimum {
        warnings.push(clamp_warning(value, minimum));
        minimum as u32
    } else {
        value as u32
    }
}

fn valid_limit(value: i32, flag: &str, default: u32, warnings: &mut Vec<String>) -> Option<u32> {
    match value {
        -1 => None,
        v if v >= 1 => Some(v as u32),
        v => {
            warnings.push(invalid_limit_warning(v, flag, default));
            Some(default)
        }
    }
}

// Reads in this order: the warnings and the first failure depend on it.
fn evaluate(d: &Detected, warnings: &mut Vec<String>) -> Result<ServerArgs, Thrown> {
    let port = clamp_port(d.int("port", 12346)?);
    let host = d.string("host", "0.0.0.0")?;
    let http_host = d.string("http-host", "0.0.0.0")?;
    let http_port = clamp_port(d.int("http-port", 8080)?);
    let plugins_dir = PathBuf::from(d.string("plugins", "plugins")?);
    let proxy_protocol = d.proxy_protocol()?;
    let default_language = d.string("language", "zh-CN")?;
    let read_timeout_seconds = clamp_positive(d.int("read-timeout", 3600)?, 5, warnings);
    let max_connections = clamp_positive(d.int("max-connections", 1000)?, 1, warnings);
    let max_rooms = clamp_positive(d.int("max-rooms", 200)?, 1, warnings);
    let empty_room_ttl_minutes = d.int("empty-room-ttl", 60)?.max(0) as u32;
    let max_pending_submissions = valid_limit(
        d.int("max-pending-submissions", 50)?,
        "--max-pending-submissions",
        50,
        warnings,
    );
    let login_rate_limit = valid_limit(
        d.int("login-rate-limit", 30)?,
        "--login-rate-limit",
        30,
        warnings,
    );
    Ok(ServerArgs {
        port,
        host,
        http_host,
        http_port,
        plugins_dir,
        proxy_protocol,
        default_language,
        read_timeout_seconds,
        max_connections,
        max_rooms,
        empty_room_ttl_minutes,
        max_pending_submissions,
        login_rate_limit,
    })
}

/// Never logs or exits; the caller logs the warnings and acts on the outcome.
pub fn parse<S: AsRef<str>>(args: &[S]) -> ArgsOutcome {
    let args: Vec<String> = args.iter().map(|a| a.as_ref().to_owned()).collect();
    let mut parser = Parser {
        args: &args,
        next: 0,
        detected: Detected::default(),
    };
    if let Err(message) = parser.run() {
        return ArgsOutcome::ParseError { message };
    }
    let d = parser.detected;
    if d.values.contains_key("help") {
        return ArgsOutcome::Help;
    }

    let mut warnings = Vec::new();
    let result = evaluate(&d, &mut warnings);
    match result {
        Ok(args) => ArgsOutcome::Run { args, warnings },
        Err(exception) => ArgsOutcome::Fatal {
            warnings,
            exception,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let ArgsOutcome::Run { args, warnings } = parse::<&str>(&[]) else {
            panic!("expected Run");
        };
        assert_eq!(args, ServerArgs::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn help_lines_are_the_log_lines() {
        let lines: Vec<&str> = help_lines().collect();
        assert_eq!(lines.len(), 26);
        assert!(lines.iter().all(|l| l.len() == 79));
        assert_eq!(lines[25].trim(), "(default: 3600)");
        assert!(HELP_TEXT.contains(
            "--login-rate-limit <Integer>         HTTP login attempts allowed per minute per"
        ));
    }
}
