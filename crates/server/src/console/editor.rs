use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

pub const PROMPT: &str = "> ";

const HISTORY_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Nothing,
    Changed,
    Submit(String),
    Interrupt,
    Eof,
    ClearScreen,
}

#[derive(Debug, Default)]
pub struct LineEditor {
    buf: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    browsing: Option<usize>,
    stash: Vec<char>,
    kill: Vec<char>,
}

impl LineEditor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn chars(&self) -> &[char] {
        &self.buf
    }

    pub fn text(&self) -> String {
        self.buf.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    #[cfg(test)]
    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn key(&mut self, key: KeyEvent) -> Edit {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter => self.submit(),
            KeyCode::Char(c) if ctrl => self.control(c),
            KeyCode::Char(c) if alt => self.meta(c),
            KeyCode::Char(c) => self.insert(c),
            KeyCode::Backspace if alt => self.kill_word_back(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left if ctrl || alt => self.move_to(self.word_start()),
            KeyCode::Right if ctrl || alt => self.move_to(self.word_end()),
            KeyCode::Left => self.move_to(self.cursor.saturating_sub(1)),
            KeyCode::Right => self.move_to((self.cursor + 1).min(self.buf.len())),
            KeyCode::Home => self.move_to(0),
            KeyCode::End => self.move_to(self.buf.len()),
            KeyCode::Up => self.history_prev(),
            KeyCode::Down => self.history_next(),
            _ => Edit::Nothing,
        }
    }

    /// Returns the lines finished by line breaks in `text`; the rest stays in the editor.
    pub fn paste(&mut self, text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut segments = normalized.split('\n').peekable();
        while let Some(segment) = segments.next() {
            for c in segment.chars() {
                self.insert(c);
            }
            if segments.peek().is_some()
                && let Edit::Submit(line) = self.submit()
            {
                lines.push(line);
            }
        }
        lines
    }

    fn control(&mut self, c: char) -> Edit {
        match c.to_ascii_lowercase() {
            'c' => {
                self.buf.clear();
                self.cursor = 0;
                Edit::Interrupt
            }
            'd' if self.buf.is_empty() => Edit::Eof,
            'd' => self.delete(),
            'a' => self.move_to(0),
            'e' => self.move_to(self.buf.len()),
            'b' => self.move_to(self.cursor.saturating_sub(1)),
            'f' => self.move_to((self.cursor + 1).min(self.buf.len())),
            'h' => self.backspace(),
            'j' | 'm' => self.submit(),
            'u' => self.kill_range(0, self.cursor),
            'k' => self.kill_range(self.cursor, self.buf.len()),
            'w' => self.kill_word_back(),
            'y' => self.yank(),
            'l' => Edit::ClearScreen,
            'p' => self.history_prev(),
            'n' => self.history_next(),
            _ => Edit::Nothing,
        }
    }

    fn meta(&mut self, c: char) -> Edit {
        match c.to_ascii_lowercase() {
            'b' => self.move_to(self.word_start()),
            'f' => self.move_to(self.word_end()),
            'd' => {
                let end = self.word_end();
                self.kill_range(self.cursor, end)
            }
            _ => Edit::Nothing,
        }
    }

    fn insert(&mut self, c: char) -> Edit {
        if c.is_control() {
            return Edit::Nothing;
        }
        self.leave_history();
        self.buf.insert(self.cursor, c);
        self.cursor += 1;
        Edit::Changed
    }

    fn backspace(&mut self) -> Edit {
        if self.cursor == 0 {
            return Edit::Nothing;
        }
        self.leave_history();
        self.cursor -= 1;
        self.buf.remove(self.cursor);
        Edit::Changed
    }

    fn delete(&mut self) -> Edit {
        if self.cursor >= self.buf.len() {
            return Edit::Nothing;
        }
        self.leave_history();
        self.buf.remove(self.cursor);
        Edit::Changed
    }

    fn move_to(&mut self, position: usize) -> Edit {
        if position == self.cursor {
            return Edit::Nothing;
        }
        self.cursor = position;
        Edit::Changed
    }

    fn word_start(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.buf[i - 1] == ' ' {
            i -= 1;
        }
        while i > 0 && self.buf[i - 1] != ' ' {
            i -= 1;
        }
        i
    }

    fn word_end(&self) -> usize {
        let mut i = self.cursor;
        while i < self.buf.len() && self.buf[i] == ' ' {
            i += 1;
        }
        while i < self.buf.len() && self.buf[i] != ' ' {
            i += 1;
        }
        i
    }

    fn kill_range(&mut self, from: usize, to: usize) -> Edit {
        if from >= to {
            return Edit::Nothing;
        }
        self.leave_history();
        self.kill = self.buf.drain(from..to).collect();
        self.cursor = from;
        Edit::Changed
    }

    fn kill_word_back(&mut self) -> Edit {
        let start = self.word_start();
        self.kill_range(start, self.cursor)
    }

    fn yank(&mut self) -> Edit {
        if self.kill.is_empty() {
            return Edit::Nothing;
        }
        self.leave_history();
        let kill = self.kill.clone();
        for c in kill {
            self.buf.insert(self.cursor, c);
            self.cursor += 1;
        }
        Edit::Changed
    }

    fn submit(&mut self) -> Edit {
        let line = self.text();
        self.buf.clear();
        self.cursor = 0;
        self.browsing = None;
        self.stash.clear();
        if !line.trim().is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > HISTORY_LIMIT {
                self.history.remove(0);
            }
        }
        Edit::Submit(line)
    }

    fn leave_history(&mut self) {
        self.browsing = None;
    }

    fn load(&mut self, text: &str) {
        self.buf = text.chars().collect();
        self.cursor = self.buf.len();
    }

    fn history_prev(&mut self) -> Edit {
        let next = match self.browsing {
            None if self.history.is_empty() => return Edit::Nothing,
            None => {
                self.stash = self.buf.clone();
                self.history.len() - 1
            }
            Some(0) => return Edit::Nothing,
            Some(i) => i - 1,
        };
        self.browsing = Some(next);
        let line = self.history[next].clone();
        self.load(&line);
        Edit::Changed
    }

    fn history_next(&mut self) -> Edit {
        let Some(i) = self.browsing else {
            return Edit::Nothing;
        };
        if i + 1 < self.history.len() {
            self.browsing = Some(i + 1);
            let line = self.history[i + 1].clone();
            self.load(&line);
        } else {
            self.browsing = None;
            self.buf = std::mem::take(&mut self.stash);
            self.cursor = self.buf.len();
        }
        Edit::Changed
    }
}

fn advance(text: impl Iterator<Item = char>, cols: usize) -> (usize, usize) {
    let (mut row, mut col) = (0, 0);
    for c in text {
        let width = c.width().unwrap_or(0);
        if width == 0 {
            continue;
        }
        if col + width > cols {
            row += 1;
            col = 0;
        }
        col += width;
    }
    (row, col)
}

fn position(text: impl Iterator<Item = char>, cols: usize) -> (usize, usize) {
    let (row, col) = advance(text, cols);
    if col >= cols {
        (row + 1, 0)
    } else {
        (row, col)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Painted {
    pub bytes: String,
    pub cursor_row: usize,
    pub end_row: usize,
}

pub fn clear_region(cursor_row: usize) -> String {
    let mut out = String::from("\r");
    if cursor_row > 0 {
        out.push_str(&format!("\x1b[{cursor_row}A"));
    }
    out.push_str("\x1b[J");
    out
}

/// Paints the prompt and line with the cursor at `cursor`, starting from the prompt's first column.
pub fn draw(prompt: &str, line: &[char], cursor: usize, cols: usize) -> Painted {
    let cols = cols.max(2);
    let mut bytes = String::with_capacity(prompt.len() + line.len() + 16);
    bytes.push_str(prompt);
    bytes.extend(line.iter());
    let all = || prompt.chars().chain(line.iter().copied());
    let (end_row_raw, end_col) = advance(all(), cols);
    let (end_row, end_col) = if end_col >= cols {
        // A line that fills its last row needs an explicit wrap before the cursor can be addressed.
        bytes.push_str("\n\r");
        (end_row_raw + 1, 0)
    } else {
        (end_row_raw, end_col)
    };
    let before = prompt
        .chars()
        .chain(line[..cursor.min(line.len())].iter().copied());
    let (cursor_row, cursor_col) = position(before, cols);
    if (cursor_row, cursor_col) != (end_row, end_col) {
        if end_row > cursor_row {
            bytes.push_str(&format!("\x1b[{}A", end_row - cursor_row));
        }
        bytes.push('\r');
        if cursor_col > 0 {
            bytes.push_str(&format!("\x1b[{cursor_col}C"));
        }
    }
    Painted {
        bytes,
        cursor_row,
        end_row,
    }
}

pub fn finish_line(painted_cursor_row: usize, end_row: usize) -> String {
    let mut out = String::new();
    if end_row > painted_cursor_row {
        out.push_str(&format!("\x1b[{}B", end_row - painted_cursor_row));
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_text(ed: &mut LineEditor, text: &str) {
        for c in text.chars() {
            ed.key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_editing_and_submitting() {
        let mut ed = LineEditor::new();
        type_text(&mut ed, "room lisx");
        assert_eq!(ed.key(key(KeyCode::Backspace)), Edit::Changed);
        type_text(&mut ed, "t");
        assert_eq!(ed.text(), "room list");
        ed.key(key(KeyCode::Home));
        assert_eq!(ed.cursor(), 0);
        ed.key(key(KeyCode::Delete));
        assert_eq!(ed.text(), "oom list");
        ed.key(ctrl('e'));
        assert_eq!(ed.cursor(), 8);
        assert_eq!(
            ed.key(key(KeyCode::Enter)),
            Edit::Submit("oom list".to_owned())
        );
        assert_eq!(ed.text(), "");
        assert_eq!(ed.cursor(), 0);
    }

    #[test]
    fn insert_in_the_middle_and_cursor_keys() {
        let mut ed = LineEditor::new();
        type_text(&mut ed, "ab");
        ed.key(key(KeyCode::Left));
        type_text(&mut ed, "X");
        assert_eq!(ed.text(), "aXb");
        assert_eq!(ed.cursor(), 2);
        ed.key(key(KeyCode::Right));
        ed.key(key(KeyCode::Right));
        assert_eq!(ed.cursor(), 3);
        assert_eq!(ed.key(key(KeyCode::Right)), Edit::Nothing);
    }

    #[test]
    fn kill_and_yank() {
        let mut ed = LineEditor::new();
        type_text(&mut ed, "say hello world");
        ed.key(ctrl('w'));
        assert_eq!(ed.text(), "say hello ");
        ed.key(ctrl('u'));
        assert_eq!(ed.text(), "");
        ed.key(ctrl('y'));
        assert_eq!(ed.text(), "say hello ");
        ed.key(ctrl('a'));
        ed.key(ctrl('k'));
        assert_eq!(ed.text(), "");
        ed.key(ctrl('y'));
        assert_eq!(ed.text(), "say hello ");
    }

    #[test]
    fn word_motion() {
        let mut ed = LineEditor::new();
        type_text(&mut ed, "pool add 1");
        ed.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(ed.cursor(), 9);
        ed.key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(ed.cursor(), 5);
        ed.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT));
        assert_eq!(ed.cursor(), 8);
    }

    #[test]
    fn history_browsing_keeps_the_unfinished_line() {
        let mut ed = LineEditor::new();
        for line in ["one", "two", "two", "three"] {
            type_text(&mut ed, line);
            ed.key(key(KeyCode::Enter));
        }
        assert_eq!(ed.history(), ["one", "two", "three"]);
        type_text(&mut ed, "draft");
        ed.key(key(KeyCode::Up));
        assert_eq!(ed.text(), "three");
        ed.key(key(KeyCode::Up));
        ed.key(key(KeyCode::Up));
        assert_eq!(ed.text(), "one");
        assert_eq!(ed.key(key(KeyCode::Up)), Edit::Nothing);
        ed.key(key(KeyCode::Down));
        assert_eq!(ed.text(), "two");
        ed.key(key(KeyCode::Down));
        ed.key(key(KeyCode::Down));
        assert_eq!(ed.text(), "draft");
        assert_eq!(ed.key(key(KeyCode::Down)), Edit::Nothing);
    }

    #[test]
    fn a_blank_line_is_not_recorded() {
        let mut ed = LineEditor::new();
        type_text(&mut ed, "   ");
        assert_eq!(ed.key(key(KeyCode::Enter)), Edit::Submit("   ".to_owned()));
        assert!(ed.history().is_empty());
    }

    #[test]
    fn control_c_and_control_d() {
        let mut ed = LineEditor::new();
        assert_eq!(ed.key(ctrl('d')), Edit::Eof);
        type_text(&mut ed, "ab");
        ed.key(ctrl('a'));
        assert_eq!(ed.key(ctrl('d')), Edit::Changed);
        assert_eq!(ed.text(), "b");
        assert_eq!(ed.key(ctrl('c')), Edit::Interrupt);
        assert_eq!(ed.key(ctrl('l')), Edit::ClearScreen);
    }

    #[test]
    fn control_characters_are_not_inserted() {
        let mut ed = LineEditor::new();
        assert_eq!(ed.key(key(KeyCode::Char('\u{7}'))), Edit::Nothing);
        assert_eq!(ed.key(key(KeyCode::Tab)), Edit::Nothing);
        assert_eq!(ed.text(), "");
    }

    #[test]
    fn paste_splits_lines() {
        let mut ed = LineEditor::new();
        let lines = ed.paste("room list\r\nadmin list\nsay hi");
        assert_eq!(lines, ["room list", "admin list"]);
        assert_eq!(ed.text(), "say hi");
    }

    #[test]
    fn wide_characters_edit_by_character() {
        let mut ed = LineEditor::new();
        type_text(&mut ed, "say 你好");
        ed.key(key(KeyCode::Backspace));
        assert_eq!(ed.text(), "say 你");
    }

    #[test]
    fn draws_a_short_line_with_the_cursor_at_the_end() {
        let line: Vec<char> = "room".chars().collect();
        let painted = draw(PROMPT, &line, 4, 80);
        assert_eq!(painted.bytes, "> room");
        assert_eq!(painted.cursor_row, 0);
        assert_eq!(painted.end_row, 0);
    }

    #[test]
    fn draws_the_cursor_inside_the_line() {
        let line: Vec<char> = "room".chars().collect();
        let painted = draw(PROMPT, &line, 1, 80);
        assert_eq!(painted.bytes, "> room\r\x1b[3C");
    }

    #[test]
    fn a_line_that_fills_the_row_wraps_explicitly() {
        let line: Vec<char> = "abcdef".chars().collect();
        let painted = draw(PROMPT, &line, 6, 8);
        assert_eq!(painted.bytes, "> abcdef\n\r");
        assert_eq!(painted.cursor_row, 1);
        assert_eq!(painted.end_row, 1);
    }

    #[test]
    fn a_wrapped_line_moves_the_cursor_up() {
        let line: Vec<char> = "abcdefghij".chars().collect();
        let painted = draw(PROMPT, &line, 2, 8);
        assert_eq!(painted.end_row, 1);
        assert_eq!(painted.cursor_row, 0);
        assert_eq!(painted.bytes, "> abcdefghij\x1b[1A\r\x1b[4C");
        assert_eq!(clear_region(painted.cursor_row), "\r\x1b[J");
        assert_eq!(clear_region(1), "\r\x1b[1A\x1b[J");
        assert_eq!(finish_line(0, 1), "\x1b[1B\r\n");
    }

    #[test]
    fn wide_characters_wrap_as_a_unit() {
        let line: Vec<char> = "你你".chars().collect();
        let painted = draw(PROMPT, &line, 2, 5);
        assert_eq!(painted.end_row, 1);
        assert_eq!(painted.cursor_row, 1);
    }
}
