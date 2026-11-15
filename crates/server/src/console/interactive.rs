use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind, poll, read,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
use tracing::Level;
use zphira_common::logging::{ConsoleLine, ConsoleSink};

use super::editor::{self, Edit, LineEditor, PROMPT};

const POLL: Duration = Duration::from_millis(100);

pub fn is_interactive() -> bool {
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && std::env::var("TERM").is_ok_and(|t| !t.is_empty() && t != "dumb")
}

struct State {
    editor: LineEditor,
    cursor_row: usize,
    end_row: usize,
    active: bool,
}

pub struct Interactive {
    state: Mutex<State>,
    stop: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn columns() -> usize {
    size().map_or(80, |(cols, _)| usize::from(cols).max(2))
}

fn emit(text: &str) {
    let mut out = io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

impl Interactive {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                editor: LineEditor::new(),
                cursor_row: 0,
                end_row: 0,
                active: false,
            }),
            stop: AtomicBool::new(false),
            thread: Mutex::new(None),
        })
    }

    /// `submit` gets every finished line; `interrupt` runs on Ctrl-C.
    pub fn start(
        self: &Arc<Self>,
        submit: impl Fn(String) + Send + 'static,
        interrupt: impl Fn() + Send + 'static,
    ) -> io::Result<()> {
        {
            // Under the lock, so no log line is written while the terminal is half switched.
            let mut state = lock(&self.state);
            enable_raw_mode()?;
            let _ = crossterm::execute!(io::stdout(), EnableBracketedPaste);
            state.active = true;
            self.repaint(&mut state);
        }
        let this = Arc::clone(self);
        let thread = std::thread::Builder::new()
            .name("Console".to_owned())
            .spawn(move || this.input_loop(&submit, &interrupt))?;
        *lock(&self.thread) = Some(thread);
        Ok(())
    }

    pub fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let thread = lock(&self.thread).take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }

    fn input_loop(&self, submit: &dyn Fn(String), interrupt: &dyn Fn()) {
        let _restore = Restore(self);
        while !self.stop.load(Ordering::SeqCst) {
            match poll(POLL) {
                Ok(false) => {}
                Ok(true) => match read() {
                    Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                        let edit = lock(&self.state).editor.key(key);
                        match edit {
                            Edit::Submit(line) => {
                                self.finish_line();
                                submit(line);
                            }
                            Edit::Interrupt => {
                                self.finish_line();
                                interrupt();
                            }
                            Edit::Eof => self.finish_line(),
                            Edit::ClearScreen => {
                                emit("\x1b[2J\x1b[H");
                                let mut state = lock(&self.state);
                                state.cursor_row = 0;
                                state.end_row = 0;
                                self.repaint(&mut state);
                            }
                            Edit::Changed => {
                                let mut state = lock(&self.state);
                                self.repaint_in_place(&mut state);
                            }
                            Edit::Nothing => {}
                        }
                    }
                    Ok(Event::Paste(text)) => {
                        let lines = lock(&self.state).editor.paste(&text);
                        for line in lines {
                            self.finish_line();
                            submit(line);
                        }
                        let mut state = lock(&self.state);
                        self.repaint_in_place(&mut state);
                    }
                    Ok(Event::Resize(..)) => {
                        let mut state = lock(&self.state);
                        self.repaint_in_place(&mut state);
                    }
                    Ok(_) => {}
                    Err(_) => break,
                },
                Err(_) => break,
            }
        }
    }

    fn finish_line(&self) {
        let mut state = lock(&self.state);
        emit(&editor::finish_line(state.cursor_row, state.end_row));
        state.cursor_row = 0;
        state.end_row = 0;
        self.repaint(&mut state);
    }

    fn repaint(&self, state: &mut State) {
        let painted = editor::draw(
            PROMPT,
            state.editor.chars(),
            state.editor.cursor(),
            columns(),
        );
        state.cursor_row = painted.cursor_row;
        state.end_row = painted.end_row;
        emit(&painted.bytes);
    }

    fn repaint_in_place(&self, state: &mut State) {
        emit(&editor::clear_region(state.cursor_row));
        self.repaint(state);
    }

    fn print_above(&self, state: &mut State, line: &ConsoleLine) {
        let mut out = editor::clear_region(state.cursor_row);
        out.push_str(&render_raw(line));
        emit(&out);
        state.cursor_row = 0;
        state.end_row = 0;
        self.repaint(state);
    }
}

// In raw mode a line feed does not return the carriage, so every line ends in `\r\n`.
fn render_raw(line: &ConsoleLine) -> String {
    let body = line.text.replace('\n', "\r\n");
    match line.level {
        Level::ERROR => format!("\x1b[31;1m{body}\x1b[m\r\n"),
        Level::WARN => format!("\x1b[33;1m{body}\x1b[m\r\n"),
        _ => format!("{body}\r\n"),
    }
}

struct Restore<'a>(&'a Interactive);

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.0.state);
        if state.active {
            state.active = false;
            emit(&editor::clear_region(state.cursor_row));
            state.cursor_row = 0;
            state.end_row = 0;
        }
        let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste);
        let _ = disable_raw_mode();
        drop(state);
    }
}

impl ConsoleSink for Interactive {
    fn write_line(&self, line: &ConsoleLine) {
        let mut state = lock(&self.state);
        if state.active {
            self.print_above(&mut state, line);
        } else {
            emit(&line.render(true));
        }
    }

    fn supports_ansi(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: Level, text: &str) -> ConsoleLine {
        ConsoleLine {
            level,
            text: text.to_owned(),
        }
    }

    #[test]
    fn lines_end_in_carriage_return_and_line_feed_in_raw_mode() {
        assert_eq!(
            render_raw(&line(Level::INFO, "[00:00:00 INFO] [Server]: hi")),
            "[00:00:00 INFO] [Server]: hi\r\n"
        );
        assert_eq!(
            render_raw(&line(Level::WARN, "[00:00:00 WARN] [Server]: careful")),
            "\x1b[33;1m[00:00:00 WARN] [Server]: careful\x1b[m\r\n"
        );
        assert_eq!(
            render_raw(&line(Level::ERROR, "[00:00:00 ERROR] [Server]: bad")),
            "\x1b[31;1m[00:00:00 ERROR] [Server]: bad\x1b[m\r\n"
        );
        assert_eq!(
            render_raw(&line(Level::ERROR, "a\ncause")),
            "\x1b[31;1ma\r\ncause\x1b[m\r\n"
        );
    }

    #[test]
    fn closing_a_console_that_never_started_is_harmless() {
        let terminal = Interactive::new();
        assert!(terminal.supports_ansi());
        terminal.close();
        terminal.close();
    }
}
