mod support;

use std::io::Read;

use support::{
    ZEntry, build_zip, cbr128, deflate, expected_probe, payload_bytes, read_fixture_bytes, read_tsv,
};
use zphira_phira::duration::{
    AUDIO_FULL_BYTES, AUDIO_PROBE_BYTES, METHOD_DEFLATE, METHOD_STORED, SongChoice, TAIL_BYTES,
    ZipEntry, find_audio_entry, find_info_file, first_audio_entry, inflate_entry, info_music_range,
    is_zip, list_entries, parse_audio_entry, parse_bare_audio, read_info_music, select_song,
};

fn probe_in_memory(file: &[u8]) -> Option<i32> {
    let head = &file[..file.len().min(AUDIO_PROBE_BYTES)];
    if !is_zip(head) {
        return parse_bare_audio(head, file.len() as u64);
    }
    let tail = &file[file.len().saturating_sub(TAIL_BYTES)..];
    let entries = list_entries(tail);
    let info = find_info_file(&entries).filter(|_| first_audio_entry(&entries).is_some());
    let music = info.as_ref().and_then(|info| {
        let (start, len) = info_music_range(info)?;
        let start = usize::try_from(start).ok()?.min(file.len());
        read_info_music(info, &file[start..file.len().min(start + len)])
    });
    let SongChoice::Entry(entry) = select_song(&entries, info.as_ref(), music.as_deref()) else {
        return None;
    };
    if entry.needs_zip64() {
        return None;
    }
    for window in [AUDIO_PROBE_BYTES, AUDIO_FULL_BYTES] {
        let start = usize::try_from(entry.local_offset).ok()?.min(file.len());
        let raw = &file[start..file.len().min(start + window + 1)];
        let audio = inflate_entry(raw, &entry, window);
        if let Some(seconds) = parse_audio_entry(&entry, &audio) {
            return Some(seconds);
        }
    }
    None
}

#[test]
fn central_directory_of_real_archives() {
    type Expected = (&'static str, u16, u64);
    let cases: &[(&str, Option<Expected>)] = &[
        (
            "zip_deflated_cbr.zip",
            Some(("song.mp3", METHOD_DEFLATE, 80_000)),
        ),
        (
            "zip_deflated_cbr_192.zip",
            Some(("song.mp3", METHOD_DEFLATE, 72_000)),
        ),
        (
            "zip_deflated_ogg.zip",
            Some(("song.ogg", METHOD_DEFLATE, 60_000)),
        ),
        (
            "zip_upper_audio_last.zip",
            Some(("music.mp3", METHOD_DEFLATE, 80_000)),
        ),
        (
            "zip_first_audio_wins.zip",
            Some(("a.ogg", METHOD_DEFLATE, 60_000)),
        ),
        (
            "zip_stored_cbr.zip",
            Some(("song.mp3", METHOD_STORED, 80_000)),
        ),
        ("zip_method_other.zip", Some(("song.mp3", 12, 80_000))),
        ("zip_no_audio.zip", None),
    ];
    for (name, expected) in cases {
        let zip = read_fixture_bytes(name);
        assert!(is_zip(&zip), "{name}");
        let entry = find_audio_entry(&zip);
        match (expected, entry) {
            (None, None) => {}
            (Some((entry_name, method, size)), Some(entry)) => {
                assert_eq!(entry.name, *entry_name, "{name}");
                assert_eq!(entry.method, *method, "{name}");
                assert_eq!(entry.size, *size, "{name}");
                if *method == METHOD_STORED {
                    assert_eq!(entry.compressed_size, *size, "{name}");
                }
                let at = usize::try_from(entry.local_offset).unwrap();
                assert_eq!(&zip[at..at + 4], b"PK\x03\x04", "{name}");
            }
            (expected, entry) => panic!("{name}: expected {expected:?}, found {entry:?}"),
        }
    }
}

#[test]
fn central_directory_from_a_tail_window() {
    let zip = read_fixture_bytes("zip_deflated_cbr.zip");
    let whole = find_audio_entry(&zip).expect("whole archive");
    let eocd = zip.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
    let directory_size = u32::from_le_bytes(zip[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
    for extra in [0usize, 1, 100, 10_000] {
        let tail = &zip[zip.len() - (directory_size + 22 + extra)..];
        assert_eq!(find_audio_entry(tail), Some(whole.clone()), "extra {extra}");
    }
    let short = &zip[zip.len() - (directory_size + 22 - 1)..];
    assert_eq!(find_audio_entry(short), None);
}

#[test]
fn inflate_matches_an_independent_decoder() {
    for name in [
        "zip_deflated_cbr.zip",
        "zip_deflated_cbr_192.zip",
        "zip_deflated_id3_cover.zip",
        "zip_deflated_ogg.zip",
    ] {
        let zip = read_fixture_bytes(name);
        let entry = find_audio_entry(&zip).expect("entry");
        let at = usize::try_from(entry.local_offset).unwrap();
        let name_len = usize::from(u16::from_le_bytes([zip[at + 26], zip[at + 27]]));
        let extra_len = usize::from(u16::from_le_bytes([zip[at + 28], zip[at + 29]]));
        let data_start = at + 30 + name_len + extra_len;
        let compressed =
            &zip[data_start..data_start + usize::try_from(entry.compressed_size).unwrap()];
        let mut full = Vec::new();
        flate2::read::DeflateDecoder::new(compressed)
            .read_to_end(&mut full)
            .unwrap();
        assert_eq!(full.len() as u64, entry.size, "{name}: independent decode");

        for window in [AUDIO_PROBE_BYTES, AUDIO_FULL_BYTES] {
            let raw = &zip[at..zip.len().min(at + window + 1)];
            let out = inflate_entry(raw, &entry, window);
            assert!(out.len() < window + 8192, "{name}: {} bytes", out.len());
            let expected = full.len().min(window).min(out.len());
            assert_eq!(out[..expected], full[..expected], "{name}: prefix");
            if full.len() >= window && raw.len() > window / 2 {
                assert!(
                    out.len() >= window.min(full.len()) / 2,
                    "{name}: {} bytes",
                    out.len()
                );
            }
            if full.len() <= window {
                assert_eq!(out, full, "{name}: the whole entry fits the window");
            }
        }
    }
}

#[test]
fn a14_stored_entries_are_read_directly() {
    let zip = read_fixture_bytes("zip_stored_cbr.zip");
    let entry = find_audio_entry(&zip).unwrap();
    assert_eq!(entry.method, METHOD_STORED);
    let at = usize::try_from(entry.local_offset).unwrap();
    let out = inflate_entry(&zip[at..], &entry, AUDIO_PROBE_BYTES);
    assert_eq!(out.len(), AUDIO_PROBE_BYTES, "limited to the window");
    assert_eq!(&out[..4], &[0xFF, 0xFB, 0x90, 0x00]);
    let out = inflate_entry(&zip[at..], &entry, AUDIO_FULL_BYTES);
    assert_eq!(out.len(), 80_000);
    assert_eq!(parse_audio_entry(&entry, &out), Some(5));

    let out = inflate_entry(&zip[at..at + 1000], &entry, AUDIO_PROBE_BYTES);
    assert_eq!(out.len(), 1000 - 30 - "song.mp3".len());

    let mut hostile = entry.clone();
    hostile.compressed_size = u64::MAX;
    assert_eq!(inflate_entry(&zip[at..], &hostile, 1000).len(), 1000);
    hostile.compressed_size = 0;
    assert!(inflate_entry(&zip[at..], &hostile, 1000).is_empty());

    let zip = read_fixture_bytes("zip_method_other.zip");
    let entry = find_audio_entry(&zip).unwrap();
    let at = usize::try_from(entry.local_offset).unwrap();
    assert_eq!(entry.method, 12);
    assert!(inflate_entry(&zip[at..], &entry, AUDIO_PROBE_BYTES).is_empty());
}

#[test]
fn a14_stored_and_deflated_agree() {
    let mp3 = cbr128(5 * 16_000);
    let name = "chart.mp3";
    let (stored, _) = build_zip(&[ZEntry {
        name,
        method: METHOD_STORED,
        data: mp3.clone(),
        size: mp3.len() as u32,
    }]);
    let (deflated, _) = build_zip(&[ZEntry {
        name,
        method: METHOD_DEFLATE,
        data: deflate(&mp3),
        size: mp3.len() as u32,
    }]);
    assert_eq!(probe_in_memory(&stored), Some(5));
    assert_eq!(probe_in_memory(&deflated), Some(5));
}

#[test]
fn inflate_is_bounded_against_a_decompression_bomb() {
    let bomb = deflate(&vec![0u8; 64 * 1024 * 1024]);
    assert!(bomb.len() < 200 * 1024, "{}", bomb.len());
    let (zip, offsets) = build_zip(&[ZEntry {
        name: "bomb.mp3",
        method: METHOD_DEFLATE,
        data: bomb,
        size: 64 * 1024 * 1024,
    }]);
    let entry = find_audio_entry(&zip).unwrap();
    let at = offsets[0] as usize;
    for limit in [0usize, 1, 100, AUDIO_PROBE_BYTES, AUDIO_FULL_BYTES] {
        let out = inflate_entry(&zip[at..], &entry, limit);
        assert!(
            out.len() < limit + 8192,
            "limit {limit}: {} bytes",
            out.len()
        );
        assert!(
            out.len() >= limit.min(64 * 1024 * 1024),
            "limit {limit}: {} bytes",
            out.len()
        );
    }
}

#[test]
fn inflate_keeps_what_was_decoded_before_a_corrupt_block() {
    let data: Vec<u8> = (0..100_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    let mut compressed = deflate(&data);
    let cut = compressed.len() / 2;
    let name = "a.mp3";
    let header_len = 30 + name.len();

    let (zip, offsets) = build_zip(&[ZEntry {
        name,
        method: METHOD_DEFLATE,
        data: compressed.clone(),
        size: data.len() as u32,
    }]);
    let entry = find_audio_entry(&zip).unwrap();
    let at = offsets[0] as usize;
    let out = inflate_entry(&zip[at..at + header_len + cut], &entry, AUDIO_FULL_BYTES);
    assert!(!out.is_empty() && out.len() < data.len());
    assert_eq!(out[..], data[..out.len()]);

    let len = compressed.len();
    for i in cut..cut + 64 {
        compressed[i % len] ^= 0xA5;
    }
    let (zip, offsets) = build_zip(&[ZEntry {
        name,
        method: METHOD_DEFLATE,
        data: compressed,
        size: data.len() as u32,
    }]);
    let entry = find_audio_entry(&zip).unwrap();
    let out = inflate_entry(&zip[offsets[0] as usize..], &entry, AUDIO_FULL_BYTES);
    assert!(out.len() < AUDIO_FULL_BYTES + 8192);
    let good = out.iter().zip(&data).take_while(|(a, b)| a == b).count();
    assert!(good > 1000, "the intact prefix is kept: {good}");
}

#[test]
fn local_header_problems_give_nothing() {
    let entry = ZipEntry {
        name: "a.mp3".into(),
        path: "a.mp3".into(),
        local_offset: 0,
        size: 10,
        compressed_size: 10,
        method: METHOD_STORED,
    };
    assert!(inflate_entry(&[], &entry, 100).is_empty());
    assert!(inflate_entry(b"PK\x03\x04", &entry, 100).is_empty());
    assert!(inflate_entry(&[0u8; 100], &entry, 100).is_empty());
    let mut raw = vec![0u8; 100];
    raw[..4].copy_from_slice(b"PK\x03\x04");
    raw[26..28].copy_from_slice(&0xFFFFu16.to_le_bytes());
    raw[28..30].copy_from_slice(&0xFFFFu16.to_le_bytes());
    assert!(inflate_entry(&raw, &entry, 100).is_empty());
    raw[26..28].copy_from_slice(&70u16.to_le_bytes());
    raw[28..30].copy_from_slice(&0u16.to_le_bytes());
    assert!(inflate_entry(&raw, &entry, 100).is_empty());
    raw[26..28].copy_from_slice(&69u16.to_le_bytes());
    assert_eq!(inflate_entry(&raw, &entry, 100).len(), 1);
}

#[test]
fn central_directory_edge_cases() {
    let (zip, _) = build_zip(&[
        ZEntry {
            name: "info.yml",
            method: METHOD_STORED,
            data: b"x".to_vec(),
            size: 1,
        },
        ZEntry {
            name: "dir/Song.MP3",
            method: METHOD_STORED,
            data: cbr128(100),
            size: 100,
        },
    ]);
    let entry = find_audio_entry(&zip).unwrap();
    assert_eq!(entry.name, "dir/song.mp3");
    assert_eq!(entry.size, 100);

    assert_eq!(find_audio_entry(&[]), None);
    assert_eq!(find_audio_entry(b"PK\x05\x06"), None);
    assert_eq!(find_audio_entry(&zip[..zip.len() - 1]), None);
    assert_eq!(find_audio_entry(&zip[..zip.len() - 22]), None);

    let mut claims = zip.clone();
    let eocd = claims.len() - 22;
    claims[eocd + 10..eocd + 12].copy_from_slice(&0xFFFFu16.to_le_bytes());
    assert_eq!(find_audio_entry(&claims).unwrap().name, "dir/song.mp3");

    let mut none = zip.clone();
    none[eocd + 10..eocd + 12].copy_from_slice(&0u16.to_le_bytes());
    assert_eq!(find_audio_entry(&none), None);

    for size in [u32::MAX, 0x8000_0000, 100_000] {
        let mut huge = zip.clone();
        huge[eocd + 12..eocd + 16].copy_from_slice(&size.to_le_bytes());
        assert_eq!(find_audio_entry(&huge), None, "size {size:#x}");
    }

    let mut long_name = zip.clone();
    let directory =
        eocd - u32::from_le_bytes(zip[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
    long_name[directory + 28..directory + 30].copy_from_slice(&0xFFFFu16.to_le_bytes());
    assert_eq!(find_audio_entry(&long_name), None);

    let mut bad_sig = zip.clone();
    bad_sig[directory] = b'X';
    assert_eq!(find_audio_entry(&bad_sig), None);
}

#[test]
fn zip64_entries_are_a_failed_probe() {
    let (mut zip, _) = build_zip(&[ZEntry {
        name: "a.mp3",
        method: METHOD_STORED,
        data: cbr128(100),
        size: 100,
    }]);
    let eocd = zip.len() - 22;
    let directory =
        eocd - u32::from_le_bytes(zip[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
    for field in [20usize, 24, 42] {
        let mut z = zip.clone();
        z[directory + field..directory + field + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(find_audio_entry(&z), None, "field at {field}");
    }
    assert!(find_audio_entry(&zip).is_some());
    zip.clear();
}

#[test]
fn pipeline_matches_java_probe() {
    let rows = read_tsv("duration_probe.tsv");
    assert!(rows.len() >= 19, "fixture looks truncated");
    let mut compared = 0;
    for row in rows {
        let [name, category, payload, java] = <[String; 4]>::try_from(row).expect("4 columns");
        let Some(file) = payload_bytes(&payload) else {
            continue;
        };
        let rust = probe_in_memory(&file);
        let expected = expected_probe(&name);
        assert_eq!(rust, expected, "{name} ({category}); Java said {java}");
        if category == "same" || category == "exc" {
            assert_eq!(
                java,
                expected.map_or("null".to_owned(), |v| v.to_string()),
                "{name}"
            );
        } else {
            assert_ne!(
                java,
                expected.map_or("null".to_owned(), |v| v.to_string()),
                "{name}: a finding differs from Java"
            );
        }
        compared += 1;
    }
    assert!(compared >= 18);
}

#[test]
fn n31_resource_fork_stubs_are_skipped() {
    fn entry(name: &str) -> ZEntry<'_> {
        ZEntry {
            name,
            method: METHOD_STORED,
            data: cbr128(100),
            size: 100,
        }
    }
    let find = |names: &[&str]| -> Option<String> {
        let entries: Vec<ZEntry<'_>> = names.iter().map(|n| entry(n)).collect();
        let (zip, _) = build_zip(&entries);
        find_audio_entry(&zip).map(|e| e.name)
    };
    for stub in [
        "__MACOSX/._song.mp3",
        "__macosx/._song.mp3",
        "__MACOSX/song.mp3",
        "__MACOSX/sub/dir/song.ogg",
        "dir/__MACOSX/song.mp3",
        "._song.mp3",
        "._SONG.OGG",
        "dir/._song.mp3",
        "dir\\._song.mp3",
        "__MACOSX\\song.mp3",
    ] {
        assert_eq!(
            find(&[stub, "song.mp3"]).as_deref(),
            Some("song.mp3"),
            "{stub}"
        );
        assert_eq!(
            find(&["info.yml", stub, "dir/real.ogg", "song.mp3"]).as_deref(),
            Some("dir/real.ogg"),
            "{stub}"
        );
        assert_eq!(find(&[stub, "info.yml"]), None, "{stub}");
    }
    for real in [
        "my._song.mp3",
        "song._mp3.mp3",
        "__MACOSXX/song.mp3",
        "x__macosx/song.mp3",
        "dir/._/song.mp3",
        "_.mp3",
        "._.x/a.mp3",
        "a/.ogg",
    ] {
        let expected = real.to_lowercase();
        assert_eq!(find(&[real]).as_deref(), Some(expected.as_str()), "{real}");
    }
    assert_eq!(
        find(&[
            "__MACOSX/._a.mp3",
            "._b.ogg",
            "__MACOSX/._c.mp3",
            "c.OGG",
            "d.mp3"
        ])
        .as_deref(),
        Some("c.ogg")
    );
    assert_eq!(
        find(&["song.mp3", "__MACOSX/._song.mp3"]).as_deref(),
        Some("song.mp3")
    );
}
