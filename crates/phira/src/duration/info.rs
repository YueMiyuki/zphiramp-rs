use super::zip::is_resource_fork;
use super::{METHOD_DEFLATE, METHOD_STORED, ZipEntry, first_audio_entry, inflate_entry};

pub const INFO_MAX_BYTES: u64 = 64 * 1024;

// The central directory does not state the local extra field length, so the range allows for it.
const LOCAL_HEADER_ALLOWANCE: usize = 30 + 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoFormat {
    Yml,
    Txt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoFile {
    pub entry: ZipEntry,
    pub format: InfoFormat,
}

pub fn find_info_file(entries: &[ZipEntry]) -> Option<InfoFile> {
    let folder = single_top_folder(entries);
    for (format, file_name) in [(InfoFormat::Yml, "info.yml"), (InfoFormat::Txt, "info.txt")] {
        let prefixes = [Some(String::new()), folder.clone()];
        for prefix in prefixes.into_iter().flatten() {
            let wanted = format!("{prefix}{file_name}");
            if let Some(entry) = entries
                .iter()
                .find(|e| e.name == wanted && !e.is_directory() && !is_resource_fork(&e.name))
            {
                return Some(InfoFile {
                    entry: entry.clone(),
                    format,
                });
            }
        }
    }
    None
}

fn single_top_folder(entries: &[ZipEntry]) -> Option<String> {
    let mut folder: Option<&str> = None;
    for entry in entries.iter().filter(|e| !is_resource_fork(&e.name)) {
        let Some((first, _)) = entry.name.split_once('/') else {
            continue;
        };
        if first.is_empty() {
            continue;
        }
        match folder {
            None => folder = Some(first),
            Some(known) if known == first => {}
            Some(_) => return None,
        }
    }
    folder.map(|f| format!("{f}/"))
}

/// The `(offset, length)` of one range request covering the info file's local header and data.
pub fn info_music_range(info: &InfoFile) -> Option<(u64, usize)> {
    let entry = &info.entry;
    let readable = !entry.needs_zip64()
        && entry.size > 0
        && entry.size <= INFO_MAX_BYTES
        && entry.compressed_size <= INFO_MAX_BYTES
        && matches!(entry.method, METHOD_STORED | METHOD_DEFLATE);
    if !readable {
        return None;
    }
    let compressed = usize::try_from(entry.compressed_size).ok()?;
    let length = LOCAL_HEADER_ALLOWANCE
        .checked_add(entry.path.len())?
        .checked_add(compressed)?;
    Some((entry.local_offset, length))
}

pub fn read_info_music(info: &InfoFile, raw: &[u8]) -> Option<String> {
    let size = usize::try_from(info.entry.size).ok()?;
    if info.entry.size > INFO_MAX_BYTES {
        return None;
    }
    let mut text = inflate_entry(raw, &info.entry, size);
    if text.len() < size {
        return None;
    }
    text.truncate(size);
    parse_info_music(info.format, &text)
}

pub fn parse_info_music(format: InfoFormat, bytes: &[u8]) -> Option<String> {
    match format {
        InfoFormat::Yml => parse_info_yml_music(bytes),
        InfoFormat::Txt => parse_info_txt_music(bytes),
    }
}

fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

pub fn parse_info_yml_music(bytes: &[u8]) -> Option<String> {
    let text = decode(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        let Some(rest) = yaml_key_rest(line) else {
            continue;
        };
        let mut value = rest.trim();
        if value.is_empty() || value.starts_with('#') {
            value = lines[index + 1..]
                .iter()
                .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .filter(|l| l.starts_with([' ', '\t']))
                .map_or("", |l| l.trim());
        }
        found = yaml_scalar(value);
    }
    found.filter(|music| !music.is_empty())
}

fn yaml_key_rest(line: &str) -> Option<&str> {
    for key in ["music", "\"music\"", "'music'"] {
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim_start_matches([' ', '\t']);
            if let Some(after) = rest.strip_prefix(':')
                && (after.is_empty() || after.starts_with([' ', '\t']))
            {
                return Some(after);
            }
        }
    }
    None
}

fn yaml_scalar(value: &str) -> Option<String> {
    let value = value.trim();
    match value.chars().next()? {
        '"' => Some(double_quoted(&value[1..])),
        '\'' => Some(single_quoted(&value[1..])),
        '|' | '>' | '[' | '{' | '&' | '*' | '!' | '#' => None,
        _ => {
            let end = value
                .find(" #")
                .or_else(|| value.find("\t#"))
                .unwrap_or(value.len());
            Some(value[..end].trim_end().to_owned())
        }
    }
}

fn double_quoted(rest: &str) -> String {
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('0') => out.push('\0'),
                Some(digits @ ('x' | 'u' | 'U')) => {
                    let count = match digits {
                        'x' => 2,
                        'u' => 4,
                        _ => 8,
                    };
                    let code: String = chars.by_ref().take(count).collect();
                    if code.chars().count() == count
                        && let Some(ch) =
                            u32::from_str_radix(&code, 16).ok().and_then(char::from_u32)
                    {
                        out.push(ch);
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }
    out
}

fn single_quoted(rest: &str) -> String {
    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                out.push('\'');
            } else {
                break;
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse_info_txt_music(bytes: &[u8]) -> Option<String> {
    let text = decode(bytes);
    let mut found = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.eq_ignore_ascii_case("music") || key.eq_ignore_ascii_case("song") {
            found = Some(value.trim().to_owned());
        }
    }
    found.filter(|music| !music.is_empty())
}

/// Matches the path exactly first, then in any case, relative to the info file's folder.
pub fn music_entry(entries: &[ZipEntry], info: &InfoFile, music: &str) -> Option<ZipEntry> {
    let relative = normalize_music_path(music)?;
    let folder_end = info.entry.path.rfind('/').map_or(0, |i| i + 1);
    let joined = format!("{}{relative}", &info.entry.path[..folder_end]);
    let candidates = || entries.iter().filter(|e| !e.is_directory());
    candidates()
        .find(|e| e.path == joined)
        .or_else(|| {
            let lower = joined.to_lowercase();
            candidates().find(|e| e.name == lower)
        })
        .cloned()
}

fn normalize_music_path(music: &str) -> Option<String> {
    let music = music.trim().replace('\\', "/");
    if music.starts_with('/') || music.ends_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in music.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SongChoice {
    Entry(ZipEntry),
    Unsupported(ZipEntry),
    NoAudio,
}

/// The named file when the archive holds it, otherwise the first real audio entry.
pub fn select_song(
    entries: &[ZipEntry],
    info: Option<&InfoFile>,
    music: Option<&str>,
) -> SongChoice {
    if let (Some(info), Some(music)) = (info, music)
        && let Some(entry) = music_entry(entries, info, music)
    {
        return if entry.is_audio() {
            SongChoice::Entry(entry)
        } else {
            SongChoice::Unsupported(entry)
        };
    }
    first_audio_entry(entries).map_or(SongChoice::NoAudio, |entry| {
        SongChoice::Entry(entry.clone())
    })
}
