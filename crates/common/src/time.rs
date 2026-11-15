use std::cmp::Ordering;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn now_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

fn now_instant() -> (i64, u32) {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos()),
        Err(e) => {
            let d = e.duration();
            let mut secs = -(d.as_secs() as i64);
            let mut nanos = d.subsec_nanos();
            if nanos > 0 {
                secs -= 1;
                nanos = 1_000_000_000 - nanos;
            }
            (secs, nanos)
        }
    }
}

const MAX_YEAR: i64 = 999_999_999;
const MIN_YEAR: i64 = -999_999_999;
const MAX_OFFSET_SECONDS: i32 = 18 * 3600;
const SECONDS_PER_DAY: i64 = 86_400;

/// Parses and prints like Java's `OffsetDateTime`; ordered by instant, then local date-time.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct JavaOffsetDateTime {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    nano: u32,
    offset_seconds: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeParseError {
    message: String,
}

impl DateTimeParseError {
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DateTimeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DateTimeParseError {}

impl JavaOffsetDateTime {
    pub fn parse(text: &str) -> Result<Self, DateTimeParseError> {
        let chars: Vec<char> = text.chars().collect();
        let fail = |message: String| DateTimeParseError { message };
        let fields = match parse_fields(&chars) {
            Ok(f) => f,
            Err(ParseFail::At(i)) => {
                return Err(fail(format!(
                    "Text '{}' could not be parsed at index {i}",
                    abbreviate(text)
                )));
            }
            Err(ParseFail::Unparsed(i)) => {
                return Err(fail(format!(
                    "Text '{}' could not be parsed, unparsed text found at index {i}",
                    abbreviate(text)
                )));
            }
            Err(ParseFail::Exception(msg)) => {
                return Err(fail(format!(
                    "Text '{}' could not be parsed: {msg}",
                    abbreviate(text)
                )));
            }
        };
        fields.resolve().map_err(|msg| {
            fail(format!(
                "Text '{}' could not be parsed: {msg}",
                abbreviate(text)
            ))
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn of(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        nano: u32,
        offset_seconds: i32,
    ) -> Result<Self, DateTimeParseError> {
        let fields = Fields {
            year: year as i64,
            month: month as i64,
            day: day as i64,
            hour: hour as i64,
            minute: minute as i64,
            second: second as i64,
            nano: nano as i64,
            offset: offset_seconds as i64,
        };
        if nano > 999_999_999 {
            return Err(DateTimeParseError {
                message: format!(
                    "Invalid value for NanoOfSecond (valid values 0 - 999999999): {nano}"
                ),
            });
        }
        fields
            .resolve()
            .map_err(|message| DateTimeParseError { message })
    }

    pub fn from_instant(epoch_second: i64, nano: u32, offset_seconds: i32) -> Option<Self> {
        if offset_seconds.abs() > MAX_OFFSET_SECONDS || nano > 999_999_999 {
            return None;
        }
        let local = epoch_second.checked_add(offset_seconds as i64)?;
        let days = local.div_euclid(SECONDS_PER_DAY);
        let secs = local.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
            return None;
        }
        Some(Self {
            year: year as i32,
            month: month as u8,
            day: day as u8,
            hour: (secs / 3600) as u8,
            minute: (secs / 60 % 60) as u8,
            second: (secs % 60) as u8,
            nano,
            offset_seconds,
        })
    }

    pub fn from_epoch_millis(millis: i64, offset_seconds: i32) -> Option<Self> {
        let secs = millis.div_euclid(1000);
        let nano = (millis.rem_euclid(1000) * 1_000_000) as u32;
        Self::from_instant(secs, nano, offset_seconds)
    }

    pub fn now_utc() -> Self {
        let (secs, nano) = now_instant();
        Self::from_instant(secs, nano, 0).expect("system clock within java.time range")
    }

    pub fn now() -> Self {
        let (secs, nano) = now_instant();
        let offset = local_offset_seconds(secs);
        Self::from_instant(secs, nano, offset).expect("system clock within java.time range")
    }

    pub fn year(&self) -> i32 {
        self.year
    }

    pub fn month(&self) -> u32 {
        self.month as u32
    }

    pub fn day(&self) -> u32 {
        self.day as u32
    }

    pub fn hour(&self) -> u32 {
        self.hour as u32
    }

    pub fn minute(&self) -> u32 {
        self.minute as u32
    }

    pub fn second(&self) -> u32 {
        self.second as u32
    }

    pub fn nano(&self) -> u32 {
        self.nano
    }

    pub fn offset_seconds(&self) -> i32 {
        self.offset_seconds
    }

    pub fn epoch_second(&self) -> i64 {
        let days = days_from_civil(self.year as i64, self.month as i64, self.day as i64);
        days * SECONDS_PER_DAY
            + self.hour as i64 * 3600
            + self.minute as i64 * 60
            + self.second as i64
            - self.offset_seconds as i64
    }

    pub fn to_epoch_milli(&self) -> Option<i64> {
        let seconds = self.epoch_second();
        let nanos = self.nano as i64;
        if seconds < 0 && nanos > 0 {
            let millis = (seconds + 1).checked_mul(1000)?;
            millis.checked_add(nanos / 1_000_000 - 1000)
        } else {
            seconds.checked_mul(1000)?.checked_add(nanos / 1_000_000)
        }
    }

    pub fn is_same_instant(&self, other: &Self) -> bool {
        self.epoch_second() == other.epoch_second() && self.nano == other.nano
    }

    fn local_key(&self) -> (i32, u8, u8, u8, u8, u8, u32) {
        (
            self.year,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            self.nano,
        )
    }
}

fn local_offset_seconds(epoch_second: i64) -> i32 {
    use chrono::{Local, Offset, TimeZone};
    match Local.timestamp_opt(epoch_second, 0) {
        chrono::LocalResult::Single(t) => t.offset().fix().local_minus_utc(),
        chrono::LocalResult::Ambiguous(t, _) => t.offset().fix().local_minus_utc(),
        chrono::LocalResult::None => 0,
    }
}

impl Ord for JavaOffsetDateTime {
    fn cmp(&self, other: &Self) -> Ordering {
        let by_instant = if self.offset_seconds == other.offset_seconds {
            self.local_key().cmp(&other.local_key())
        } else {
            (self.epoch_second(), self.nano).cmp(&(other.epoch_second(), other.nano))
        };
        by_instant.then_with(|| self.local_key().cmp(&other.local_key()))
    }
}

impl PartialOrd for JavaOffsetDateTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for JavaOffsetDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let y = self.year;
        if y.unsigned_abs() < 1000 {
            if y < 0 {
                write!(f, "-{:04}", -y)?;
            } else {
                write!(f, "{y:04}")?;
            }
        } else {
            if y > 9999 {
                f.write_str("+")?;
            }
            write!(f, "{y}")?;
        }
        write!(
            f,
            "-{:02}-{:02}T{:02}:{:02}",
            self.month, self.day, self.hour, self.minute
        )?;
        if self.second > 0 || self.nano > 0 {
            write!(f, ":{:02}", self.second)?;
            let n = self.nano;
            if n > 0 {
                if n % 1_000_000 == 0 {
                    write!(f, ".{:03}", n / 1_000_000)?;
                } else if n % 1000 == 0 {
                    write!(f, ".{:06}", n / 1000)?;
                } else {
                    write!(f, ".{n:09}")?;
                }
            }
        }
        let total = self.offset_seconds;
        if total == 0 {
            return f.write_str("Z");
        }
        let abs = total.unsigned_abs();
        write!(
            f,
            "{}{:02}:{:02}",
            if total < 0 { '-' } else { '+' },
            abs / 3600,
            abs / 60 % 60
        )?;
        if abs % 60 != 0 {
            write!(f, ":{:02}", abs % 60)?;
        }
        Ok(())
    }
}

impl fmt::Debug for JavaOffsetDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JavaOffsetDateTime({self})")
    }
}

impl std::str::FromStr for JavaOffsetDateTime {
    type Err = DateTimeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for JavaOffsetDateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for JavaOffsetDateTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = JavaOffsetDateTime;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an ISO offset date-time string")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                JavaOffsetDateTime::parse(v).map_err(|e| E::custom(e.message))
            }
        }
        deserializer.deserialize_str(V)
    }
}

enum ParseFail {
    At(usize),
    Unparsed(usize),
    Exception(String),
}

struct Fields {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    nano: i64,
    offset: i64,
}

fn abbreviate(text: &str) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() > 64 {
        format!("{}...", String::from_utf16_lossy(&units[..64]))
    } else {
        text.to_owned()
    }
}

fn digit(c: char) -> Option<i64> {
    c.to_digit(10).filter(|_| c.is_ascii_digit()).map(i64::from)
}

fn parse_fixed2(text: &[char], position: usize) -> Result<(i64, usize), ParseFail> {
    let len = text.len();
    if position == len {
        return Err(ParseFail::At(position));
    }
    if text[position] == '+' || text[position] == '-' {
        return Err(ParseFail::At(position));
    }
    if position + 2 > len {
        return Err(ParseFail::At(position));
    }
    let mut total = 0;
    for &c in &text[position..position + 2] {
        match digit(c) {
            Some(d) => total = total * 10 + d,
            None => return Err(ParseFail::At(position)),
        }
    }
    Ok((total, position + 2))
}

fn parse_year(text: &[char], mut position: usize) -> Result<(i64, usize), ParseFail> {
    const MIN_WIDTH: usize = 4;
    const MAX_WIDTH: usize = 10;
    let len = text.len();
    if position == len {
        return Err(ParseFail::At(position));
    }
    let mut negative = false;
    let mut positive = false;
    match text[position] {
        '+' => {
            positive = true;
            position += 1;
        }
        '-' => {
            negative = true;
            position += 1;
        }
        _ => {}
    }
    let min_end = position + MIN_WIDTH;
    if min_end > len {
        return Err(ParseFail::At(position));
    }
    let max_end = (position + MAX_WIDTH).min(len);
    let mut pos = position;
    let mut total: i64 = 0;
    while pos < max_end {
        match digit(text[pos]) {
            Some(d) => {
                total = total * 10 + d;
                pos += 1;
            }
            None => {
                if pos < min_end {
                    return Err(ParseFail::At(position));
                }
                break;
            }
        }
    }
    if negative {
        if total == 0 {
            return Err(ParseFail::At(position - 1));
        }
        total = -total;
    } else {
        let parse_len = pos - position;
        if positive {
            if parse_len <= MIN_WIDTH {
                return Err(ParseFail::At(position - 1));
            }
        } else if parse_len > MIN_WIDTH {
            return Err(ParseFail::At(position));
        }
    }
    Ok((total, pos))
}

fn parse_literal(text: &[char], position: usize, literal: char) -> Result<usize, ParseFail> {
    match text.get(position) {
        Some(&c) if c == literal || c.eq_ignore_ascii_case(&literal) => Ok(position + 1),
        _ => Err(ParseFail::At(position)),
    }
}

fn parse_fraction(text: &[char], position: usize) -> (Option<i64>, usize) {
    let len = text.len();
    if position == len || text[position] != '.' {
        return (None, position);
    }
    let start = position + 1;
    let max_end = (start + 9).min(len);
    let mut pos = start;
    let mut total = 0;
    while pos < max_end {
        match digit(text[pos]) {
            Some(d) => {
                total = total * 10 + d;
                pos += 1;
            }
            None => break,
        }
    }
    let count = (pos - start) as u32;
    (Some(total * 10_i64.pow(9 - count)), pos)
}

fn parse_offset(text: &[char], position: usize) -> Result<(i64, usize), ParseFail> {
    let len = text.len();
    if position == len {
        return Err(ParseFail::At(position));
    }
    if text[position].eq_ignore_ascii_case(&'Z') {
        return Ok((0, position + 1));
    }
    let sign = text[position];
    if sign != '+' && sign != '-' {
        return Err(ParseFail::At(position));
    }
    let two_digits = |pos: usize, colon: bool| -> Option<(i64, usize)> {
        let mut p = pos;
        if colon {
            if p + 1 > len || text[p] != ':' {
                return None;
            }
            p += 1;
        }
        if p + 2 > len {
            return None;
        }
        let v = digit(text[p])? * 10 + digit(text[p + 1])?;
        (v <= 59).then_some((v, p + 2))
    };
    let Some((hours, mut pos)) = two_digits(position + 1, false) else {
        return Err(ParseFail::At(position));
    };
    let mut minutes = 0;
    let mut seconds = 0;
    if let Some((m, p)) = two_digits(pos, true) {
        minutes = m;
        pos = p;
        if let Some((s, p)) = two_digits(pos, true) {
            seconds = s;
            pos = p;
        }
    }
    if hours > 23 {
        return Err(ParseFail::Exception(
            "Value out of range: Hour[0-23], Minute[0-59], Second[0-59]".to_owned(),
        ));
    }
    let negative = if sign == '-' { -1 } else { 1 };
    Ok((negative * (hours * 3600 + minutes * 60 + seconds), pos))
}

fn parse_fields(text: &[char]) -> Result<Fields, ParseFail> {
    let (year, pos) = parse_year(text, 0)?;
    let pos = parse_literal(text, pos, '-')?;
    let (month, pos) = parse_fixed2(text, pos)?;
    let pos = parse_literal(text, pos, '-')?;
    let (day, pos) = parse_fixed2(text, pos)?;
    let pos = parse_literal(text, pos, 'T')?;
    let (hour, pos) = parse_fixed2(text, pos)?;
    let pos = parse_literal(text, pos, ':')?;
    let (minute, mut pos) = parse_fixed2(text, pos)?;
    let mut second = 0;
    let mut nano = 0;
    if let Ok(p) = parse_literal(text, pos, ':')
        && let Ok((s, p)) = parse_fixed2(text, p)
    {
        second = s;
        let (fraction, p) = parse_fraction(text, p);
        nano = fraction.unwrap_or(0);
        pos = p;
    }
    let (offset, pos) = parse_offset(text, pos)?;
    if pos < text.len() {
        return Err(ParseFail::Unparsed(pos));
    }
    Ok(Fields {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nano,
        offset,
    })
}

const MONTH_NAMES: [&str; 12] = [
    "JANUARY",
    "FEBRUARY",
    "MARCH",
    "APRIL",
    "MAY",
    "JUNE",
    "JULY",
    "AUGUST",
    "SEPTEMBER",
    "OCTOBER",
    "NOVEMBER",
    "DECEMBER",
];

fn is_leap(year: i64) -> bool {
    (year & 3) == 0 && (year % 100 != 0 || year % 400 == 0)
}

impl Fields {
    fn resolve(&self) -> Result<JavaOffsetDateTime, String> {
        let invalid = |field: &str, range: &str, v: i64| {
            format!("Invalid value for {field} (valid values {range}): {v}")
        };
        if !(MIN_YEAR..=MAX_YEAR).contains(&self.year) {
            return Err(invalid("Year", "-999999999 - 999999999", self.year));
        }
        if !(1..=12).contains(&self.month) {
            return Err(invalid("MonthOfYear", "1 - 12", self.month));
        }
        if !(1..=31).contains(&self.day) {
            return Err(invalid("DayOfMonth", "1 - 28/31", self.day));
        }
        if self.day > 28 {
            let month_len = match self.month {
                2 => {
                    if is_leap(self.year) {
                        29
                    } else {
                        28
                    }
                }
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            if self.day > month_len {
                return Err(if self.day == 29 {
                    format!(
                        "Invalid date 'February 29' as '{}' is not a leap year",
                        self.year
                    )
                } else {
                    format!(
                        "Invalid date '{} {}'",
                        MONTH_NAMES[self.month as usize - 1],
                        self.day
                    )
                });
            }
        }
        if !(0..=59).contains(&self.minute) {
            return Err(invalid("MinuteOfHour", "0 - 59", self.minute));
        }
        if !(0..=23).contains(&self.hour) {
            return Err(invalid("HourOfDay", "0 - 23", self.hour));
        }
        if !(0..=59).contains(&self.second) {
            return Err(invalid("SecondOfMinute", "0 - 59", self.second));
        }
        if self.offset.abs() > MAX_OFFSET_SECONDS as i64 {
            return Err("Zone offset not in valid range: -18:00 to +18:00".to_owned());
        }
        Ok(JavaOffsetDateTime {
            year: self.year as i32,
            month: self.month as u8,
            day: self.day as u8,
            hour: self.hour as u8,
            minute: self.minute as u8,
            second: self.second as u8,
            nano: self.nano as u32,
            offset_seconds: self.offset as i32,
        })
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_basic() {
        let t = JavaOffsetDateTime::parse("2026-11-14T15:26:46.1234Z").unwrap();
        assert_eq!(t.to_string(), "2026-11-14T15:26:46.123400Z");
        assert_eq!(t.to_epoch_milli(), Some(1_794_670_006_123));
        let t = JavaOffsetDateTime::parse("2026-11-14T15:26:00+08:00").unwrap();
        assert_eq!(t.to_string(), "2026-11-14T15:26+08:00");
        assert_eq!(t.offset_seconds(), 28_800);
    }

    #[test]
    fn civil_round_trip() {
        for days in [
            -1_000_000_i64,
            -719_468,
            -1,
            0,
            1,
            59,
            60,
            365,
            10_957,
            2_000_000,
        ] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn now_is_sane() {
        let a = JavaOffsetDateTime::now_utc();
        assert_eq!(a.offset_seconds(), 0);
        let b = JavaOffsetDateTime::now();
        assert!(b.to_epoch_milli().unwrap() >= a.to_epoch_milli().unwrap());
        assert!((now_millis() - a.to_epoch_milli().unwrap()).abs() < 60_000);
    }
}
