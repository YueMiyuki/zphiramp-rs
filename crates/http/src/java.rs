use std::fmt::Display;

pub(crate) fn is_blank(s: &str) -> bool {
    s.chars().all(is_java_whitespace)
}

pub(crate) fn is_java_whitespace(c: char) -> bool {
    matches!(c, '\u{1c}'..='\u{1f}')
        || (c.is_whitespace() && !matches!(c, '\u{85}' | '\u{a0}' | '\u{2007}' | '\u{202f}'))
}

pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

pub(crate) fn blank_to_null(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !is_blank(v))
}

pub(crate) fn max_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

pub(crate) fn min_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

pub(crate) fn double_to_int(d: f64) -> i32 {
    d as i32
}

pub(crate) fn list_to_string<T: Display>(items: Option<&[T]>) -> String {
    match items {
        None => "null".to_owned(),
        Some(items) => {
            let parts: Vec<String> = items.iter().map(ToString::to_string).collect();
            format!("[{}]", parts.join(", "))
        }
    }
}

pub(crate) fn opt_to_string<T: Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_owned(), |v| v.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Charset {
    Utf8,
    Latin1,
    Ascii,
    Utf16Be,
    Utf16Le,
    Utf16,
}

impl Charset {
    pub(crate) fn for_name(name: &str) -> Option<Charset> {
        let lower = name.to_ascii_lowercase();
        let charset = match lower.as_str() {
            "utf-8" | "utf8" | "unicode-1-1-utf-8" => Charset::Utf8,
            "iso-8859-1" | "iso8859-1" | "iso8859_1" | "iso_8859_1" | "iso_8859-1" | "latin1"
            | "l1" | "8859_1" | "iso-ir-100" | "ibm819" | "cp819" | "csisolatin1" | "819"
            | "ibm-819" | "iso_8859-1:1987" => Charset::Latin1,
            "us-ascii" | "ascii" | "iso646-us" | "default" | "646" | "iso_646.irv:1983"
            | "ansi_x3.4-1968" | "ansi_x3.4-1986" | "us" | "ibm367" | "cp367" | "csascii"
            | "iso_646.irv:1991" => Charset::Ascii,
            "utf-16be" | "utf_16be" | "iso-10646-ucs-2" | "x-utf-16be" | "unicodebigunmarked" => {
                Charset::Utf16Be
            }
            "utf-16le" | "utf_16le" | "x-utf-16le" | "unicodelittleunmarked" => Charset::Utf16Le,
            "utf-16" | "utf_16" | "utf16" | "unicode" => Charset::Utf16,
            _ => return None,
        };
        Some(charset)
    }

    pub(crate) fn decode(self, bytes: &[u8]) -> String {
        match self {
            Charset::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Charset::Latin1 => bytes.iter().map(|&b| char::from(b)).collect(),
            Charset::Ascii => bytes
                .iter()
                .map(|&b| if b < 0x80 { char::from(b) } else { '\u{fffd}' })
                .collect(),
            Charset::Utf16Be => decode_utf16(bytes, true),
            Charset::Utf16Le => decode_utf16(bytes, false),
            Charset::Utf16 => match bytes {
                [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, false),
                [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, true),
                _ => decode_utf16(bytes, true),
            },
        }
    }
}

fn decode_utf16(bytes: &[u8], big_endian: bool) -> String {
    let units = bytes.chunks(2).map(|pair| match pair {
        [a, b] if big_endian => u16::from_be_bytes([*a, *b]),
        [a, b] => u16::from_le_bytes([*a, *b]),
        _ => 0xFFFD,
    });
    char::decode_utf16(units)
        .map(|r| r.unwrap_or('\u{fffd}'))
        .collect()
}

pub(crate) fn url_decode(s: &str, charset: Charset) -> Result<String, UrlDecodeError> {
    if !s.contains(['+', '%']) {
        return Ok(s.to_owned());
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '+' => {
                out.push(' ');
                i += 1;
            }
            '%' => {
                let mut bytes = Vec::new();
                while i + 2 < chars.len() && chars[i] == '%' {
                    let value = parse_hex_pair(chars[i + 1], chars[i + 2])?;
                    bytes.push(value);
                    i += 3;
                }
                if i < chars.len() && chars[i] == '%' {
                    return Err(UrlDecodeError);
                }
                out.push_str(&charset.decode(&bytes));
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UrlDecodeError;

fn parse_hex_pair(a: char, b: char) -> Result<u8, UrlDecodeError> {
    let digit = |c: char| c.to_digit(16).ok_or(UrlDecodeError);
    let value = match a {
        '+' => digit(b)?,
        '-' => {
            if digit(b)? == 0 {
                0
            } else {
                return Err(UrlDecodeError);
            }
        }
        _ => digit(a)? * 16 + digit(b)?,
    };
    u8::try_from(value).map_err(|_| UrlDecodeError)
}

pub(crate) fn path_param_decode(s: &str) -> Result<String, UrlDecodeError> {
    if !s.contains(['+', '%']) {
        return Ok(s.to_owned());
    }
    url_decode(&s.replace('+', "%2B"), Charset::Utf8)
}

pub(crate) fn split_comma(s: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(',').collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_follows_character_is_whitespace() {
        assert!(is_blank(""));
        assert!(is_blank(" \t\r\n\u{1c}\u{3000}\u{2028}"));
        assert!(!is_blank("\u{a0}"));
        assert!(!is_blank("\u{feff}"));
        assert!(!is_blank(" x "));
    }

    #[test]
    fn url_decoder() {
        assert_eq!(url_decode("a+b%20c", Charset::Utf8).unwrap(), "a b c");
        assert_eq!(url_decode("%E4%B8%AD", Charset::Utf8).unwrap(), "中");
        assert_eq!(url_decode("%e4%b8%ad", Charset::Utf8).unwrap(), "中");
        assert_eq!(url_decode("%C3%28", Charset::Utf8).unwrap(), "\u{fffd}(");
        assert_eq!(url_decode("%+F", Charset::Utf8).unwrap(), "\u{f}");
        assert!(url_decode("%zz", Charset::Utf8).is_err());
        assert!(url_decode("abc%2", Charset::Utf8).is_err());
        assert!(url_decode("%", Charset::Utf8).is_err());
        assert!(url_decode("%-1", Charset::Utf8).is_err());
        assert_eq!(url_decode("%E9", Charset::Latin1).unwrap(), "é");
        assert_eq!(path_param_decode("a+b").unwrap(), "a+b");
        assert_eq!(path_param_decode("a%2Bb%20").unwrap(), "a+b ");
    }

    #[test]
    fn split_like_java() {
        assert_eq!(split_comma("a,b"), vec!["a", "b"]);
        assert_eq!(split_comma(",a"), vec!["", "a"]);
        assert_eq!(split_comma("a,,"), vec!["a"]);
        assert!(split_comma(",").is_empty());
        assert_eq!(split_comma("a,,b"), vec!["a", "", "b"]);
    }

    #[test]
    fn double_math() {
        assert!(max_f64(1.0, f64::NAN).is_nan());
        assert!(min_f64(f64::NAN, 500.0).is_nan());
        assert_eq!(double_to_int(f64::NAN), 0);
        assert_eq!(double_to_int(1e300), i32::MAX);
        assert_eq!(double_to_int(-2.9), -2);
    }

    #[test]
    fn lists() {
        assert_eq!(list_to_string(Some(&[1, 2][..])), "[1, 2]");
        assert_eq!(list_to_string::<i32>(None), "null");
        assert_eq!(list_to_string::<i32>(Some(&[])), "[]");
    }
}
