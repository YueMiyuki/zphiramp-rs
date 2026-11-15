use std::panic::{self, PanicHookInfo};

// A panic on the log writer thread cannot be logged through the logger itself.
const LOG_WRITER_THREAD: &str = "zphira-log-writer";

pub fn payload_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any>".to_owned()
    }
}

pub fn describe(info: &PanicHookInfo<'_>, thread: &str) -> String {
    let location = info.location().map_or_else(String::new, |l| {
        format!(" at {}:{}:{}", l.file(), l.line(), l.column())
    });
    format!(
        "thread '{thread}' panicked{location}: {}",
        payload_message(info)
    )
}

pub fn install() {
    panic::set_hook(Box::new(|info| {
        let current = std::thread::current();
        let name = current.name().unwrap_or("<unnamed>");
        let line = describe(info, name);
        if name == LOG_WRITER_THREAD {
            eprintln!("{line}");
        } else {
            tracing::error!(target: "STDERR", "{}", line);
        }
    }));
}

#[cfg(test)]
mod tests {
    use tracing::Level;

    use super::*;
    use crate::testutil::{Captured, captured, install_capture};

    fn setup() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            install_capture();
            let previous = panic::take_hook();
            install();
            let ours = panic::take_hook();
            panic::set_hook(Box::new(move |info| {
                ours(info);
                previous(info);
            }));
        });
    }

    fn panics_logged_for(thread: &str) -> Vec<Captured> {
        captured()
            .into_iter()
            .filter(|(_, _, m)| m.contains(&format!("thread '{thread}' panicked")))
            .collect()
    }

    #[test]
    fn a_panic_on_a_thread_is_logged_as_an_error_of_the_stderr_logger() {
        setup();
        let result = std::thread::Builder::new()
            .name("victim".to_owned())
            .spawn(|| panic!("boom {}", 42))
            .unwrap()
            .join();
        assert!(result.is_err());
        let logged = panics_logged_for("victim");
        assert_eq!(logged.len(), 1, "{logged:?}");
        let (level, target, message) = &logged[0];
        assert_eq!(*level, Level::ERROR);
        assert_eq!(target, "STDERR");
        assert!(
            message.contains("panicked at crates/server/src/panics.rs:"),
            "{message}"
        );
        assert!(message.ends_with(": boom 42"), "{message}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_panic_in_a_task_is_logged_and_the_task_reports_it() {
        setup();
        let handle = tokio::spawn(async { panic!("task blew up") });
        let error = handle.await.unwrap_err();
        assert!(error.is_panic());
        let logged = captured()
            .iter()
            .filter(|(_, _, m)| m.ends_with(": task blew up"))
            .count();
        assert_eq!(logged, 1);
    }
}
