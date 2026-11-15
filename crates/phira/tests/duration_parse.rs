mod support;

use support::{from_hex, read_tsv};
use zphira_phira::duration::{
    parse_bare_audio, parse_mp3_duration, parse_ogg_duration, parse_wav_duration,
};

fn run(kind: &str, declared: u64, data: &[u8]) -> Option<i32> {
    match kind {
        "mp3" => parse_mp3_duration(data, declared),
        "ogg" => parse_ogg_duration(data, declared),
        "wav" => parse_wav_duration(data),
        other => panic!("unknown kind {other}"),
    }
}

fn java_value(text: &str) -> Option<i32> {
    match text {
        "null" => None,
        n => Some(n.parse().unwrap_or_else(|_| panic!("not a number: {n}"))),
    }
}

struct Row {
    category: String,
    kind: String,
    declared: u64,
    data: Vec<u8>,
    java: String,
    note: String,
}

fn rows() -> Vec<Row> {
    let rows: Vec<Row> = read_tsv("duration_parse.tsv")
        .into_iter()
        .map(|r| {
            let [category, kind, declared, hex, java, note] = <[String; 6]>::try_from(r)
                .unwrap_or_else(|r| panic!("bad row: {} fields", r.len()));
            Row {
                category,
                kind,
                declared: declared.parse().expect("declared size"),
                data: from_hex(&hex),
                java,
                note,
            }
        })
        .collect();
    assert!(rows.len() > 900, "fixture looks truncated: {}", rows.len());
    rows
}

#[test]
fn where_java_is_correct_rust_returns_the_same_value() {
    let mut checked = 0;
    let mut numeric = 0;
    for row in rows().iter().filter(|r| r.category == "same") {
        let expected = java_value(&row.java);
        assert_eq!(
            run(&row.kind, row.declared, &row.data),
            expected,
            "{} / declared {} / {}",
            row.kind,
            row.declared,
            row.note
        );
        checked += 1;
        numeric += usize::from(expected.is_some());
    }
    assert!(
        checked > 800 && numeric > 250,
        "{checked} rows, {numeric} with a value"
    );
}

#[test]
fn where_java_threw_rust_fails_the_probe() {
    let exc: Vec<_> = rows().into_iter().filter(|r| r.category == "exc").collect();
    assert!(exc.len() >= 15, "{} rows", exc.len());
    for row in exc {
        assert!(row.java.starts_with("EXC "), "{}", row.java);
        assert_eq!(
            run(&row.kind, row.declared, &row.data),
            None,
            "{} / {}",
            row.kind,
            row.note
        );
    }
}

#[test]
fn flawed_rows_assert_the_corrected_values() {
    let expected: &[(&str, &str, &str, Option<i32>)] = &[
        (
            "A-02",
            "MPEG-1 stereo Xing, BE flags 0xF and 9000 frames: true 235 s",
            "449",
            Some(235),
        ),
        (
            "A-02",
            "MPEG-1 mono Info, BE 44100 frames: true 1152 s",
            "62",
            Some(1152),
        ),
        (
            "A-02",
            "the old test fixture: little-endian fields (1000 frames)",
            "26",
            Some(8),
        ),
        (
            "A-02",
            "Xing without the frames flag falls back to the bitrate",
            "8",
            Some(8),
        ),
        (
            "A-17",
            "100 s of audio plus a 16000 byte tag: true 100 s",
            "101",
            Some(100),
        ),
        (
            "A-17",
            "java test: id3TagIsSkipped (declared 128000 including the 10 byte tag)",
            "8",
            Some(7),
        ),
        (
            "A-27",
            "MPEG-2 22.05 kHz 80 kbps CBR, 300 s",
            "187",
            Some(300),
        ),
        (
            "A-27",
            "MPEG-2 22.05 kHz stereo Xing, 20000 frames: true 522 s",
            "326",
            Some(522),
        ),
        (
            "A-27",
            "MPEG-2.5 11.025 kHz 80 kbps CBR, 100 s",
            "62",
            Some(100),
        ),
        ("A-27", "MPEG-1 Layer II header", "8", None),
        (
            "A-27",
            "reserved MPEG version bits (Java reads it as MPEG-2 tables)",
            "8",
            None,
        ),
        (
            "A-28",
            "LIST chunk of 0x7FFFFFF0 bytes",
            "EXC StringIndexOutOfBoundsException",
            None,
        ),
        (
            "A-28",
            "fmt chunk whose byte rate is cut off by the end of the data",
            "EXC ArrayIndexOutOfBoundsException",
            None,
        ),
        (
            "A-28",
            "nominal bitrate 1 on a huge file: result does not fit an int",
            "0",
            None,
        ),
        (
            "A-28",
            "nominal bitrate 1 on a 4 GiB file: result does not fit an int",
            "-8",
            None,
        ),
    ];
    let rows = rows();
    let flawed: Vec<_> = rows
        .iter()
        .filter(|r| r.category.starts_with("A-"))
        .collect();
    assert_eq!(
        flawed.len(),
        expected.len(),
        "every flawed row has an expectation"
    );
    for (finding, note, java, corrected) in expected {
        let row = flawed
            .iter()
            .find(|r| r.note == *note)
            .unwrap_or_else(|| panic!("no row {note}"));
        assert_eq!(row.category, *finding, "{note}");
        assert_eq!(row.java, *java, "recorded Java value: {note}");
        assert_eq!(
            run(&row.kind, row.declared, &row.data),
            *corrected,
            "{finding}: {note}"
        );
    }
}

fn put_be(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_le(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_ascii(data: &mut [u8], offset: usize, text: &str) {
    data[offset..offset + text.len()].copy_from_slice(text.as_bytes());
}

fn mp3_frame() -> Vec<u8> {
    let mut data = vec![0u8; 64];
    data[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
    data
}

#[test]
fn xing_header_gives_the_exact_frame_count() {
    let mut data = mp3_frame();
    let xing = 4 + 32;
    put_ascii(&mut data, xing, "Xing");
    put_be(&mut data, xing + 4, 0x1);
    put_be(&mut data, xing + 8, 1000);
    assert_eq!(parse_mp3_duration(&data, 0), Some(26));

    let mut java_fixture = mp3_frame();
    put_ascii(&mut java_fixture, xing, "Xing");
    put_le(&mut java_fixture, xing + 4, 0x1);
    put_le(&mut java_fixture, xing + 8, 1000);
    assert_eq!(parse_mp3_duration(&java_fixture, 0), None);
    assert_eq!(parse_mp3_duration(&java_fixture, 128_000), Some(8));
}

#[test]
fn info_tag_is_accepted_as_well() {
    let mut data = mp3_frame();
    let info = 4 + 32;
    put_ascii(&mut data, info, "Info");
    put_be(&mut data, info + 4, 0x1);
    put_be(&mut data, info + 8, 44_100);
    assert_eq!(parse_mp3_duration(&data, 0), Some(1152));
}

#[test]
fn constant_bitrate_falls_back_to_the_file_size() {
    assert_eq!(parse_mp3_duration(&mp3_frame(), 128_000), Some(8));
}

#[test]
fn id3_tag_is_skipped() {
    let mut data = vec![0u8; 128];
    put_ascii(&mut data, 0, "ID3");
    data[3] = 0x03;
    data[9] = 0x00;
    data[10..14].copy_from_slice(&mp3_frame()[..4]);
    assert_eq!(parse_mp3_duration(&data, 128_000), Some(7));
    assert_eq!(parse_mp3_duration(&data, 128_010), Some(8));
}

#[test]
fn garbage_is_rejected() {
    assert_eq!(parse_mp3_duration(&[], 0), None);
    assert_eq!(parse_mp3_duration(b"not audio at all", 0), None);
}

#[test]
fn ogg_uses_the_nominal_bitrate() {
    let mut data = vec![0u8; 64];
    put_ascii(&mut data, 0, "OggS");
    data[26] = 1;
    data[27] = 30;
    let body = 27 + 1;
    data[body] = 0x01;
    put_ascii(&mut data, body + 1, "vorbis");
    put_le(&mut data, body + 12, 44_100);
    put_le(&mut data, body + 16, 0);
    put_le(&mut data, body + 20, 128_000);
    assert_eq!(parse_ogg_duration(&data, 256_000), Some(16));
}

#[test]
fn ogg_needs_its_identification_header() {
    let mut data = vec![0u8; 64];
    put_ascii(&mut data, 0, "OggS");
    assert_eq!(parse_ogg_duration(&data, 256_000), None);
    assert_eq!(parse_ogg_duration(&[], 256_000), None);
}

#[test]
fn wav_is_read_from_its_chunks() {
    let mut data = vec![0u8; 44];
    put_ascii(&mut data, 0, "RIFF");
    put_le(&mut data, 4, 36);
    put_ascii(&mut data, 8, "WAVE");
    put_ascii(&mut data, 12, "fmt ");
    put_le(&mut data, 16, 16);
    put_le(&mut data, 20, 1 | (2 << 16));
    put_le(&mut data, 24, 44_100);
    put_le(&mut data, 28, 176_400);
    put_ascii(&mut data, 36, "data");
    put_le(&mut data, 40, 529_200);
    assert_eq!(parse_wav_duration(&data), Some(3));
}

#[test]
fn truncated_wav_is_rejected() {
    assert_eq!(parse_wav_duration(&[0; 8]), None);
}

fn header(version: u8, layer: u8, bitrate_index: u8, rate_index: u8, channel_mode: u8) -> [u8; 4] {
    [
        0xFF,
        0xE0 | (version << 3) | (layer << 1) | 1,
        (bitrate_index << 4) | (rate_index << 2),
        channel_mode << 6,
    ]
}

fn frame_with(header: [u8; 4], tag_at: usize, tag: &str, flags: u32, frames: u32) -> Vec<u8> {
    let mut data = vec![0u8; 200];
    data[..4].copy_from_slice(&header);
    put_ascii(&mut data, tag_at, tag);
    put_be(&mut data, tag_at + 4, flags);
    put_be(&mut data, tag_at + 8, frames);
    data
}

#[test]
fn a02_xing_and_info_fields_are_big_endian() {
    let h = header(3, 1, 9, 0, 0);
    for tag in ["Xing", "Info"] {
        let data = frame_with(h, 4 + 32, tag, 0x0F, 9000);
        assert_eq!(
            parse_mp3_duration(&data, 235 * 245_000 / 8),
            Some(235),
            "{tag}"
        );
        assert_eq!(parse_mp3_duration(&data, 0), Some(235));
        assert_eq!(parse_mp3_duration(&data, u64::MAX), Some(235));
    }
    let data = frame_with(h, 4 + 32, "Xing", 0x0E, 9000);
    assert_eq!(parse_mp3_duration(&data, 128_000), Some(8));
    assert_eq!(
        parse_mp3_duration(&frame_with(h, 4 + 32, "Xing", 1, 0), 128_000),
        Some(8)
    );
    assert_eq!(
        parse_mp3_duration(&frame_with(h, 4 + 32, "Xing", 1, 0x8000_0000), 128_000),
        Some(8)
    );
    assert_eq!(
        parse_mp3_duration(&frame_with(h, 4 + 32, "Xinh", 1, 9000), 128_000),
        Some(8)
    );
}

#[test]
fn a02_vbri_frame_count() {
    let mut data = vec![0u8; 200];
    data[..4].copy_from_slice(&header(3, 1, 9, 0, 0));
    put_ascii(&mut data, 4 + 32, "VBRI");
    put_be(&mut data, 4 + 32 + 10, 123_456);
    put_be(&mut data, 4 + 32 + 14, 4000);
    assert_eq!(parse_mp3_duration(&data, 999), Some(104));
    put_be(&mut data, 4 + 32 + 14, 0);
    assert_eq!(parse_mp3_duration(&data, 128_000), Some(8));
}

#[test]
fn a17_id3_tag_is_not_audio() {
    let tag = 400 * 1024;
    let mut data = vec![0u8; 10 + tag + 64];
    put_ascii(&mut data, 0, "ID3");
    data[3] = 3;
    data[6] = ((tag >> 21) & 0x7F) as u8;
    data[7] = ((tag >> 14) & 0x7F) as u8;
    data[8] = ((tag >> 7) & 0x7F) as u8;
    data[9] = (tag & 0x7F) as u8;
    data[10 + tag..10 + tag + 4].copy_from_slice(&header(3, 1, 9, 0, 0));
    let audio = 340 * 16_000;
    let declared = (10 + tag + audio) as u64;
    assert_eq!(parse_mp3_duration(&data, declared), Some(340));
    assert_eq!(declared * 8 / 128_000, 365);

    assert_eq!(parse_mp3_duration(&data, 100), None);

    let mut footer = vec![0u8; 10 + tag + 10 + 64];
    footer[..10].copy_from_slice(&data[..10]);
    footer[5] = 0x10;
    footer[10 + tag + 10..10 + tag + 14].copy_from_slice(&header(3, 1, 9, 0, 0));
    assert_eq!(
        parse_mp3_duration(&footer, (10 + tag + 10 + audio) as u64),
        Some(340)
    );
}

#[test]
fn a18_bare_audio_is_timed_from_its_total_size() {
    let mut window = vec![0u8; 32 * 1024];
    window[..4].copy_from_slice(&header(3, 1, 9, 0, 0));
    assert_eq!(parse_bare_audio(&window, 400 * 16_000), Some(400));
    assert_eq!(parse_bare_audio(&window, 0), None);
    assert_eq!(window.len() as u64 * 8 / 128_000, 2);

    let mut ogg = vec![0u8; 4000];
    put_ascii(&mut ogg, 0, "OggS");
    ogg[26] = 1;
    ogg[27] = 30;
    ogg[28] = 1;
    put_ascii(&mut ogg, 29, "vorbis");
    put_le(&mut ogg, 28 + 20, 128_000);
    assert_eq!(parse_bare_audio(&ogg, 1_600_000), Some(100));
    assert_eq!(parse_bare_audio(&ogg, 0), None);

    let mut wav = vec![0u8; 44];
    put_ascii(&mut wav, 0, "RIFF");
    put_ascii(&mut wav, 8, "WAVE");
    put_ascii(&mut wav, 12, "fmt ");
    put_le(&mut wav, 16, 16);
    put_le(&mut wav, 28, 176_400);
    put_ascii(&mut wav, 36, "data");
    put_le(&mut wav, 40, 1_764_000);
    assert_eq!(parse_bare_audio(&wav, 0), Some(10));
    assert_eq!(parse_bare_audio(&wav, 12345), Some(10));

    assert_eq!(parse_bare_audio(b"this is not audio", 17), None);
    assert_eq!(parse_bare_audio(&[1, 2, 3], 3), None);
    assert_eq!(parse_bare_audio(&[], 0), None);
    assert_eq!(parse_bare_audio(b"RIFF", 4), None);
}

#[test]
fn a27_mpeg_version_tables() {
    const MPEG1: [u32; 14] = [
        32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const MPEG2: [u32; 14] = [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let rates = [
        (3u8, [44_100u32, 48_000, 32_000], &MPEG1),
        (2, [22_050, 24_000, 16_000], &MPEG2),
        (0, [11_025, 12_000, 8_000], &MPEG2),
    ];
    for (version, _rates, table) in rates {
        for (i, kbps) in table.iter().enumerate() {
            for rate_index in 0..3u8 {
                for mode in [0u8, 3] {
                    let mut data = vec![0u8; 64];
                    data[..4].copy_from_slice(&header(version, 1, i as u8 + 1, rate_index, mode));
                    let size = u64::from(*kbps) * 1000 / 8 * 100;
                    assert_eq!(
                        parse_mp3_duration(&data, size),
                        Some(100),
                        "version {version} {kbps} kbps"
                    );
                }
            }
        }
    }

    let cases = [
        (3u8, 0u8, 1152u64, 44_100u64),
        (3, 1, 1152, 48_000),
        (3, 2, 1152, 32_000),
        (2, 0, 576, 22_050),
        (2, 1, 576, 24_000),
        (2, 2, 576, 16_000),
        (0, 0, 576, 11_025),
        (0, 1, 576, 12_000),
        (0, 2, 576, 8_000),
    ];
    for (version, rate_index, samples, rate) in cases {
        let side_info = if version == 3 { 32 } else { 17 };
        let data = frame_with(
            header(version, 1, 9, rate_index, 0),
            4 + side_info,
            "Xing",
            1,
            10_000,
        );
        assert_eq!(
            parse_mp3_duration(&data, 0),
            Some((10_000 * samples / rate) as i32),
            "version {version} rate {rate}"
        );
    }
}

#[test]
fn a27_side_info_offsets() {
    for (version, mode, side_info) in [
        (3u8, 0u8, 32usize),
        (3, 3, 17),
        (2, 0, 17),
        (2, 3, 9),
        (0, 0, 17),
        (0, 3, 9),
    ] {
        let right = frame_with(
            header(version, 1, 9, 0, mode),
            4 + side_info,
            "Xing",
            1,
            5000,
        );
        assert!(
            parse_mp3_duration(&right, 0).is_some(),
            "version {version} mode {mode}"
        );
        let wrong = frame_with(
            header(version, 1, 9, 0, mode),
            4 + side_info + 1,
            "Xing",
            1,
            5000,
        );
        assert_eq!(
            parse_mp3_duration(&wrong, 0),
            None,
            "version {version} mode {mode}"
        );
    }
}

#[test]
fn a27_invalid_headers() {
    let size = 128_000;
    for layer in [0u8, 2, 3] {
        let mut data = vec![0u8; 64];
        data[..4].copy_from_slice(&header(3, layer, 9, 0, 0));
        assert_eq!(parse_mp3_duration(&data, size), None, "layer bits {layer}");
    }
    let mut data = vec![0u8; 64];
    data[..4].copy_from_slice(&header(1, 1, 9, 0, 0));
    assert_eq!(parse_mp3_duration(&data, size), None, "reserved version");
    for bitrate_index in [0u8, 15] {
        data[..4].copy_from_slice(&header(3, 1, bitrate_index, 0, 0));
        assert_eq!(
            parse_mp3_duration(&data, size),
            None,
            "bitrate index {bitrate_index}"
        );
    }
    data[..4].copy_from_slice(&header(3, 1, 9, 3, 0));
    assert_eq!(
        parse_mp3_duration(&data, size),
        None,
        "reserved sample rate"
    );
    let mut only_junk = vec![0u8; 64];
    only_junk[..2].copy_from_slice(&[0xFF, 0xFF]);
    only_junk[10..12].copy_from_slice(&[0xFF, 0xE0]);
    assert_eq!(parse_mp3_duration(&only_junk, size), None);
}

#[test]
fn a28_wav_walk_is_checked() {
    fn wav(first_chunk: (&str, u32), body_len: usize) -> Vec<u8> {
        let mut d = vec![0u8; 12 + 8 + body_len];
        put_ascii(&mut d, 0, "RIFF");
        put_ascii(&mut d, 8, "WAVE");
        put_ascii(&mut d, 12, first_chunk.0);
        put_le(&mut d, 16, first_chunk.1);
        d
    }
    for size in [
        0x7FFF_FFF0u32,
        0x7FFF_FFFF,
        0x7FFF_FFFE,
        0x8000_0000,
        0xFFFF_FFFF,
        0xFFFF_FFF0,
    ] {
        let mut d = wav(("LIST", size), 64);
        let n = d.len();
        put_ascii(&mut d, n - 24, "fmt ");
        put_ascii(&mut d, n - 8, "data");
        assert_eq!(parse_wav_duration(&d), None, "LIST size {size:#x}");
    }
    for tail_len in 8..20 {
        let mut d = vec![0u8; 12 + 8 + 16 + tail_len];
        put_ascii(&mut d, 0, "RIFF");
        put_ascii(&mut d, 8, "WAVE");
        put_ascii(&mut d, 12, "LIST");
        put_le(&mut d, 16, 16);
        let fmt_at = 12 + 8 + 16;
        put_ascii(&mut d, fmt_at, "fmt ");
        put_le(&mut d, fmt_at + 4, 16);
        assert_eq!(parse_wav_duration(&d), None, "fmt with {tail_len} bytes");
    }
}

#[test]
fn a28_sizes_are_checked() {
    let frame = mp3_frame();
    for declared in [
        0,
        1,
        u64::from(u32::MAX),
        u64::MAX / 8,
        u64::MAX / 8 + 1,
        u64::MAX,
    ] {
        let _ = parse_mp3_duration(&frame, declared);
        let _ = parse_ogg_duration(&frame, declared);
    }
    assert_eq!(parse_mp3_duration(&frame, u64::MAX), None);
    let mut low = vec![0u8; 64];
    low[..4].copy_from_slice(&header(2, 1, 1, 0, 0));
    assert_eq!(
        parse_mp3_duration(&low, u64::from(u32::MAX)),
        Some(4_294_967)
    );
    assert_eq!(parse_mp3_duration(&low, u64::MAX / 8), None);
    let mut ogg = vec![0u8; 64];
    put_ascii(&mut ogg, 0, "OggS");
    ogg[26] = 1;
    ogg[28] = 1;
    put_ascii(&mut ogg, 29, "vorbis");
    put_le(&mut ogg, 28 + 20, 1);
    assert_eq!(parse_ogg_duration(&ogg, 1 << 40), None);
    assert_eq!(parse_ogg_duration(&ogg, 1000), Some(8000));
    let mut wav = vec![0u8; 44];
    put_ascii(&mut wav, 0, "RIFF");
    put_ascii(&mut wav, 8, "WAVE");
    put_ascii(&mut wav, 12, "fmt ");
    put_le(&mut wav, 16, 16);
    put_le(&mut wav, 28, 1);
    put_ascii(&mut wav, 36, "data");
    put_le(&mut wav, 40, i32::MAX as u32);
    assert_eq!(parse_wav_duration(&wav), Some(i32::MAX));
    put_le(&mut wav, 40, 0x8000_0000);
    assert_eq!(parse_wav_duration(&wav), None);
}

#[test]
fn results_are_at_least_one_second() {
    let mut data = vec![0u8; 64];
    data[..4].copy_from_slice(&header(3, 1, 9, 0, 0));
    assert_eq!(parse_mp3_duration(&data, 1), Some(1));
    assert_eq!(parse_mp3_duration(&data, 100), Some(1));
    let mut ogg = vec![0u8; 64];
    put_ascii(&mut ogg, 0, "OggS");
    ogg[26] = 1;
    ogg[28] = 1;
    put_ascii(&mut ogg, 29, "vorbis");
    put_le(&mut ogg, 28 + 20, 500_000);
    assert_eq!(parse_ogg_duration(&ogg, 10), Some(1));
}

fn reference_wav(d: &[u8]) -> Option<i32> {
    if d.len() < 44 {
        return None;
    }
    let (mut rate, mut data_size) = (0i64, 0i64);
    let mut position = 12usize;
    while position + 8 <= d.len() {
        let id = &d[position..position + 4];
        let size = i64::from(i32::from_le_bytes(
            d[position + 4..position + 8].try_into().unwrap(),
        ));
        if id == b"fmt " && position + 20 <= d.len() {
            rate = i64::from(i32::from_le_bytes(
                d[position + 16..position + 20].try_into().unwrap(),
            ));
        } else if id == b"data" {
            data_size = size;
            break;
        }
        if size < 0 {
            break;
        }
        position += 8 + size as usize + (size as usize & 1);
    }
    (rate > 0 && data_size > 0).then(|| (data_size / rate).max(1) as i32)
}

#[test]
fn n29_empty_chunks_are_stepped_over() {
    let rows: Vec<Row> = rows()
        .into_iter()
        .filter(|r| r.category == "N-29")
        .collect();
    assert!(rows.len() >= 3, "{} rows", rows.len());
    for row in &rows {
        assert_eq!(row.kind, "wav");
        assert_eq!(row.java, "null", "{}", row.note);
        let rust = parse_wav_duration(&row.data);
        assert_eq!(rust, reference_wav(&row.data), "{}", row.note);
    }
    for note in [
        "empty LIST chunk before data: Java stops at the zero-size chunk",
        "empty LIST chunk before fmt: Java stops at the zero-size chunk",
    ] {
        let row = rows.iter().find(|r| r.note == note).expect(note);
        assert_eq!(parse_wav_duration(&row.data), Some(10), "{note}");
    }

    fn chunk(out: &mut Vec<u8>, id: &str, body: &[u8], size: u32) {
        out.extend_from_slice(id.as_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
    }
    let mut fmt = vec![0u8; 16];
    fmt[8..12].copy_from_slice(&176_400u32.to_le_bytes());
    let mut wav = b"RIFF\0\0\0\0WAVE".to_vec();
    chunk(&mut wav, "JUNK", &[], 0);
    chunk(&mut wav, "fmt ", &fmt, 16);
    chunk(&mut wav, "LIST", &[], 0);
    chunk(&mut wav, "LIST", &[], 0);
    chunk(&mut wav, "odd ", &[1, 2, 3], 3);
    chunk(&mut wav, "LIST", &[], 0);
    chunk(&mut wav, "data", &[0; 8], 1_764_000);
    assert_eq!(parse_wav_duration(&wav), Some(10));
    let mut many = b"RIFF\0\0\0\0WAVE".to_vec();
    chunk(&mut many, "fmt ", &fmt, 16);
    for _ in 0..1000 {
        chunk(&mut many, "JUNK", &[], 0);
    }
    chunk(&mut many, "data", &[0; 8], 1_764_000);
    assert_eq!(parse_wav_duration(&many), Some(10));
    let mut empty_tail = b"RIFF\0\0\0\0WAVE".to_vec();
    chunk(&mut empty_tail, "fmt ", &fmt, 16);
    for _ in 0..100 {
        chunk(&mut empty_tail, "JUNK", &[], 0);
    }
    assert_eq!(parse_wav_duration(&empty_tail), None);
    let mut empty_data = b"RIFF\0\0\0\0WAVE".to_vec();
    chunk(&mut empty_data, "fmt ", &fmt, 16);
    chunk(&mut empty_data, "data", &[], 0);
    chunk(&mut empty_data, "JUNK", &[0; 16], 16);
    assert_eq!(parse_wav_duration(&empty_data), None);
    let mut huge = b"RIFF\0\0\0\0WAVE".to_vec();
    chunk(&mut huge, "fmt ", &fmt, 16);
    chunk(&mut huge, "LIST", &[0; 8], 0x8000_0000);
    chunk(&mut huge, "data", &[0; 8], 1_764_000);
    assert_eq!(parse_wav_duration(&huge), None);
}

fn expectation(note: &str) -> Option<i32> {
    let (_, value) = note.rsplit_once("expect=").expect("expect= in the note");
    java_value(value)
}

#[test]
fn n30_rows_match_the_expectation_and_java_gave_up_or_misread() {
    let rows: Vec<Row> = rows()
        .into_iter()
        .filter(|r| r.category == "N-30")
        .collect();
    assert!(rows.len() >= 50, "{} rows", rows.len());
    let (mut decoys, mut explicit) = (0, 0);
    for row in &rows {
        let expected = expectation(&row.note);
        assert_eq!(
            parse_mp3_duration(&row.data, row.declared),
            expected,
            "{}",
            row.note
        );
        if row.note.starts_with("decoy sync") {
            decoys += 1;
            assert_eq!(row.java, "null", "Java gave up at the decoy");
        } else {
            explicit += 1;
            assert_ne!(
                java_value(&row.java),
                expected,
                "{}: Java differs",
                row.note
            );
        }
    }
    assert!(
        decoys >= 40 && explicit == 4,
        "{decoys} decoys, {explicit} explicit rows"
    );
    let java = |needle: &str| -> String {
        rows.iter()
            .find(|r| r.note.contains(needle))
            .unwrap()
            .java
            .clone()
    };
    assert_eq!(java("CRC protected MPEG-1 stereo"), "449");
    assert_eq!(java("CRC protected MPEG-2 mono"), "326");
    assert_eq!(java("FF FF junk"), "null");
    assert_eq!(java("wrong ID3 size"), "null");
}

#[test]
fn n30_the_scan_continues_past_invalid_candidates() {
    let valid = header(3, 1, 9, 0, 0);
    let invalid: [[u8; 4]; 9] = [
        [0xFF, 0xFB, 0x00, 0x00],
        [0xFF, 0xFB, 0xF0, 0x00],
        [0xFF, 0xFB, 0x9C, 0x00],
        [0xFF, 0xEB, 0x90, 0x00],
        [0xFF, 0xFD, 0x90, 0x00],
        [0xFF, 0xFF, 0x90, 0x00],
        [0xFF, 0xF9, 0x90, 0x00],
        [0xFF, 0xFF, 0xFF, 0xFF],
        [0xFF, 0xE0, 0x00, 0x00],
    ];
    for bad in invalid {
        for lead in [0usize, 1, 3, 9, 100] {
            for repeats in [1usize, 2, 5] {
                let mut data = vec![0u8; lead];
                for _ in 0..repeats {
                    data.extend_from_slice(&bad);
                }
                data.extend_from_slice(&valid);
                data.extend_from_slice(&[0u8; 60]);
                assert_eq!(
                    parse_mp3_duration(&data, 128_000),
                    Some(8),
                    "{bad:02x?} x{repeats} after {lead} bytes"
                );
            }
        }
    }
    let mut data = vec![0xFFu8; 6];
    data.extend_from_slice(&valid);
    data.extend_from_slice(&[0u8; 60]);
    assert_eq!(parse_mp3_duration(&data, 128_000), Some(8));
    for len in 0..40 {
        let mut data = vec![0u8; len];
        for chunk in data.as_chunks_mut::<4>().0 {
            *chunk = invalid[len % invalid.len()];
        }
        assert_eq!(parse_mp3_duration(&data, 128_000), None, "{len}");
    }
    let mut data = vec![0u8; 20];
    data.extend_from_slice(&valid[..3]);
    assert_eq!(parse_mp3_duration(&data, 128_000), None);
}

#[test]
fn n30_junk_inside_a_mislabelled_tag() {
    let mut data = vec![0u8; 4000];
    data[..3].copy_from_slice(b"ID3");
    data[3] = 3;
    for i in (20..3000).step_by(97) {
        data[i] = 0xFF;
        data[i + 1] = 0xF5 + (i % 3) as u8;
    }
    data[3000..3004].copy_from_slice(&header(3, 1, 9, 0, 0));
    assert_eq!(parse_mp3_duration(&data, 128_010), Some(8));
}

#[test]
fn n30_crc_protected_frames() {
    fn protected(version: u8, mode: u8, tag_at: usize, tag: &str, frames: u32) -> Vec<u8> {
        let mut data = vec![0u8; 200];
        let mut h = header(version, 1, 9, 0, mode);
        h[1] &= 0xFE;
        data[..4].copy_from_slice(&h);
        data[tag_at..tag_at + 4].copy_from_slice(tag.as_bytes());
        data[tag_at + 4..tag_at + 8].copy_from_slice(&1u32.to_be_bytes());
        data[tag_at + 8..tag_at + 12].copy_from_slice(&frames.to_be_bytes());
        data
    }
    for (version, mode, side, samples, rate) in [
        (3u8, 0u8, 32usize, 1152u64, 44_100u64),
        (3, 3, 17, 1152, 44_100),
        (2, 0, 17, 576, 22_050),
        (2, 3, 9, 576, 22_050),
        (0, 3, 9, 576, 11_025),
    ] {
        let expected = Some((10_000 * samples / rate) as i32);
        for tag in ["Xing", "Info"] {
            let after_crc = protected(version, mode, 4 + 2 + side, tag, 10_000);
            assert_eq!(
                parse_mp3_duration(&after_crc, 0),
                expected,
                "v{version} m{mode} {tag} after the CRC"
            );
            let lame = protected(version, mode, 4 + side, tag, 10_000);
            assert_eq!(
                parse_mp3_duration(&lame, 0),
                expected,
                "v{version} m{mode} {tag} at the unprotected offset"
            );
            let off = protected(version, mode, 4 + 2 + side + 1, tag, 10_000);
            assert_eq!(
                parse_mp3_duration(&off, 0),
                None,
                "v{version} m{mode} {tag} off by one"
            );
        }
    }
    let mut data = vec![0u8; 200];
    data[..4].copy_from_slice(&header(3, 1, 9, 0, 0));
    data[38..42].copy_from_slice(b"Xing");
    data[42..46].copy_from_slice(&1u32.to_be_bytes());
    data[46..50].copy_from_slice(&9000u32.to_be_bytes());
    assert_eq!(parse_mp3_duration(&data, 0), None);
    assert_eq!(parse_mp3_duration(&data, 128_000), Some(8));
    let mut plain = vec![0u8; 200];
    let mut h = header(3, 1, 9, 0, 0);
    h[1] &= 0xFE;
    plain[..4].copy_from_slice(&h);
    assert_eq!(parse_mp3_duration(&plain, 128_000), Some(8));
}
