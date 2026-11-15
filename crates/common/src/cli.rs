use std::path::PathBuf;

use clap::Parser;
use clap::error::ErrorKind;

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
pub enum CliOutcome {
    Run {
        args: ServerArgs,
        warnings: Vec<String>,
    },
    Help(String),
    Error(String),
}

#[derive(Parser, Debug)]
#[command(
    name = "zphiramp",
    about = "Phira Server",
    allow_negative_numbers = true
)]
struct Flags {
    #[arg(long, default_value_t = 12346, help = "Server listening port")]
    port: i32,
    #[arg(
        long,
        default_value = "0.0.0.0",
        help = "Bind address (IP to listen on)"
    )]
    host: String,
    #[arg(long, default_value = "plugins", help = "Plugins directory path")]
    plugins: String,
    #[arg(
        long = "proxy-protocol",
        num_args = 0..=1,
        default_value = "false",
        default_missing_value = "true",
        value_parser = parse_bool,
        help = "Enable proxy protocol support"
    )]
    proxy_protocol: bool,
    #[arg(
        long,
        default_value = "zh-CN",
        help = "Default server language (e.g., zh-CN, en-US)"
    )]
    language: String,
    #[arg(
        long = "http-host",
        default_value = "0.0.0.0",
        help = "HTTP API bind address"
    )]
    http_host: String,
    #[arg(
        long = "http-port",
        default_value_t = 8080,
        help = "HTTP API listening port"
    )]
    http_port: i32,
    #[arg(
        long = "read-timeout",
        default_value_t = 3600,
        help = "Seconds before an idle client is dropped"
    )]
    read_timeout: i32,
    #[arg(
        long = "max-connections",
        default_value_t = 1000,
        help = "Maximum simultaneous connections"
    )]
    max_connections: i32,
    #[arg(
        long = "max-rooms",
        default_value_t = 200,
        help = "Maximum rooms alive at once"
    )]
    max_rooms: i32,
    #[arg(
        long = "empty-room-ttl",
        default_value_t = 60,
        help = "Minutes a room may stay empty before being reclaimed (0 disables reclaiming)"
    )]
    empty_room_ttl: i32,
    #[arg(
        long = "max-pending-submissions",
        default_value_t = 50,
        help = "Pending chart submissions allowed per account (-1 for no limit)"
    )]
    max_pending_submissions: i32,
    #[arg(
        long = "login-rate-limit",
        default_value_t = 30,
        help = "HTTP login attempts allowed per minute per client address (-1 for no limit)"
    )]
    login_rate_limit: i32,
}

fn parse_bool(text: &str) -> Result<bool, String> {
    Ok(text.eq_ignore_ascii_case("true"))
}

/// Never prints or exits; `args` excludes the program name.
pub fn parse<I, S>(args: I) -> CliOutcome
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let argv = std::iter::once(std::ffi::OsString::from("zphiramp"))
        .chain(args.into_iter().map(Into::into));
    let flags = match Flags::try_parse_from(argv) {
        Ok(flags) => flags,
        Err(e) if e.kind() == ErrorKind::DisplayHelp => {
            return CliOutcome::Help(e.render().to_string());
        }
        Err(e) => return CliOutcome::Error(e.render().to_string()),
    };
    let mut warnings = Vec::new();
    let args = ServerArgs {
        port: flags.port.clamp(1, 65535) as u16,
        host: flags.host,
        http_host: flags.http_host,
        http_port: flags.http_port.clamp(1, 65535) as u16,
        plugins_dir: PathBuf::from(flags.plugins),
        proxy_protocol: flags.proxy_protocol,
        default_language: flags.language,
        read_timeout_seconds: at_least(flags.read_timeout, 5, &mut warnings),
        max_connections: at_least(flags.max_connections, 1, &mut warnings),
        max_rooms: at_least(flags.max_rooms, 1, &mut warnings),
        empty_room_ttl_minutes: flags.empty_room_ttl.max(0) as u32,
        max_pending_submissions: limit(
            flags.max_pending_submissions,
            "--max-pending-submissions",
            50,
            &mut warnings,
        ),
        login_rate_limit: limit(
            flags.login_rate_limit,
            "--login-rate-limit",
            30,
            &mut warnings,
        ),
    };
    CliOutcome::Run { args, warnings }
}

fn at_least(value: i32, minimum: i32, warnings: &mut Vec<String>) -> u32 {
    if value < minimum {
        warnings.push(format!(
            "Value {value} is below the minimum {minimum}, using {minimum}"
        ));
        minimum as u32
    } else {
        value as u32
    }
}

fn limit(value: i32, flag: &str, default: u32, warnings: &mut Vec<String>) -> Option<u32> {
    match value {
        -1 => None,
        v if v >= 1 => Some(v as u32),
        v => {
            warnings.push(format!(
                "Value {v} is not valid for {flag}, using {default}"
            ));
            Some(default)
        }
    }
}
