mod support;

use support::{read_fixture_bytes, read_tsv};
use zphira_phira::duration::{
    AUDIO_FULL_BYTES, AUDIO_PROBE_BYTES, parse_mp3_duration, parse_ogg_duration, parse_wav_duration,
};

fn rust_probe(kind: &str, head: &[u8], total: u64) -> Option<i32> {
    if kind == "wav" {
        return parse_wav_duration(head);
    }
    for window in [AUDIO_PROBE_BYTES, AUDIO_FULL_BYTES] {
        let audio = &head[..head.len().min(window)];
        let seconds = if kind == "ogg" {
            parse_ogg_duration(audio, total)
        } else {
            parse_mp3_duration(audio, total)
        };
        if seconds.is_some() {
            return seconds;
        }
    }
    None
}

fn finding(name: &str) -> Option<&'static str> {
    match name {
        "lame_vbr_v0_44k" | "lame_vbr_crc_44k" => Some("A-02"),
        "lame_vbr_v4_mono_22k" | "lame_vbr_v5_8k" => Some("A-02, A-27"),
        "lame_cbr64_notag_22k" | "lame_cbr32_notag_mono_11k" => Some("A-27"),
        "lame_cbr128_cover_44k" => Some("A-17"),
        _ => None,
    }
}

#[test]
fn rust_matches_ffprobe_and_java_is_wrong_where_the_findings_say() {
    let rows = read_tsv("real_audio.tsv");
    assert_eq!(rows.len(), 12);
    let mut wrong_in_java = 0;
    for row in rows {
        let [name, kind, total, truth_ms, java] = <[String; 5]>::try_from(row).expect("5 columns");
        let total: u64 = total.parse().unwrap();
        let truth_ms: i64 = truth_ms.parse().unwrap();
        let truth = i32::try_from(truth_ms / 1000).unwrap();
        let head = read_fixture_bytes(&format!("real/{name}.head"));
        let rust = rust_probe(&kind, &head, total);

        if kind == "ogg" {
            assert_eq!((rust, java.as_str()), (None, "null"), "{name}");
            continue;
        }
        assert_eq!(rust, Some(truth), "{name}: ffprobe says {truth_ms} ms");

        match finding(&name) {
            Some(finding) => {
                assert_ne!(
                    java,
                    truth.to_string(),
                    "{name}: Java should be wrong ({finding})"
                );
                wrong_in_java += 1;
            }
            None => assert_eq!(java, truth.to_string(), "{name}: Java is right here"),
        }
    }
    assert_eq!(wrong_in_java, 7);
}

#[test]
fn two_of_the_java_numbers_follow_from_the_flaws() {
    let rows = read_tsv("real_audio.tsv");
    let java = |name: &str| -> i32 {
        rows.iter().find(|r| r[0] == name).unwrap()[4]
            .parse()
            .unwrap()
    };
    assert_eq!(java("lame_vbr_v0_44k"), 21);
    assert_eq!(345_279u64 * 8 / 128_000, 21);
    assert_eq!(java("lame_cbr128_cover_44k"), 9);
    assert_eq!(149_189u64 * 8 / 128_000, 9);
}

#[test]
fn a17_real_id3v2_cover() {
    let file = read_fixture_bytes("real/lame_cbr128_cover_44k.head");
    assert_eq!(file.len(), 149_189, "the head is the whole file");
    assert_eq!(&file[..3], b"ID3");
    let tag = 10
        + ((usize::from(file[6]) << 21)
            | (usize::from(file[7]) << 14)
            | (usize::from(file[8]) << 7)
            | usize::from(file[9]));
    assert!(
        tag > AUDIO_PROBE_BYTES,
        "the tag does not fit the first window: {tag} bytes"
    );
    assert_eq!(
        parse_mp3_duration(&file[..AUDIO_PROBE_BYTES], file.len() as u64),
        None
    );
    assert_eq!(parse_mp3_duration(&file, file.len() as u64), Some(6));
    assert_eq!((file.len() - tag) * 8 / 128_000, 6);
}

#[test]
fn a_real_wav_with_a_list_chunk() {
    let head = read_fixture_bytes("real/ffmpeg_pcm_44k.head");
    assert!(head.windows(4).any(|w| w == b"LIST"));
    assert_eq!(parse_wav_duration(&head), Some(24));
}

#[test]
fn n30_real_crc_protected_files() {
    for name in ["lame_vbr_crc_44k", "lame_cbr128_crc_44k"] {
        let head = read_fixture_bytes(&format!("real/{name}.head"));
        assert_eq!(
            &head[..2],
            &[0xFF, 0xFA],
            "{name}: Layer III, protection bit clear"
        );
        let tag_at = 36;
        assert!(
            &head[tag_at..tag_at + 4] == b"Xing" || &head[tag_at..tag_at + 4] == b"Info",
            "{name}: LAME's tag is at 4 + 32, not 4 + 2 + 32"
        );
        assert_ne!(&head[38..42], b"Xing");
        let total = if name == "lame_vbr_crc_44k" {
            246_473
        } else {
            384_939
        };
        assert_eq!(
            parse_mp3_duration(&head, total),
            Some(24),
            "{name}: LAME's layout"
        );

        let mut moved = head.clone();
        let tag: Vec<u8> = moved[tag_at..tag_at + 12].to_vec();
        moved[tag_at..tag_at + 12].fill(0);
        moved[38..50].copy_from_slice(&tag);
        assert_eq!(
            parse_mp3_duration(&moved, total),
            Some(24),
            "{name}: spec layout"
        );

        let mut none = head.clone();
        none[tag_at..tag_at + 12].fill(0);
        let estimate = parse_mp3_duration(&none, total).unwrap();
        assert_eq!(i64::from(estimate), (total as i64) * 8 / 128_000);
    }
}
