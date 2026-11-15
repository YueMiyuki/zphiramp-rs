#![allow(dead_code)]

pub mod server;

use std::path::PathBuf;

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

pub fn read_fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                let cp = u32::from_str_radix(&hex, 16).expect("hex escape");
                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
            }
            other => panic!("bad escape {other:?} in {s:?}"),
        }
    }
    out
}

pub fn read_tsv(name: &str) -> Vec<Vec<String>> {
    read_fixture(name)
        .lines()
        .map(|l| l.split('\t').map(unescape).collect())
        .collect()
}

pub fn from_hex(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "odd hex length");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex digit"))
        .collect()
}

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub struct ZEntry<'a> {
    pub name: &'a str,
    pub method: u16,
    pub data: Vec<u8>,
    pub size: u32,
}

pub fn build_zip(entries: &[ZEntry<'_>]) -> (Vec<u8>, Vec<u32>) {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for e in entries {
        offsets.push(out.len() as u32);
        le32(&mut out, 0x0403_4B50);
        le16(&mut out, 20);
        le16(&mut out, 0);
        le16(&mut out, e.method);
        le32(&mut out, 0);
        le32(&mut out, 0);
        le32(&mut out, e.data.len() as u32);
        le32(&mut out, e.size);
        le16(&mut out, e.name.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(&e.data);
    }
    let directory_start = out.len();
    for (e, offset) in entries.iter().zip(&offsets) {
        le32(&mut out, 0x0201_4B50);
        le16(&mut out, 20);
        le16(&mut out, 20);
        le16(&mut out, 0);
        le16(&mut out, e.method);
        le32(&mut out, 0);
        le32(&mut out, 0);
        le32(&mut out, e.data.len() as u32);
        le32(&mut out, e.size);
        le16(&mut out, e.name.len() as u16);
        le16(&mut out, 0);
        le16(&mut out, 0);
        le16(&mut out, 0);
        le16(&mut out, 0);
        le32(&mut out, 0);
        le32(&mut out, *offset);
        out.extend_from_slice(e.name.as_bytes());
    }
    let directory_size = out.len() - directory_start;
    le32(&mut out, 0x0605_4B50);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, entries.len() as u16);
    le16(&mut out, entries.len() as u16);
    le32(&mut out, directory_size as u32);
    le32(&mut out, directory_start as u32);
    le16(&mut out, 0);
    (out, offsets)
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

pub fn mp3_header(version: u8, layer: u8, bitrate_index: u8, rate_index: u8, mode: u8) -> [u8; 4] {
    [
        0xFF,
        0xE0 | (version << 3) | (layer << 1) | 1,
        (bitrate_index << 4) | (rate_index << 2),
        mode << 6,
    ]
}

pub fn cbr128(len: usize) -> Vec<u8> {
    let mut d = vec![0u8; len];
    d[..4].copy_from_slice(&mp3_header(3, 1, 9, 0, 0));
    d
}

pub fn expected_probe(name: &str) -> Option<i32> {
    match name {
        "zip_deflated_cbr" | "zip_upper_audio_last" | "zip_first_audio_wins" => Some(5),
        "zip_deflated_cbr_192" => Some(3),
        "zip_deflated_ogg" => Some(5),
        "zip_no_audio" => None,
        "zip_stored_cbr" => Some(5),
        "zip_method_other" => None,
        "zip_deflated_xing_be" => Some(235),
        "zip_deflated_id3_cover" => Some(4),
        "zip_deflated_mpeg2" => Some(4),
        "zip_macosx_stub_first" | "zip_macosx_stub_after" | "zip_dot_underscore_ogg" => Some(5),
        "zip_real_cover" => Some(6),
        "bare_wav" => Some(10),
        "bare_cbr_400s" => Some(400),
        "bare_ogg" => Some(6),
        "bare_xing_be" => Some(235),
        "bare_tiny_cbr" => Some(1),
        "bare_garbage" | "bare_three_bytes" | "missing_never_stored" => None,
        "zip_song_second_yml"
        | "zip_song_second_txt"
        | "zip_song_second_both"
        | "zip_song_folder"
        | "zip_song_case_insensitive"
        | "zip_song_quoted_bom_crlf"
        | "zip_song_info_stored"
        | "zip_song_ogg" => Some(5),
        "zip_info_names_wav" => None,
        "zip_info_names_missing_file"
        | "zip_info_no_music"
        | "zip_info_garbage"
        | "zip_no_info_preview_first"
        | "zip_info_too_big" => Some(2),
        other => panic!("no expectation for {other}"),
    }
}

pub fn payload_bytes(payload: &str) -> Option<Vec<u8>> {
    if let Some(file) = payload.strip_prefix("file:") {
        return Some(read_fixture_bytes(file));
    }
    let recipe = payload.strip_prefix("recipe:").expect("file: or recipe:");
    match recipe {
        "cbr128:400" => Some(cbr128(400 * 16_000)),
        "cbr128:tiny200" => Some(cbr128(200)),
        "xing_be_9000:1000000" => {
            let mut d = vec![0u8; 1_000_000];
            d[..4].copy_from_slice(&mp3_header(3, 1, 9, 0, 0));
            d[36..40].copy_from_slice(b"Xing");
            d[40..44].copy_from_slice(&0xFu32.to_be_bytes());
            d[44..48].copy_from_slice(&9000u32.to_be_bytes());
            Some(d)
        }
        "garbage" => Some(b"this is not audio".to_vec()),
        "three_bytes" => Some(vec![1, 2, 3]),
        "404" => None,
        other => panic!("unknown recipe {other}"),
    }
}
