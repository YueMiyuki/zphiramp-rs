mod bytes;
pub(crate) mod http;
mod info;
mod mp3;
mod ogg;
pub(crate) mod probe;
mod wav;
mod zip;

pub use info::{
    INFO_MAX_BYTES, InfoFile, InfoFormat, SongChoice, find_info_file, info_music_range,
    music_entry, parse_info_music, parse_info_txt_music, parse_info_yml_music, read_info_music,
    select_song,
};
pub use mp3::parse_mp3_duration;
pub use ogg::parse_ogg_duration;
pub use wav::parse_wav_duration;
pub use zip::{
    METHOD_DEFLATE, METHOD_STORED, find_audio_entry, first_audio_entry, inflate_entry, list_entries,
};

use bytes::le_i32;

pub const TAIL_BYTES: usize = 512 * 1024;
pub const AUDIO_PROBE_BYTES: usize = 32 * 1024;
pub const AUDIO_FULL_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    pub name: String,
    pub path: String,
    pub local_offset: u64,
    pub size: u64,
    pub compressed_size: u64,
    pub method: u16,
}

impl ZipEntry {
    pub fn is_audio(&self) -> bool {
        self.name.ends_with(".mp3") || self.name.ends_with(".ogg")
    }

    pub fn is_directory(&self) -> bool {
        self.path.ends_with('/')
    }

    pub fn needs_zip64(&self) -> bool {
        [self.size, self.compressed_size, self.local_offset]
            .contains(&u64::from(zip::ZIP64_SENTINEL))
    }
}

pub fn is_zip(head: &[u8]) -> bool {
    le_i32(head, 0) == Some(zip::LOCAL_SIG)
}

pub fn parse_audio_entry(entry: &ZipEntry, audio: &[u8]) -> Option<i32> {
    if entry.name.ends_with(".ogg") {
        parse_ogg_duration(audio, entry.size)
    } else {
        parse_mp3_duration(audio, entry.size)
    }
}

/// `total_size` is the size of the whole file; 0 means unknown.
pub fn parse_bare_audio(head: &[u8], total_size: u64) -> Option<i32> {
    if head.len() >= 12 && head.starts_with(b"RIFF") {
        return parse_wav_duration(head);
    }
    if head.starts_with(b"OggS") {
        return parse_ogg_duration(head, total_size);
    }
    parse_mp3_duration(head, total_size)
}
