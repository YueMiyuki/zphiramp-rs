use super::error::GsonError;
use super::node::Node;

const NESTING_LIMIT: usize = 255;
const MIN_INCOMPLETE_INTEGER: i64 = i64::MIN / 10;
const BUFFER_SIZE: usize = 1024;
const MALFORMED_URL: &str =
    "https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    EmptyArray,
    NonemptyArray,
    EmptyObject,
    DanglingName,
    NonemptyObject,
    EmptyDocument,
    NonemptyDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Peeked {
    None,
    BeginObject,
    EndObject,
    BeginArray,
    EndArray,
    True,
    False,
    Null,
    SingleQuoted,
    DoubleQuoted,
    Unquoted,
    SingleQuotedName,
    DoubleQuotedName,
    UnquotedName,
    Long,
    Number,
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token {
    BeginArray,
    EndArray,
    BeginObject,
    EndObject,
    Name,
    String,
    Number,
    Boolean,
    Null,
    EndDocument,
}

impl Token {
    pub(crate) fn java_name(self) -> &'static str {
        match self {
            Token::BeginArray => "BEGIN_ARRAY",
            Token::EndArray => "END_ARRAY",
            Token::BeginObject => "BEGIN_OBJECT",
            Token::EndObject => "END_OBJECT",
            Token::Name => "NAME",
            Token::String => "STRING",
            Token::Number => "NUMBER",
            Token::Boolean => "BOOLEAN",
            Token::Null => "NULL",
            Token::EndDocument => "END_DOCUMENT",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ReadError {
    Eof(String),
    Malformed(String),
    IllegalState(String),
}

impl ReadError {
    pub(crate) fn into_syntax_error(self) -> GsonError {
        match self {
            ReadError::Eof(m) => GsonError::syntax(format!("java.io.EOFException: {m}")),
            ReadError::Malformed(m) => GsonError::syntax(format!(
                "com.google.gson.stream.MalformedJsonException: {m}"
            )),
            ReadError::IllegalState(m) => {
                GsonError::syntax(format!("java.lang.IllegalStateException: {m}"))
            }
        }
    }
}

type Result<T> = std::result::Result<T, ReadError>;

pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    line_number: usize,
    line_start: usize,
    peeked: Peeked,
    peeked_long: i64,
    peeked_number_length: usize,
    stack: Vec<Scope>,
    path_names: Vec<Option<String>>,
    path_indices: Vec<usize>,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(input: &'a str) -> Self {
        let buf = input.as_bytes();
        let mut reader = Reader {
            buf,
            pos: 0,
            line_number: 0,
            line_start: 0,
            peeked: Peeked::None,
            peeked_long: 0,
            peeked_number_length: 0,
            stack: vec![Scope::EmptyDocument],
            path_names: vec![None],
            path_indices: vec![0],
        };
        if buf.starts_with("\u{feff}".as_bytes()) {
            reader.pos = 3;
            reader.line_start = 3;
        }
        reader
    }

    fn path(&self) -> String {
        let mut result = String::from("$");
        for (i, scope) in self.stack.iter().enumerate() {
            match scope {
                Scope::EmptyArray | Scope::NonemptyArray => {
                    result.push('[');
                    result.push_str(&self.path_indices[i].to_string());
                    result.push(']');
                }
                Scope::EmptyObject | Scope::DanglingName | Scope::NonemptyObject => {
                    result.push('.');
                    if let Some(name) = &self.path_names[i] {
                        result.push_str(name);
                    }
                }
                Scope::EmptyDocument | Scope::NonemptyDocument => {}
            }
        }
        result
    }

    fn location(&self) -> String {
        let start = self.line_start.min(self.pos);
        let column = String::from_utf8_lossy(&self.buf[start..self.pos])
            .encode_utf16()
            .count()
            + 1;
        format!(
            " at line {} column {} path {}",
            self.line_number + 1,
            column,
            self.path()
        )
    }

    fn syntax_error(&self, message: &str) -> ReadError {
        ReadError::Malformed(format!("{message}{}\nSee {MALFORMED_URL}", self.location()))
    }

    fn unexpected_token(&mut self, expected: &str) -> ReadError {
        let token = match self.peek() {
            Ok(t) => t,
            Err(e) => return e,
        };
        let id = if token == Token::Null {
            "adapter-not-null-safe"
        } else {
            "unexpected-json-structure"
        };
        ReadError::IllegalState(format!(
            "Expected {expected} but was {}{}\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#{id}",
            token.java_name(),
            self.location()
        ))
    }

    fn top(&self) -> Scope {
        *self.stack.last().expect("reader stack never empty")
    }

    fn set_top(&mut self, scope: Scope) {
        *self.stack.last_mut().expect("reader stack never empty") = scope;
    }

    fn push(&mut self, scope: Scope) -> Result<()> {
        if self.stack.len() > NESTING_LIMIT {
            return Err(ReadError::Malformed(format!(
                "Nesting limit {NESTING_LIMIT} reached{}",
                self.location()
            )));
        }
        self.stack.push(scope);
        if self.path_names.len() < self.stack.len() {
            self.path_names.push(None);
            self.path_indices.push(0);
        }
        Ok(())
    }

    fn bump_index(&mut self) {
        let i = self.stack.len() - 1;
        self.path_indices[i] += 1;
    }

    pub(crate) fn peek(&mut self) -> Result<Token> {
        let p = if self.peeked == Peeked::None {
            self.do_peek()?
        } else {
            self.peeked
        };
        Ok(match p {
            Peeked::BeginObject => Token::BeginObject,
            Peeked::EndObject => Token::EndObject,
            Peeked::BeginArray => Token::BeginArray,
            Peeked::EndArray => Token::EndArray,
            Peeked::SingleQuotedName | Peeked::DoubleQuotedName | Peeked::UnquotedName => {
                Token::Name
            }
            Peeked::True | Peeked::False => Token::Boolean,
            Peeked::Null => Token::Null,
            Peeked::SingleQuoted | Peeked::DoubleQuoted | Peeked::Unquoted => Token::String,
            Peeked::Long | Peeked::Number => Token::Number,
            Peeked::Eof => Token::EndDocument,
            Peeked::None => unreachable!("do_peek never leaves None"),
        })
    }

    fn peeked_or_peek(&mut self) -> Result<Peeked> {
        if self.peeked == Peeked::None {
            self.do_peek()
        } else {
            Ok(self.peeked)
        }
    }

    pub(crate) fn begin_array(&mut self) -> Result<()> {
        if self.peeked_or_peek()? == Peeked::BeginArray {
            self.push(Scope::EmptyArray)?;
            let i = self.stack.len() - 1;
            self.path_indices[i] = 0;
            self.peeked = Peeked::None;
            Ok(())
        } else {
            Err(self.unexpected_token("BEGIN_ARRAY"))
        }
    }

    pub(crate) fn end_array(&mut self) -> Result<()> {
        if self.peeked_or_peek()? == Peeked::EndArray {
            self.stack.pop();
            self.bump_index();
            self.peeked = Peeked::None;
            Ok(())
        } else {
            Err(self.unexpected_token("END_ARRAY"))
        }
    }

    pub(crate) fn begin_object(&mut self) -> Result<()> {
        if self.peeked_or_peek()? == Peeked::BeginObject {
            self.push(Scope::EmptyObject)?;
            self.peeked = Peeked::None;
            Ok(())
        } else {
            Err(self.unexpected_token("BEGIN_OBJECT"))
        }
    }

    pub(crate) fn end_object(&mut self) -> Result<()> {
        if self.peeked_or_peek()? == Peeked::EndObject {
            self.stack.pop();
            let len = self.stack.len();
            self.path_names[len] = None;
            self.bump_index();
            self.peeked = Peeked::None;
            Ok(())
        } else {
            Err(self.unexpected_token("END_OBJECT"))
        }
    }

    pub(crate) fn has_next(&mut self) -> Result<bool> {
        let p = self.peeked_or_peek()?;
        Ok(p != Peeked::EndObject && p != Peeked::EndArray && p != Peeked::Eof)
    }

    pub(crate) fn next_name(&mut self) -> Result<String> {
        let result = match self.peeked_or_peek()? {
            Peeked::UnquotedName => self.next_unquoted_value(),
            Peeked::SingleQuotedName => self.next_quoted_value(b'\'')?,
            Peeked::DoubleQuotedName => self.next_quoted_value(b'"')?,
            _ => return Err(self.unexpected_token("a name")),
        };
        self.peeked = Peeked::None;
        let i = self.stack.len() - 1;
        self.path_names[i] = Some(result.clone());
        Ok(result)
    }

    pub(crate) fn next_string(&mut self) -> Result<String> {
        let result = match self.peeked_or_peek()? {
            Peeked::Unquoted => self.next_unquoted_value(),
            Peeked::SingleQuoted => self.next_quoted_value(b'\'')?,
            Peeked::DoubleQuoted => self.next_quoted_value(b'"')?,
            Peeked::Long => self.peeked_long.to_string(),
            Peeked::Number => {
                let end = self.pos + self.peeked_number_length;
                let text = std::str::from_utf8(&self.buf[self.pos..end])
                    .expect("number literal is ASCII")
                    .to_owned();
                self.pos = end;
                text
            }
            _ => return Err(self.unexpected_token("a string")),
        };
        self.peeked = Peeked::None;
        self.bump_index();
        Ok(result)
    }

    pub(crate) fn next_boolean(&mut self) -> Result<bool> {
        let result = match self.peeked_or_peek()? {
            Peeked::True => true,
            Peeked::False => false,
            _ => return Err(self.unexpected_token("a boolean")),
        };
        self.peeked = Peeked::None;
        self.bump_index();
        Ok(result)
    }

    pub(crate) fn next_null(&mut self) -> Result<()> {
        if self.peeked_or_peek()? == Peeked::Null {
            self.peeked = Peeked::None;
            self.bump_index();
            Ok(())
        } else {
            Err(self.unexpected_token("null"))
        }
    }

    fn do_peek(&mut self) -> Result<Peeked> {
        let peek_stack = self.top();
        match peek_stack {
            Scope::EmptyArray => self.set_top(Scope::NonemptyArray),
            Scope::NonemptyArray => {
                let c = self.next_non_whitespace(true)?;
                match c {
                    Some(b']') => return Ok(self.set_peeked(Peeked::EndArray)),
                    Some(b';') | Some(b',') => {}
                    _ => return Err(self.syntax_error("Unterminated array")),
                }
            }
            Scope::EmptyObject | Scope::NonemptyObject => {
                self.set_top(Scope::DanglingName);
                if peek_stack == Scope::NonemptyObject {
                    let c = self.next_non_whitespace(true)?;
                    match c {
                        Some(b'}') => return Ok(self.set_peeked(Peeked::EndObject)),
                        Some(b';') | Some(b',') => {}
                        _ => return Err(self.syntax_error("Unterminated object")),
                    }
                }
                let c = self.next_non_whitespace(true)?.expect("throwOnEof");
                return match c {
                    b'"' => Ok(self.set_peeked(Peeked::DoubleQuotedName)),
                    b'\'' => Ok(self.set_peeked(Peeked::SingleQuotedName)),
                    b'}' => {
                        if peek_stack != Scope::NonemptyObject {
                            Ok(self.set_peeked(Peeked::EndObject))
                        } else {
                            Err(self.syntax_error("Expected name"))
                        }
                    }
                    _ => {
                        self.pos -= 1;
                        if is_literal(c) {
                            Ok(self.set_peeked(Peeked::UnquotedName))
                        } else {
                            Err(self.syntax_error("Expected name"))
                        }
                    }
                };
            }
            Scope::DanglingName => {
                self.set_top(Scope::NonemptyObject);
                let c = self.next_non_whitespace(true)?;
                match c {
                    Some(b':') => {}
                    Some(b'=') => {
                        if self.pos < self.buf.len() && self.buf[self.pos] == b'>' {
                            self.pos += 1;
                        }
                    }
                    _ => return Err(self.syntax_error("Expected ':'")),
                }
            }
            Scope::EmptyDocument => {
                self.consume_non_execute_prefix()?;
                self.set_top(Scope::NonemptyDocument);
            }
            Scope::NonemptyDocument => {
                let c = self.next_non_whitespace(false)?;
                if c.is_none() {
                    return Ok(self.set_peeked(Peeked::Eof));
                }
                self.pos -= 1;
            }
        }

        let c = self.next_non_whitespace(true)?.expect("throwOnEof");
        match c {
            b']' if peek_stack == Scope::EmptyArray => return Ok(self.set_peeked(Peeked::EndArray)),
            b']' | b';' | b',' => {
                // In an array a missing value is null: "[1,]" has two elements.
                if peek_stack == Scope::EmptyArray || peek_stack == Scope::NonemptyArray {
                    self.pos -= 1;
                    return Ok(self.set_peeked(Peeked::Null));
                }
                return Err(self.syntax_error("Unexpected value"));
            }
            b'\'' => return Ok(self.set_peeked(Peeked::SingleQuoted)),
            b'"' => return Ok(self.set_peeked(Peeked::DoubleQuoted)),
            b'[' => return Ok(self.set_peeked(Peeked::BeginArray)),
            b'{' => return Ok(self.set_peeked(Peeked::BeginObject)),
            _ => self.pos -= 1,
        }

        let result = self.peek_keyword();
        if result != Peeked::None {
            return Ok(result);
        }
        let result = self.peek_number();
        if result != Peeked::None {
            return Ok(result);
        }
        if !is_literal(self.buf[self.pos]) {
            return Err(self.syntax_error("Expected value"));
        }
        Ok(self.set_peeked(Peeked::Unquoted))
    }

    fn set_peeked(&mut self, p: Peeked) -> Peeked {
        self.peeked = p;
        p
    }

    fn peek_keyword(&mut self) -> Peeked {
        let c = self.buf[self.pos];
        let (keyword, upper, peeking): (&[u8], &[u8], Peeked) = match c {
            b't' | b'T' => (b"true", b"TRUE", Peeked::True),
            b'f' | b'F' => (b"false", b"FALSE", Peeked::False),
            b'n' | b'N' => (b"null", b"NULL", Peeked::Null),
            _ => return Peeked::None,
        };
        let length = keyword.len();
        for i in 0..length {
            if self.pos + i >= self.buf.len() {
                return Peeked::None;
            }
            let c = self.buf[self.pos + i];
            if c != keyword[i] && c != upper[i] {
                return Peeked::None;
            }
        }
        if self.pos + length < self.buf.len() && is_literal(self.buf[self.pos + length]) {
            return Peeked::None;
        }
        self.pos += length;
        self.set_peeked(peeking)
    }

    fn peek_number(&mut self) -> Peeked {
        #[derive(PartialEq, Eq, Clone, Copy)]
        enum Last {
            None,
            Sign,
            Digit,
            Decimal,
            FractionDigit,
            ExpE,
            ExpSign,
            ExpDigit,
        }
        // A literal of BUFFER_SIZE characters or more is not a number; it reads as a string.
        let buf = self.buf;
        let p = self.pos;
        let mut value: i64 = 0;
        let mut negative = false;
        let mut fits_in_long = true;
        let mut last = Last::None;
        let mut i = 0;
        loop {
            if p + i == buf.len() {
                if i == BUFFER_SIZE {
                    return Peeked::None;
                }
                break;
            }
            if i == BUFFER_SIZE {
                return Peeked::None;
            }
            let c = buf[p + i];
            match c {
                b'-' => {
                    if last == Last::None {
                        negative = true;
                        last = Last::Sign;
                    } else if last == Last::ExpE {
                        last = Last::ExpSign;
                    } else {
                        return Peeked::None;
                    }
                }
                b'+' => {
                    if last == Last::ExpE {
                        last = Last::ExpSign;
                    } else {
                        return Peeked::None;
                    }
                }
                b'e' | b'E' => {
                    if last == Last::Digit || last == Last::FractionDigit {
                        last = Last::ExpE;
                    } else {
                        return Peeked::None;
                    }
                }
                b'.' => {
                    if last == Last::Digit {
                        last = Last::Decimal;
                    } else {
                        return Peeked::None;
                    }
                }
                _ => {
                    if !c.is_ascii_digit() {
                        if !is_literal(c) {
                            break;
                        }
                        return Peeked::None;
                    }
                    let d = (c - b'0') as i64;
                    match last {
                        Last::Sign | Last::None => {
                            value = -d;
                            last = Last::Digit;
                        }
                        Last::Digit => {
                            if value == 0 {
                                return Peeked::None;
                            }
                            let new_value = value.wrapping_mul(10).wrapping_sub(d);
                            fits_in_long &= value > MIN_INCOMPLETE_INTEGER
                                || (value == MIN_INCOMPLETE_INTEGER && new_value < value);
                            value = new_value;
                        }
                        Last::Decimal => last = Last::FractionDigit,
                        Last::ExpE | Last::ExpSign => last = Last::ExpDigit,
                        Last::FractionDigit | Last::ExpDigit => {}
                    }
                }
            }
            i += 1;
        }

        if last == Last::Digit
            && fits_in_long
            && (value != i64::MIN || negative)
            && (value != 0 || !negative)
        {
            self.peeked_long = if negative { value } else { -value };
            self.pos += i;
            self.set_peeked(Peeked::Long)
        } else if last == Last::Digit || last == Last::FractionDigit || last == Last::ExpDigit {
            self.peeked_number_length = i;
            self.set_peeked(Peeked::Number)
        } else {
            Peeked::None
        }
    }

    fn next_non_whitespace(&mut self, throw_on_eof: bool) -> Result<Option<u8>> {
        let len = self.buf.len();
        let mut p = self.pos;
        loop {
            if p == len {
                self.pos = p;
                break;
            }
            let c = self.buf[p];
            p += 1;
            if c == b'\n' {
                self.line_number += 1;
                self.line_start = p;
                continue;
            } else if c == b' ' || c == b'\r' || c == b'\t' {
                continue;
            }

            if c == b'/' {
                self.pos = p;
                if p == len {
                    return Ok(Some(c));
                }
                match self.buf[self.pos] {
                    b'*' => {
                        self.pos += 1;
                        if !self.skip_to(b"*/") {
                            return Err(self.syntax_error("Unterminated comment"));
                        }
                        p = self.pos + 2;
                        continue;
                    }
                    b'/' => {
                        self.pos += 1;
                        self.skip_to_end_of_line();
                        p = self.pos;
                        continue;
                    }
                    _ => return Ok(Some(c)),
                }
            } else if c == b'#' {
                self.pos = p;
                self.skip_to_end_of_line();
                p = self.pos;
            } else {
                self.pos = p;
                return Ok(Some(c));
            }
        }
        if throw_on_eof {
            Err(ReadError::Eof(format!("End of input{}", self.location())))
        } else {
            Ok(None)
        }
    }

    fn skip_to_end_of_line(&mut self) {
        while self.pos < self.buf.len() {
            let c = self.buf[self.pos];
            self.pos += 1;
            if c == b'\n' {
                self.line_number += 1;
                self.line_start = self.pos;
                break;
            } else if c == b'\r' {
                break;
            }
        }
    }

    fn skip_to(&mut self, to_find: &[u8]) -> bool {
        while self.pos + to_find.len() <= self.buf.len() {
            if self.buf[self.pos] == b'\n' {
                self.line_number += 1;
                self.line_start = self.pos + 1;
                self.pos += 1;
                continue;
            }
            if &self.buf[self.pos..self.pos + to_find.len()] == to_find {
                return true;
            }
            self.pos += 1;
        }
        false
    }

    fn consume_non_execute_prefix(&mut self) -> Result<()> {
        self.next_non_whitespace(true)?;
        self.pos -= 1;
        if self.pos + 5 > self.buf.len() {
            return Ok(());
        }
        if &self.buf[self.pos..self.pos + 5] == b")]}'\n" {
            self.pos += 5;
        }
        Ok(())
    }

    fn next_quoted_value(&mut self, quote: u8) -> Result<String> {
        let mut out = Utf16Builder::default();
        let mut p = self.pos;
        let mut start = p;
        loop {
            if p >= self.buf.len() {
                out.push_bytes(&self.buf[start..p]);
                self.pos = p;
                return Err(self.syntax_error("Unterminated string"));
            }
            let c = self.buf[p];
            p += 1;
            if c == quote {
                self.pos = p;
                out.push_bytes(&self.buf[start..p - 1]);
                return Ok(out.finish());
            } else if c == b'\\' {
                self.pos = p;
                out.push_bytes(&self.buf[start..p - 1]);
                let unit = self.read_escape_character()?;
                out.push_unit(unit);
                p = self.pos;
                start = p;
            } else if c == b'\n' {
                self.line_number += 1;
                self.line_start = p;
            }
        }
    }

    fn next_unquoted_value(&mut self) -> String {
        let mut i = 0;
        while self.pos + i < self.buf.len() {
            if !is_literal(self.buf[self.pos + i]) {
                break;
            }
            i += 1;
        }
        let s = String::from_utf8_lossy(&self.buf[self.pos..self.pos + i]).into_owned();
        self.pos += i;
        s
    }

    fn read_escape_character(&mut self) -> Result<u16> {
        if self.pos == self.buf.len() {
            return Err(self.syntax_error("Unterminated escape sequence"));
        }
        let escaped = self.buf[self.pos];
        self.pos += 1;
        match escaped {
            b'u' => {
                let rest = &self.buf[self.pos..];
                let four: Vec<char> = String::from_utf8_lossy(rest).chars().take(4).collect();
                if four.iter().map(|c| c.len_utf16()).sum::<usize>() < 4 {
                    return Err(self.syntax_error("Unterminated escape sequence"));
                }
                let mut result: u16 = 0;
                for &c in &four {
                    match c.to_digit(16) {
                        Some(d) if c.is_ascii_hexdigit() => result = (result << 4) | d as u16,
                        _ => {
                            let shown: String = four.iter().collect();
                            return Err(
                                self.syntax_error(&format!("Malformed Unicode escape \\u{shown}"))
                            );
                        }
                    }
                }
                self.pos += 4;
                Ok(result)
            }
            b't' => Ok(b'\t' as u16),
            b'b' => Ok(0x08),
            b'n' => Ok(b'\n' as u16),
            b'r' => Ok(b'\r' as u16),
            b'f' => Ok(0x0c),
            b'\n' => {
                self.line_number += 1;
                self.line_start = self.pos;
                Ok(b'\n' as u16)
            }
            b'\'' | b'"' | b'\\' | b'/' => Ok(escaped as u16),
            _ => Err(self.syntax_error("Invalid escape sequence")),
        }
    }
}

fn is_literal(c: u8) -> bool {
    !matches!(
        c,
        b'/' | b'\\'
            | b';'
            | b'#'
            | b'='
            | b'{'
            | b'}'
            | b'['
            | b']'
            | b':'
            | b','
            | b' '
            | b'\t'
            | 0x0c
            | b'\r'
            | b'\n'
    )
}

#[derive(Default)]
struct Utf16Builder {
    out: Vec<u8>,
    pending_high: Option<u16>,
}

impl Utf16Builder {
    fn flush_pending(&mut self) {
        if self.pending_high.take().is_some() {
            self.out.extend_from_slice("\u{fffd}".as_bytes());
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.flush_pending();
        self.out.extend_from_slice(bytes);
    }

    fn push_unit(&mut self, unit: u16) {
        match unit {
            0xD800..=0xDBFF => {
                self.flush_pending();
                self.pending_high = Some(unit);
            }
            0xDC00..=0xDFFF => match self.pending_high.take() {
                Some(high) => {
                    let cp = 0x10000 + (((high as u32) - 0xD800) << 10) + ((unit as u32) - 0xDC00);
                    let c = char::from_u32(cp).expect("valid surrogate pair");
                    let mut tmp = [0u8; 4];
                    self.out
                        .extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
                }
                None => self.out.extend_from_slice("\u{fffd}".as_bytes()),
            },
            _ => {
                self.flush_pending();
                let c = char::from_u32(unit as u32).expect("BMP scalar");
                let mut tmp = [0u8; 4];
                self.out
                    .extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
            }
        }
    }

    fn finish(mut self) -> String {
        self.flush_pending();
        String::from_utf8(self.out)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }
}

pub(crate) fn read_node(reader: &mut Reader<'_>) -> Result<Node> {
    match reader.peek()? {
        Token::BeginArray => {
            reader.begin_array()?;
            let mut items = Vec::new();
            while reader.has_next()? {
                items.push(read_node(reader)?);
            }
            reader.end_array()?;
            Ok(Node::Array(items))
        }
        Token::BeginObject => {
            reader.begin_object()?;
            let mut entries = Vec::new();
            while reader.has_next()? {
                let name = reader.next_name()?;
                let value = read_node(reader)?;
                entries.push((name, value));
            }
            reader.end_object()?;
            Ok(Node::Object(entries))
        }
        Token::String => Ok(Node::String(reader.next_string()?)),
        Token::Number => Ok(Node::Number(reader.next_string()?)),
        Token::Boolean => Ok(Node::Bool(reader.next_boolean()?)),
        Token::Null => {
            reader.next_null()?;
            Ok(Node::Null)
        }
        other => Err(ReadError::IllegalState(format!(
            "Unexpected token: {}",
            other.java_name()
        ))),
    }
}

pub(crate) enum Document {
    Empty,
    Null,
    Value { node: Node, trailing: Trailing },
}

pub(crate) enum Trailing {
    None,
    Value,
    Error(ReadError),
}

impl Reader<'_> {
    fn check_trailing_strict(&mut self) -> Result<()> {
        let len = self.buf.len();
        let mut p = self.pos;
        loop {
            if p == len {
                self.pos = p;
                return Ok(());
            }
            let c = self.buf[p];
            p += 1;
            if c == b'\n' {
                self.line_number += 1;
                self.line_start = p;
            } else if c != b' ' && c != b'\r' && c != b'\t' {
                self.pos = p;
                return Err(self.syntax_error(
                    "Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON",
                ));
            }
        }
    }
}

pub(crate) fn read_document(
    input: &str,
    strict_trailing: bool,
) -> std::result::Result<Document, ReadError> {
    let mut reader = Reader::new(input);
    match reader.peek() {
        Ok(Token::Null) => return Ok(Document::Null),
        Ok(_) => {}
        Err(ReadError::Eof(_)) => return Ok(Document::Empty),
        Err(e) => return Err(e),
    }
    let node = read_node(&mut reader)?;
    if strict_trailing {
        let trailing = match reader.check_trailing_strict() {
            Ok(()) => Trailing::None,
            Err(e) => Trailing::Error(e),
        };
        return Ok(Document::Value { node, trailing });
    }
    let trailing = match reader.peek() {
        Ok(Token::EndDocument) => Trailing::None,
        Ok(_) => Trailing::Value,
        Err(e) => Trailing::Error(e),
    };
    Ok(Document::Value { node, trailing })
}
