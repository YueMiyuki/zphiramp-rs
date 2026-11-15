mod support;

use support::{ZEntry, build_zip, cbr128, deflate};
use zphira_phira::duration::{
    INFO_MAX_BYTES, InfoFile, InfoFormat, METHOD_DEFLATE, METHOD_STORED, SongChoice, ZipEntry,
    find_info_file, first_audio_entry, info_music_range, list_entries, music_entry,
    parse_info_music, read_info_music, select_song,
};

fn yml(text: &str) -> Option<String> {
    parse_info_music(InfoFormat::Yml, text.as_bytes())
}

fn txt(text: &str) -> Option<String> {
    parse_info_music(InfoFormat::Txt, text.as_bytes())
}

fn some(value: &str) -> Option<String> {
    Some(value.to_owned())
}

#[test]
fn yml_plain_and_quoted_scalars() {
    assert_eq!(yml("music: song.mp3\n"), some("song.mp3"));
    assert_eq!(yml("music: song.mp3"), some("song.mp3"), "no final newline");
    assert_eq!(yml("music: \"song.mp3\"\n"), some("song.mp3"));
    assert_eq!(yml("music: 'song.mp3'\n"), some("song.mp3"));
    assert_eq!(
        yml(
            "id: 4242\nname: Some Song\ndifficulty: 16.5\nlevel: AT Lv.16\ncharter: c\nchart: chart.json\nmusic: music.ogg\nillustration: illustration.png\npreviewStart: 15.0\n"
        ),
        some("music.ogg")
    );
    assert_eq!(
        yml("music: my song (final).mp3\n"),
        some("my song (final).mp3")
    );
    assert_eq!(yml("music: 歌曲 - 名.mp3\n"), some("歌曲 - 名.mp3"));
    assert_eq!(
        yml("music: a#b.mp3\n"),
        some("a#b.mp3"),
        "# without a space before it is not a comment"
    );
    assert_eq!(yml("music: C:\\songs\\a.mp3\n"), some("C:\\songs\\a.mp3"));
}

#[test]
fn yml_escapes_in_double_quotes_and_doubled_quotes_in_single() {
    assert_eq!(yml(r#"music: "say \"hi\".mp3""#), some("say \"hi\".mp3"));
    assert_eq!(yml(r#"music: "back\\slash.mp3""#), some("back\\slash.mp3"));
    assert_eq!(yml(r#"music: "caf\u00e9.mp3""#), some("café.mp3"));
    assert_eq!(yml(r#"music: "A\x42.mp3""#), some("AB.mp3"));
    assert_eq!(yml(r#"music: "tab\there.mp3""#), some("tab\there.mp3"));
    assert_eq!(yml(r#"music: "slash\/.mp3""#), some("slash/.mp3"));
    assert_eq!(yml(r#"music: "\U0001F600.mp3""#), some("😀.mp3"));
    assert_eq!(yml(r#"music: "bad\u12"#), some("bad"));
    assert_eq!(yml(r#"music: "bad\uD800.mp3""#), some("bad.mp3"));
    assert_eq!(yml("music: 'it''s.mp3'\n"), some("it's.mp3"));
    assert_eq!(yml("music: 'a ''b'' c.mp3'\n"), some("a 'b' c.mp3"));
    assert_eq!(yml("music: \"a # b.mp3\"  # comment\n"), some("a # b.mp3"));
    assert_eq!(yml("music: 'a # b.mp3' # comment\n"), some("a # b.mp3"));
    assert_eq!(yml("music: \"song.mp3\n"), some("song.mp3"));
    assert_eq!(yml("music: 'song.mp3\n"), some("song.mp3"));
}

#[test]
fn yml_bom_crlf_spaces_comments() {
    assert_eq!(yml("\u{feff}music: song.mp3\n"), some("song.mp3"));
    assert_eq!(
        parse_info_music(
            InfoFormat::Yml,
            b"\xEF\xBB\xBFname: x\r\nmusic: \"song.mp3\"\r\nchart: c\r\n"
        ),
        some("song.mp3")
    );
    assert_eq!(
        yml("name: x\r\nmusic: song.mp3\r\n"),
        some("song.mp3"),
        "CRLF"
    );
    assert_eq!(
        yml("music:    song.mp3   \n"),
        some("song.mp3"),
        "extra spaces"
    );
    assert_eq!(yml("music:\tsong.mp3\t\n"), some("song.mp3"), "tabs");
    assert_eq!(
        yml("music : song.mp3\n"),
        some("song.mp3"),
        "space before the colon"
    );
    assert_eq!(yml("music: song.mp3 # the song\n"), some("song.mp3"));
    assert_eq!(yml("music: song.mp3\t# the song\n"), some("song.mp3"));
    assert_eq!(
        yml("# comment\n---\nname: x\nmusic: song.mp3\n...\n"),
        some("song.mp3")
    );
    assert_eq!(yml("\n\n   \nmusic: song.mp3\n\n"), some("song.mp3"));
    assert_eq!(yml("\"music\": song.mp3\n"), some("song.mp3"));
    assert_eq!(yml("'music': song.mp3\n"), some("song.mp3"));
    assert_eq!(
        parse_info_music(InfoFormat::Yml, b"name: \xFF\xFE\nmusic: song.mp3\n"),
        some("song.mp3")
    );
}

#[test]
fn yml_value_on_the_next_line() {
    assert_eq!(yml("music:\n  song.mp3\nchart: c\n"), some("song.mp3"));
    assert_eq!(yml("music: # the song\n  \"song.mp3\"\n"), some("song.mp3"));
    assert_eq!(yml("music:\n\n  # c\n  song.mp3\n"), some("song.mp3"));
    assert_eq!(yml("music:\nchart: c.json\n"), None);
    assert_eq!(yml("music:\n"), None);
    assert_eq!(yml("music:"), None);
}

#[test]
fn yml_what_is_not_a_music_entry() {
    assert_eq!(yml(""), None);
    assert_eq!(yml("   \n"), None);
    assert_eq!(yml("name: x\nchart: c.json\n"), None);
    assert_eq!(yml("music: \n"), None, "empty value");
    assert_eq!(yml("music: \"\"\n"), None);
    assert_eq!(yml("music: ''\n"), None);
    assert_eq!(yml("music: |\n  song.mp3\n"), None, "block scalar");
    assert_eq!(yml("music: >\n  song.mp3\n"), None);
    assert_eq!(yml("music: [a.mp3]\n"), None, "sequence");
    assert_eq!(yml("music: {a: b}\n"), None, "mapping");
    assert_eq!(yml("music: *alias\n"), None);
    assert_eq!(yml("tip:\n  music: song.mp3\n"), None);
    assert_eq!(yml("  music: song.mp3\n"), None);
    assert_eq!(
        yml("musicVolume: 3\nmusic_file: a.mp3\nmusics: b.mp3\n"),
        None
    );
    assert_eq!(
        yml("Music: song.mp3\n"),
        None,
        "the client's key is exactly `music`"
    );
    assert_eq!(yml("# music: song.mp3\n"), None);
    assert_eq!(yml("musicx: y\n"), None);
    assert_eq!(
        yml("music:song.mp3\n"),
        None,
        "YAML needs a space after the colon"
    );
    assert_eq!(
        parse_info_music(InfoFormat::Yml, &[0xFF, 0x00, 0x80, 0xFE]),
        None
    );
}

#[test]
fn yml_a_repeated_key_takes_the_last_value() {
    assert_eq!(yml("music: a.mp3\nmusic: b.mp3\n"), some("b.mp3"));
    assert_eq!(yml("music: a.mp3\nmusic:\n"), None);
}

#[test]
fn txt_music_and_song_lines() {
    assert_eq!(txt("#\nName: x\nSong: song.mp3\n"), some("song.mp3"));
    assert_eq!(txt("#\nName: x\nMusic: song.ogg\n"), some("song.ogg"));
    assert_eq!(
        txt("#\r\nName: x\r\nSong: song.mp3\r\nLevel: AT 16\r\n"),
        some("song.mp3"),
        "CRLF"
    );
    assert_eq!(txt("\u{feff}#\nSong: song.mp3\n"), some("song.mp3"), "BOM");
    assert_eq!(
        parse_info_music(
            InfoFormat::Txt,
            b"\xEF\xBB\xBF#\r\nName: \xFF\r\nSong: song.mp3\r\n"
        ),
        some("song.mp3")
    );
    assert_eq!(
        txt(
            "#\nName: Some Song\nPath: 123\nSong: 123.mp3\nPicture: 123.png\nChart: 123.json\nLevel: SP Lv.15\nComposer: x\nCharter: y\n"
        ),
        some("123.mp3")
    );
}

#[test]
fn txt_keys_in_any_case_spacing_and_values_with_colons() {
    for key in ["song", "SONG", "Song", "sOnG", "music", "MUSIC", "Music"] {
        assert_eq!(txt(&format!("#\n{key}: a.mp3\n")), some("a.mp3"), "{key}");
    }
    assert_eq!(
        txt("#\nSong:a.mp3\n"),
        some("a.mp3"),
        "no space after the colon"
    );
    assert_eq!(
        txt("#\n  Song  :   a.mp3   \n"),
        some("a.mp3"),
        "spaces around key and value"
    );
    assert_eq!(
        txt("#\nSong: C:\\x\\a.mp3\n"),
        some("C:\\x\\a.mp3"),
        "a colon in the value"
    );
    assert_eq!(txt("#\nSong: 歌 曲.mp3\n"), some("歌 曲.mp3"));
    assert_eq!(
        txt("Song: a.mp3\n"),
        some("a.mp3"),
        "the leading # line is optional here"
    );
    assert_eq!(txt("\n\n#\n\nSong: a.mp3\n\n"), some("a.mp3"));
    assert_eq!(
        txt("#\nnonsense\nSong: a.mp3\nmore nonsense\n"),
        some("a.mp3")
    );
    assert_eq!(txt("#\nMusic Offset: 3\nSongName: x\nMy Song: y\n"), None);
}

#[test]
fn txt_both_keys_the_last_one_wins_and_empty_is_nothing() {
    assert_eq!(txt("#\nMusic: a.mp3\nSong: b.mp3\n"), some("b.mp3"));
    assert_eq!(txt("#\nSong: b.mp3\nMusic: a.mp3\n"), some("a.mp3"));
    assert_eq!(txt("#\nSong: a.mp3\nSong: b.mp3\n"), some("b.mp3"));
    assert_eq!(txt("#\nSong: a.mp3\nSong:\n"), None);
    assert_eq!(txt("#\nSong:   \n"), None);
    assert_eq!(txt(""), None);
    assert_eq!(txt("#\nName: x\n"), None);
    assert_eq!(parse_info_music(InfoFormat::Txt, &[0xFF, 0xFE, 0x00]), None);
}

fn entries_of(names: &[&str]) -> Vec<ZipEntry> {
    let zip_entries: Vec<ZEntry<'_>> = names
        .iter()
        .map(|name| ZEntry {
            name,
            method: METHOD_STORED,
            data: b"x".to_vec(),
            size: 1,
        })
        .collect();
    let (zip, _) = build_zip(&zip_entries);
    list_entries(&zip)
}

fn info_path(names: &[&str]) -> Option<(String, InfoFormat)> {
    find_info_file(&entries_of(names)).map(|i| (i.entry.path, i.format))
}

#[test]
fn the_info_file_is_at_the_root_or_in_the_single_top_level_folder() {
    let yml = InfoFormat::Yml;
    let txt = InfoFormat::Txt;
    assert_eq!(
        info_path(&["info.yml", "a.mp3"]),
        Some(("info.yml".into(), yml))
    );
    assert_eq!(
        info_path(&["info.txt", "a.mp3"]),
        Some(("info.txt".into(), txt))
    );
    assert_eq!(
        info_path(&["info.txt", "info.yml"]),
        Some(("info.yml".into(), yml))
    );
    assert_eq!(
        info_path(&["info.yml", "info.txt"]),
        Some(("info.yml".into(), yml))
    );
    assert_eq!(
        info_path(&["info.txt", "chart/info.yml", "chart/a.mp3"]),
        Some(("chart/info.yml".into(), yml))
    );
    assert_eq!(
        info_path(&["chart/info.yml", "info.yml"]),
        Some(("info.yml".into(), yml))
    );
    assert_eq!(
        info_path(&["chart/", "chart/info.yml", "chart/a.mp3"]),
        Some(("chart/info.yml".into(), yml))
    );
    assert_eq!(
        info_path(&["chart/info.txt", "chart/a.mp3"]),
        Some(("chart/info.txt".into(), txt))
    );
    assert_eq!(
        info_path(&["INFO.YML", "a.mp3"]),
        Some(("INFO.YML".into(), yml))
    );
    assert_eq!(
        info_path(&["Chart/Info.Txt", "Chart/a.mp3"]),
        Some(("Chart/Info.Txt".into(), txt))
    );
    assert_eq!(
        info_path(&[
            "chart/info.yml",
            "chart/a.mp3",
            "__MACOSX/",
            "__MACOSX/chart/._info.yml"
        ]),
        Some(("chart/info.yml".into(), yml))
    );
    assert_eq!(info_path(&["._info.yml", "a.mp3"]), None);
    assert_eq!(info_path(&["__MACOSX/info.yml", "a.mp3"]), None);
    assert_eq!(info_path(&["a.mp3", "b.json"]), None);
    assert_eq!(
        info_path(&["a/b/info.yml", "a/b/c.mp3"]),
        None,
        "two levels down"
    );
    assert_eq!(
        info_path(&["one/info.yml", "two/a.mp3"]),
        None,
        "two top-level folders"
    );
    assert_eq!(
        info_path(&["info.yml.bak", "my-info.yml", "info.yaml", "info.json"]),
        None
    );
    assert_eq!(info_path(&["info.yml/", "a.mp3"]), None, "a directory");
    assert_eq!(info_path(&[]), None);
}

#[test]
fn the_music_entry_matches_exactly_then_in_any_case_relative_to_the_info_file() {
    let entries = entries_of(&[
        "chart/",
        "chart/info.yml",
        "chart/Song.mp3",
        "chart/song.mp3",
        "chart/SOUND.OGG",
        "chart/sub/deep.mp3",
        "chart/x.wav",
        "chart/dir.mp3/",
        "root.mp3",
    ]);
    let info = find_info_file(&entries).unwrap();
    assert_eq!(info.entry.path, "chart/info.yml");
    let find = |music: &str| music_entry(&entries, &info, music).map(|e| e.path);

    assert_eq!(find("song.mp3").as_deref(), Some("chart/song.mp3"));
    assert_eq!(find("Song.mp3").as_deref(), Some("chart/Song.mp3"));
    assert_eq!(
        find("SONG.MP3").as_deref(),
        Some("chart/Song.mp3"),
        "the first of the case-insensitive matches"
    );
    assert_eq!(find("sound.ogg").as_deref(), Some("chart/SOUND.OGG"));
    assert_eq!(find("Sub/DEEP.mp3").as_deref(), Some("chart/sub/deep.mp3"));
    assert_eq!(find("./song.mp3").as_deref(), Some("chart/song.mp3"));
    assert_eq!(find("sub\\deep.mp3").as_deref(), Some("chart/sub/deep.mp3"));
    assert_eq!(find("sub//deep.mp3").as_deref(), Some("chart/sub/deep.mp3"));
    assert_eq!(find("  song.mp3  ").as_deref(), Some("chart/song.mp3"));
    assert_eq!(
        find("x.wav").as_deref(),
        Some("chart/x.wav"),
        "any file, the caller decides what it can time"
    );
    assert_eq!(find("root.mp3"), None);
    for music in [
        "",
        " ",
        ".",
        "/",
        "/chart/song.mp3",
        "../root.mp3",
        "sub/../song.mp3",
        "song.mp3/",
        "dir.mp3",
        "dir.mp3/",
        "nothing.mp3",
        "C:\\song.mp3",
    ] {
        assert_eq!(find(music), None, "{music:?}");
    }

    let entries = entries_of(&["info.yml", "song.mp3", "chart/song.mp3"]);
    let info = find_info_file(&entries).unwrap();
    assert_eq!(
        music_entry(&entries, &info, "song.mp3").unwrap().path,
        "song.mp3"
    );
    assert_eq!(
        music_entry(&entries, &info, "chart/Song.MP3").unwrap().path,
        "chart/song.mp3"
    );
}

#[test]
fn select_song_chooses_the_named_file_and_falls_back_to_the_first_audio() {
    let entries = entries_of(&[
        "info.yml",
        "preview.mp3",
        "song.mp3",
        "other.ogg",
        "song.wav",
        "song.flac",
    ]);
    let info = find_info_file(&entries).unwrap();
    let chosen =
        |info: Option<&InfoFile>, music: Option<&str>| match select_song(&entries, info, music) {
            SongChoice::Entry(e) => format!("entry {}", e.path),
            SongChoice::Unsupported(e) => format!("unsupported {}", e.path),
            SongChoice::NoAudio => "no audio".to_owned(),
        };
    assert_eq!(chosen(Some(&info), Some("song.mp3")), "entry song.mp3");
    assert_eq!(chosen(Some(&info), Some("OTHER.OGG")), "entry other.ogg");
    assert_eq!(
        chosen(Some(&info), Some("song.wav")),
        "unsupported song.wav"
    );
    assert_eq!(
        chosen(Some(&info), Some("song.flac")),
        "unsupported song.flac"
    );
    assert_eq!(
        chosen(Some(&info), Some("info.yml")),
        "unsupported info.yml"
    );
    assert_eq!(
        chosen(Some(&info), Some("gone.mp3")),
        "entry preview.mp3",
        "named file is not in the archive"
    );
    assert_eq!(
        chosen(Some(&info), None),
        "entry preview.mp3",
        "no music entry or unreadable"
    );
    assert_eq!(
        chosen(None, Some("song.mp3")),
        "entry preview.mp3",
        "no info file"
    );
    assert_eq!(chosen(None, None), "entry preview.mp3");
    let stubs = entries_of(&["__MACOSX/._a.mp3", "._b.ogg", "c.wav", "info.yml"]);
    assert_eq!(select_song(&stubs, None, None), SongChoice::NoAudio);
    assert!(first_audio_entry(&stubs).is_none());
    let info = find_info_file(&stubs).unwrap();
    assert_eq!(
        select_song(&stubs, Some(&info), Some("c.wav")),
        SongChoice::Unsupported(stubs[2].clone())
    );
    assert_eq!(select_song(&[], None, None), SongChoice::NoAudio);
}

fn archive_with_info(info_bytes: &[u8], method: u16) -> (Vec<u8>, InfoFile, usize) {
    let data = if method == METHOD_DEFLATE {
        deflate(info_bytes)
    } else {
        info_bytes.to_vec()
    };
    let (zip, offsets) = build_zip(&[
        ZEntry {
            name: "info.yml",
            method,
            data,
            size: info_bytes.len() as u32,
        },
        ZEntry {
            name: "song.mp3",
            method: METHOD_STORED,
            data: cbr128(1000),
            size: 1000,
        },
    ]);
    let info = find_info_file(&list_entries(&zip)).unwrap();
    (zip, info, offsets[0] as usize)
}

#[test]
fn the_info_range_comes_from_the_central_directory() {
    let text = b"name: x\nmusic: song.mp3\n";
    for method in [METHOD_STORED, METHOD_DEFLATE] {
        let (zip, info, offset) = archive_with_info(text, method);
        let (start, len) = info_music_range(&info).unwrap();
        assert_eq!(start as usize, offset);
        assert_eq!(
            len,
            30 + 1024 + "info.yml".len() + info.entry.compressed_size as usize
        );
        let raw = &zip[start as usize..zip.len().min(start as usize + len)];
        assert_eq!(
            read_info_music(&info, raw).as_deref(),
            Some("song.mp3"),
            "method {method}"
        );
    }
}

#[test]
fn what_the_info_range_refuses() {
    let (_, info, _) = archive_with_info(b"music: a.mp3\n", METHOD_STORED);
    let with = |change: &dyn Fn(&mut ZipEntry)| {
        let mut info = info.clone();
        change(&mut info.entry);
        info_music_range(&info)
    };
    assert!(with(&|_| {}).is_some());
    assert!(with(&|e| e.size = 0).is_none(), "empty");
    assert!(with(&|e| e.size = INFO_MAX_BYTES).is_some(), "at the cap");
    assert!(
        with(&|e| e.size = INFO_MAX_BYTES + 1).is_none(),
        "uncompressed over the cap"
    );
    assert!(
        with(&|e| e.compressed_size = INFO_MAX_BYTES + 1).is_none(),
        "compressed over the cap"
    );
    assert!(
        with(&|e| e.size = u64::from(u32::MAX)).is_none(),
        "ZIP64 sentinel"
    );
    assert!(with(&|e| e.compressed_size = u64::from(u32::MAX)).is_none());
    assert!(with(&|e| e.local_offset = u64::from(u32::MAX)).is_none());
    assert!(with(&|e| e.size = u64::MAX).is_none());
    for method in [1u16, 9, 12, 14, 93, 0xFFFF] {
        assert!(with(&|e| e.method = method).is_none(), "method {method}");
    }
    assert!(
        with(&|e| e.path = "x".repeat(70_000)).is_some(),
        "a long name only lengthens the request"
    );
}

#[test]
fn reading_the_info_data_is_checked() {
    let text = b"name: x\nmusic: song.mp3\n";
    for method in [METHOD_STORED, METHOD_DEFLATE] {
        let (zip, info, offset) = archive_with_info(text, method);
        let header = 30 + "info.yml".len();
        let data_end = offset + header + info.entry.compressed_size as usize;
        for cut in 0..data_end - offset {
            let read = read_info_music(&info, &zip[offset..offset + cut]);
            if method == METHOD_STORED || cut < header {
                assert_eq!(read, None, "method {method}, cut {cut}");
            } else {
                assert!(
                    read.is_none() || read.as_deref() == Some("song.mp3"),
                    "method {method}, cut {cut}: {read:?}"
                );
            }
        }
        assert_eq!(
            read_info_music(&info, &zip[offset..data_end]).as_deref(),
            Some("song.mp3")
        );
        let mut lying = info.clone();
        lying.entry.size += 100;
        assert_eq!(read_info_music(&lying, &zip[offset..]), None);
        let mut short = info.clone();
        short.entry.size = 8;
        assert_eq!(
            read_info_music(&short, &zip[offset..]),
            None,
            "`name: x\\n` names no music"
        );
        assert_eq!(read_info_music(&info, &zip[offset + 1..]), None);
        assert_eq!(read_info_music(&info, &[]), None);
        let mut big = info.clone();
        big.entry.size = INFO_MAX_BYTES + 1;
        assert_eq!(read_info_music(&big, &zip[offset..]), None);
        let mut other = info.clone();
        other.entry.method = 12;
        assert_eq!(read_info_music(&other, &zip[offset..]), None);
    }
}

#[test]
fn the_info_file_cannot_inflate_beyond_its_claimed_size() {
    let mut text = b"music: song.mp3\n".to_vec();
    text.extend(std::iter::repeat_n(b'#', 32 * 1024 * 1024));
    let bomb = deflate(&text);
    assert!(bomb.len() < 100 * 1024, "{}", bomb.len());
    let (zip, offsets) = build_zip(&[ZEntry {
        name: "info.yml",
        method: METHOD_DEFLATE,
        data: bomb,
        size: 16,
    }]);
    let raw = &zip[offsets[0] as usize..];
    let info = find_info_file(&list_entries(&zip)).unwrap();
    assert_eq!(info.entry.size, 16);
    assert_eq!(read_info_music(&info, raw).as_deref(), Some("song.mp3"));
    let mut at_cap = info.clone();
    at_cap.entry.size = INFO_MAX_BYTES;
    assert_eq!(read_info_music(&at_cap, raw).as_deref(), Some("song.mp3"));
    let mut over = info;
    over.entry.size = INFO_MAX_BYTES + 1;
    assert_eq!(read_info_music(&over, raw), None);
}
