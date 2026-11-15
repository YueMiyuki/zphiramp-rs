//! `zphiramp`: the Phira multiplayer server.
//! It wires the library crates together: arguments, logging, start-up, console, signals and the stop.

mod args;
mod console;
mod panics;
mod server;
mod shutdown;
mod signals;
#[cfg(test)]
mod testutil;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use zphira_common::logging::{self, LogConfig};

use crate::console::interactive::{Interactive, is_interactive};
use crate::server::{PHIRA_HOST_ENV, RunContext, phira_base_url};

const RUNTIME_EXIT_GRACE: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    let boot = Instant::now();

    let log = match logging::init(LogConfig::default()) {
        Ok(log) => log,
        Err(e) => {
            eprintln!("Cannot start logging: {e}");
            return ExitCode::from(1);
        }
    };
    panics::install();

    let terminal = is_interactive().then(Interactive::new);
    if let Some(terminal) = &terminal {
        log.set_console_sink(terminal.clone());
    }

    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let parsed = match args::handle(&argv) {
        args::Handled::Run(parsed) => parsed,
        args::Handled::Exit(code) => {
            log.shutdown();
            if let Some(terminal) = &terminal {
                terminal.close();
            }
            return ExitCode::from(code);
        }
    };

    let code = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => {
            let cx = RunContext {
                args: parsed,
                terminal: terminal.clone(),
                boot,
                phira_url: phira_base_url(std::env::var(PHIRA_HOST_ENV).ok().as_deref()),
            };
            let code =
                catch_unwind(AssertUnwindSafe(|| runtime.block_on(server::run(cx)))).unwrap_or(1);
            runtime.shutdown_timeout(RUNTIME_EXIT_GRACE);
            code
        }
        Err(e) => {
            tracing::error!(
                error = &e as &dyn std::error::Error,
                "Failed to start server"
            );
            1
        }
    };

    // The logger must be shut down before the process exits, or buffered lines are lost.
    log.shutdown();
    if let Some(terminal) = &terminal {
        terminal.close();
    }
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
