mod support;

use std::sync::mpsc;
use std::time::Duration;

use support::{ZEntry, build_zip, deflate, from_hex, read_fixture_bytes, read_tsv};
use zphira_phira::duration::{
    AUDIO_PROBE_BYTES, INFO_MAX_BYTES, InfoFile, InfoFormat, METHOD_DEFLATE, METHOD_STORED,
    ZipEntry, find_audio_entry, find_info_file, inflate_entry, info_music_range, is_zip,
    list_entries, music_entry, parse_audio_entry, parse_bare_audio, parse_info_music,
    parse_mp3_duration, parse_ogg_duration, parse_wav_duration, read_info_music, select_song,
};

fn bounded<F: FnOnce() + Send + 'static>(limit: Duration, body: F) {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        body();
        let _ = tx.send(());
    });
    match rx.recv_timeout(limit) {
        Ok(()) => handle.join().expect("fuzz body panicked"),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            // the worker panicked: re-raise its panic
            match handle.join() {
                Err(panic) => std::panic::resume_unwind(panic),
                Ok(()) => panic!("worker ended without reporting"),
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("fuzz body still running after {limit:?}"),
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const INTERESTING: [u8; 8] = [0x00, 0xFF, 0x7F, 0x80, 0x01, 0xFE, 0x50, 0x4B];
const DECLARED: [u64; 6] = [0, 1, 4096, u32::MAX as u64, u64::MAX / 8, u64::MAX];

fn mutate(rng: &mut Rng, data: &[u8]) -> Vec<u8> {
    let mut d = data.to_vec();
    if d.is_empty() {
        return d;
    }
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(d.len());
        match rng.below(6) {
            0 => d[at] ^= 1 << rng.below(8),
            1 => d[at] = INTERESTING[rng.below(INTERESTING.len())],
            2 => d[at] = rng.next() as u8,
            3 => {
                if at + 2 <= d.len() {
                    d[at] = 0xFF;
                    d[at + 1] = 0xE0 | (rng.next() as u8 & 0x1F);
                }
            }
            4 => {
                if at + 4 <= d.len() {
                    let v: u32 =
                        [0, 1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0x7FFF_FFF0][rng.below(6)];
                    d[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            _ => {
                let cut = rng.below(d.len() + 1);
                d.truncate(cut);
                if d.is_empty() {
                    break;
                }
            }
        }
    }
    d
}

fn exercise(data: &[u8]) {
    for declared in DECLARED {
        for result in [
            parse_mp3_duration(data, declared),
            parse_ogg_duration(data, declared),
            parse_bare_audio(data, declared),
        ] {
            assert!(result.is_none_or(|v| v >= 0), "negative duration");
        }
    }
    assert!(parse_wav_duration(data).is_none_or(|v| v >= 0));
    let _ = is_zip(data);
    let entries = list_entries(data);
    if let Some(info) = find_info_file(&entries) {
        let _ = info_music_range(&info);
        let at = usize::try_from(info.entry.local_offset)
            .unwrap_or(usize::MAX)
            .min(data.len());
        let music = read_info_music(&info, &data[at..]);
        let _ = music_entry(&entries, &info, music.as_deref().unwrap_or("song.mp3"));
        let _ = select_song(&entries, Some(&info), music.as_deref());
    }
    let _ = select_song(&entries, None, None);
    if let Some(entry) = find_audio_entry(data) {
        for limit in [0usize, 1, 1000, AUDIO_PROBE_BYTES] {
            let out = inflate_entry(data, &entry, limit);
            assert!(
                out.len() < limit + 8192,
                "{} bytes for limit {limit}",
                out.len()
            );
            let _ = parse_audio_entry(&entry, &out);
        }
        let at = usize::try_from(entry.local_offset)
            .unwrap_or(usize::MAX)
            .min(data.len());
        let out = inflate_entry(&data[at..], &entry, AUDIO_PROBE_BYTES);
        assert!(out.len() < AUDIO_PROBE_BYTES + 8192);
        let _ = parse_audio_entry(&entry, &out);
    }
}

fn corpus() -> Vec<Vec<u8>> {
    let mut corpus: Vec<Vec<u8>> = read_tsv("duration_parse.tsv")
        .into_iter()
        .map(|r| from_hex(&r[3]))
        .collect();
    for name in [
        "zip_deflated_cbr.zip",
        "zip_deflated_cbr_192.zip",
        "zip_deflated_ogg.zip",
        "zip_upper_audio_last.zip",
        "zip_first_audio_wins.zip",
        "zip_no_audio.zip",
        "zip_stored_cbr.zip",
        "zip_method_other.zip",
        "zip_deflated_xing_be.zip",
        "zip_deflated_id3_cover.zip",
        "zip_deflated_mpeg2.zip",
        "zip_macosx_stub_first.zip",
        "zip_dot_underscore_ogg.zip",
        "bare_wav.bin",
        "bare_ogg.bin",
    ] {
        corpus.push(read_fixture_bytes(name));
    }
    corpus
}

#[test]
fn every_prefix_of_every_input_is_handled() {
    bounded(Duration::from_secs(120), || {
        let corpus = corpus();
        assert!(corpus.len() > 900);
        let mut calls = 0usize;
        for data in &corpus {
            for len in 0..data.len().min(160) + 1 {
                exercise(&data[..len]);
                calls += 1;
            }
            if data.len() > 160 {
                let start = data.len() - 160;
                for from in start..=data.len() {
                    exercise(&data[from..]);
                    calls += 1;
                }
            }
        }
        assert!(calls > 100_000, "{calls}");
    });
}

#[test]
fn corrupted_inputs_are_handled() {
    bounded(Duration::from_secs(120), || {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let corpus = corpus();
        for data in &corpus {
            let rounds = if data.len() > 20_000 { 6 } else { 40 };
            for _ in 0..rounds {
                exercise(&mutate(&mut rng, data));
            }
        }
    });
}

#[test]
fn corrupted_archives_are_handled() {
    bounded(Duration::from_secs(120), || {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        for name in [
            "zip_deflated_cbr.zip",
            "zip_stored_cbr.zip",
            "zip_deflated_ogg.zip",
            "zip_upper_audio_last.zip",
            "zip_deflated_xing_be.zip",
        ] {
            let zip = read_fixture_bytes(name);
            let entry = find_audio_entry(&zip).expect("entry");
            let eocd = zip.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
            let directory =
                eocd - u32::from_le_bytes(zip[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
            let local = entry.local_offset as usize;
            let regions = [(directory, zip.len()), (local, local + 64 + 30)];
            for _ in 0..600 {
                let mut z = zip.clone();
                let (from, to) = regions[rng.below(regions.len())];
                for _ in 0..1 + rng.below(5) {
                    let at = from + rng.below(to - from);
                    z[at] = if rng.below(2) == 0 {
                        INTERESTING[rng.below(INTERESTING.len())]
                    } else {
                        rng.next() as u8
                    };
                }
                if let Some(found) = find_audio_entry(&z) {
                    let at = usize::try_from(found.local_offset)
                        .unwrap_or(usize::MAX)
                        .min(z.len());
                    let end = z.len().min(at.saturating_add(AUDIO_PROBE_BYTES + 1));
                    let out = inflate_entry(&z[at..end], &found, AUDIO_PROBE_BYTES);
                    assert!(out.len() < AUDIO_PROBE_BYTES + 8192);
                    let _ = parse_audio_entry(&found, &out);
                }
            }
        }
    });
}

#[test]
fn hostile_entries_do_not_overflow_or_over_allocate() {
    bounded(Duration::from_secs(60), || {
        let stored = ZEntry {
            name: "a.mp3",
            method: METHOD_STORED,
            data: vec![0xAB; 5000],
            size: 5000,
        };
        let deflated = ZEntry {
            name: "a.mp3",
            method: METHOD_DEFLATE,
            data: deflate(&vec![0x11u8; 1 << 22]),
            size: 1 << 22,
        };
        for entry in [stored, deflated] {
            let (zip, offsets) = build_zip(&[entry]);
            let at = offsets[0] as usize;
            for method in [0u16, 8, 1, 9, 12, 14, 93, 0xFFFF] {
                for compressed_size in [0u64, 1, 5000, u64::from(u32::MAX), u64::MAX] {
                    for size in [0u64, 1, u64::from(u32::MAX), u64::MAX] {
                        let hostile = ZipEntry {
                            name: "a.mp3".into(),
                            path: "a.mp3".into(),
                            local_offset: u64::MAX,
                            size,
                            compressed_size,
                            method,
                        };
                        for limit in [0usize, 1, 100, AUDIO_PROBE_BYTES, 1 << 20] {
                            let out = inflate_entry(&zip[at..], &hostile, limit);
                            assert!(
                                out.len() <= limit + 8192,
                                "method {method} limit {limit}: {}",
                                out.len()
                            );
                            let _ = parse_audio_entry(&hostile, &out);
                        }
                    }
                }
            }
        }
    });
}

#[test]
fn big_field_values_allocate_nothing() {
    bounded(Duration::from_secs(60), || {
        let mut wav = vec![0u8; 44];
        wav[..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        wav[12..16].copy_from_slice(b"fmt ");
        wav[16..20].copy_from_slice(&16u32.to_le_bytes());
        wav[28..32].copy_from_slice(&1u32.to_le_bytes());
        wav[36..40].copy_from_slice(b"data");
        wav[40..44].copy_from_slice(&i32::MAX.to_le_bytes());
        assert_eq!(parse_wav_duration(&wav), Some(i32::MAX));
        let mut id3 = vec![0u8; 64];
        id3[..3].copy_from_slice(b"ID3");
        id3[6..10].copy_from_slice(&[0x7F, 0x7F, 0x7F, 0x7F]);
        assert_eq!(parse_mp3_duration(&id3, u64::MAX), None);
        let mut eocd = vec![0u8; 22];
        eocd[..4].copy_from_slice(b"PK\x05\x06");
        eocd[10..12].copy_from_slice(&0xFFFFu16.to_le_bytes());
        eocd[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(find_audio_entry(&eocd), None);
    });
}

#[test]
fn info_files_survive_truncation_and_corruption() {
    bounded(Duration::from_secs(120), || {
        let texts: Vec<Vec<u8>> = [
            "name: Some Song\nmusic: song.mp3\nchart: chart.json\npreviewStart: 15.0\n",
            "\u{feff}name: x\r\nmusic: \"caf\\u00e9 \\\"x\\\".mp3\"  # the song\r\n",
            "music: 'it''s a song.mp3'\n",
            "music:\n  歌曲 - 名.mp3\nchart: c\n",
            "'music' : a.ogg # c\n---\nmusic: |\n  b.mp3\n",
            "#\nName: x\nSong: song.mp3\nMusic: other.ogg\nLevel: AT 16\n",
            "\u{feff}#\r\nNAME: x\r\nsOnG :   a:b\\c.mp3   \r\n",
            "music: \"\\x4\nmusic: \"\\u12\nmusic: \"\\U0011\nmusic: '\n",
            "music:\nmusic:\nmusic:\n  \n\n\n",
            "\n\n\n\n#\n\n\nSong\nSong:\n: x\nSong: \n",
        ]
        .iter()
        .map(|t| t.as_bytes().to_vec())
        .chain([
            vec![0xFF, 0xFE, 0x00, 0x80, b':', b'\n', 0xEF, 0xBB],
            vec![b'a'; 1 << 20],
            "music: x\n".repeat(50_000).into_bytes(),
            "music:\n".repeat(50_000).into_bytes(),
            format!("music: \"{}", "\\u".repeat(100_000)).into_bytes(),
            format!("music: '{}", "''".repeat(100_000)).into_bytes(),
        ])
        .collect();

        let check = |bytes: &[u8]| {
            for format in [InfoFormat::Yml, InfoFormat::Txt] {
                if let Some(music) = parse_info_music(format, bytes) {
                    assert!(!music.is_empty());
                }
            }
        };
        let mut rng = Rng(0xA076_1D64_78BD_642F);
        for text in &texts {
            check(text);
            if text.len() > 4096 {
                continue;
            }
            for len in 0..=text.len() {
                check(&text[..len]);
                check(&text[len..]);
            }
            for _ in 0..300 {
                check(&mutate(&mut rng, text));
            }
        }
    });
}

#[test]
fn hostile_info_entries_are_refused_or_bounded() {
    bounded(Duration::from_secs(60), || {
        let (zip, offsets) = build_zip(&[ZEntry {
            name: "info.yml",
            method: METHOD_DEFLATE,
            data: deflate(&vec![b'#'; 8 * 1024 * 1024]),
            size: 16,
        }]);
        let raw = &zip[offsets[0] as usize..];
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for method in [0u16, 8, 1, 9, 12, 93, 0xFFFF] {
            for size in [
                0u64,
                1,
                16,
                4096,
                INFO_MAX_BYTES,
                INFO_MAX_BYTES + 1,
                u64::from(u32::MAX),
                u64::MAX,
            ] {
                for compressed_size in [0u64, 1, 100, INFO_MAX_BYTES, INFO_MAX_BYTES + 1, u64::MAX]
                {
                    let info = InfoFile {
                        entry: ZipEntry {
                            name: "info.yml".into(),
                            path: "info.yml".into(),
                            local_offset: u64::MAX,
                            size,
                            compressed_size,
                            method,
                        },
                        format: InfoFormat::Yml,
                    };
                    let range = info_music_range(&info);
                    if let Some((_, length)) = range {
                        assert!(length <= 30 + 1024 + 8 + INFO_MAX_BYTES as usize);
                    }
                    let _ = read_info_music(&info, raw);
                    let junk = mutate(&mut rng, raw);
                    let _ = read_info_music(&info, &junk[..junk.len().min(5000)]);
                }
            }
        }
    });
}
