use flate2::{Decompress, FlushDecompress};

use super::ZipEntry;
use super::bytes::{le_i32, le_u16, le_u32};

const EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
const CENTRAL_SIG: i32 = 0x0201_4B50;
pub(crate) const LOCAL_SIG: i32 = 0x0403_4B50;
pub(crate) const ZIP64_SENTINEL: u32 = 0xFFFF_FFFF;

pub const METHOD_STORED: u16 = 0;
pub const METHOD_DEFLATE: u16 = 8;

pub fn find_audio_entry(tail: &[u8]) -> Option<ZipEntry> {
    first_audio_entry(&list_entries(tail))
        .filter(|entry| !entry.needs_zip64())
        .cloned()
}

pub fn list_entries(tail: &[u8]) -> Vec<ZipEntry> {
    let Some(eocd) = last_index_of(tail, &EOCD_SIG) else {
        return Vec::new();
    };
    if eocd + 22 > tail.len() {
        return Vec::new();
    }
    let (Some(directory_size), Some(entries)) = (le_u32(tail, eocd + 12), le_u16(tail, eocd + 10))
    else {
        return Vec::new();
    };
    scan_central_directory(
        tail,
        eocd.saturating_sub(directory_size as usize),
        usize::from(entries),
    )
}

pub fn first_audio_entry(entries: &[ZipEntry]) -> Option<&ZipEntry> {
    entries
        .iter()
        .find(|entry| entry.is_audio() && !is_resource_fork(&entry.name))
}

pub(crate) fn is_resource_fork(name: &str) -> bool {
    let mut segments = name.rsplit(['/', '\\']);
    let file_name = segments.next().unwrap_or("");
    file_name.starts_with("._") || segments.any(|directory| directory == "__macosx")
}

fn scan_central_directory(tail: &[u8], start: usize, entries: usize) -> Vec<ZipEntry> {
    let mut found = Vec::new();
    let mut position = start;
    for _ in 0..entries {
        match read_record(tail, position) {
            Some((entry, next)) => {
                found.push(entry);
                position = next;
            }
            None => break,
        }
    }
    found
}

fn read_record(tail: &[u8], position: usize) -> Option<(ZipEntry, usize)> {
    if position.checked_add(46)? > tail.len() || le_i32(tail, position)? != CENTRAL_SIG {
        return None;
    }
    let method = le_u16(tail, position + 10)?;
    let name_length = usize::from(le_u16(tail, position + 28)?);
    let extra_length = usize::from(le_u16(tail, position + 30)?);
    let comment_length = usize::from(le_u16(tail, position + 32)?);
    let compressed = le_u32(tail, position + 20)?;
    let uncompressed = le_u32(tail, position + 24)?;
    let local_offset = le_u32(tail, position + 42)?;
    let name_start = position.checked_add(46)?;
    let name_bytes = tail.get(name_start..name_start.checked_add(name_length)?)?;
    let path = String::from_utf8_lossy(name_bytes).into_owned();
    let next = name_start
        .checked_add(name_length)?
        .checked_add(extra_length)?
        .checked_add(comment_length)?;
    let entry = ZipEntry {
        name: path.to_lowercase(),
        path,
        local_offset: u64::from(local_offset),
        size: u64::from(uncompressed),
        compressed_size: u64::from(compressed),
        method,
    };
    Some((entry, next))
}

/// `raw` starts at the entry's local header; the output stops within one 8 KiB chunk of `limit`.
pub fn inflate_entry(raw: &[u8], entry: &ZipEntry, limit: usize) -> Vec<u8> {
    let Some(data_start) = data_start(raw) else {
        return Vec::new();
    };
    let data = &raw[data_start..];
    match entry.method {
        METHOD_STORED => {
            let entry_size = usize::try_from(entry.compressed_size).unwrap_or(usize::MAX);
            let wanted = entry_size.min(limit).min(data.len());
            data[..wanted].to_vec()
        }
        METHOD_DEFLATE => inflate_raw(data, limit),
        _ => Vec::new(),
    }
}

fn data_start(raw: &[u8]) -> Option<usize> {
    if raw.len() < 30 || le_i32(raw, 0)? != LOCAL_SIG {
        return None;
    }
    let name_length = usize::from(le_u16(raw, 26)?);
    let extra_length = usize::from(le_u16(raw, 28)?);
    let start = 30 + name_length + extra_length;
    (start < raw.len()).then_some(start)
}

fn inflate_raw(input: &[u8], limit: usize) -> Vec<u8> {
    const CHUNK: usize = 8192;
    let mut decompress = Decompress::new(false);
    let mut out = Vec::new();
    let mut chunk = [0u8; CHUNK];
    let mut remaining = input;
    while out.len() < limit {
        let in_before = decompress.total_in();
        let out_before = decompress.total_out();
        let status = decompress.decompress(remaining, &mut chunk, FlushDecompress::None);
        let consumed = usize::try_from(decompress.total_in() - in_before).unwrap_or(0);
        let produced = usize::try_from(decompress.total_out() - out_before).unwrap_or(0);
        remaining = remaining.get(consumed..).unwrap_or(&[]);
        out.extend_from_slice(&chunk[..produced.min(CHUNK)]);
        match status {
            Ok(flate2::Status::Ok) if produced > 0 => {}
            _ => break,
        }
    }
    out
}

fn last_index_of(data: &[u8], signature: &[u8]) -> Option<usize> {
    if data.len() < signature.len() {
        return None;
    }
    (0..=data.len() - signature.len())
        .rev()
        .find(|&i| &data[i..i + signature.len()] == signature)
}
