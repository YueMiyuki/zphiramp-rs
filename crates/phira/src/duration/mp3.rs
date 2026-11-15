use super::bytes::be_i32;

const BITRATES_MPEG1: [u32; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];
const BITRATES_MPEG2: [u32; 16] = [
    0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    Mpeg1,
    Mpeg2,
    Mpeg25,
}

#[derive(Debug, Clone, Copy)]
struct FrameHeader {
    version: Version,
    sample_rate: u32,
    bitrate_kbps: u32,
    mono: bool,
    samples_per_frame: u32,
    crc: bool,
}

impl FrameHeader {
    fn side_info_length(&self) -> usize {
        match (self.version, self.mono) {
            (Version::Mpeg1, true) => 17,
            (Version::Mpeg1, false) => 32,
            (_, true) => 9,
            (_, false) => 17,
        }
    }
}

/// `declared_size` is the size of the whole file, ID3 tag included.
pub fn parse_mp3_duration(audio: &[u8], declared_size: u64) -> Option<i32> {
    let tag_length = id3v2_length(audio);
    let (frame, header) = find_first_frame(audio, tag_length)?;
    if let Some(seconds) = vbr_duration(audio, frame, &header) {
        return Some(seconds);
    }
    let audio_bytes = declared_size.checked_sub(tag_length as u64)?;
    if declared_size == 0 || audio_bytes == 0 {
        return None;
    }
    let bits = audio_bytes.checked_mul(8)?;
    let seconds = (bits / (u64::from(header.bitrate_kbps) * 1000)).max(1);
    i32::try_from(seconds).ok()
}

fn vbr_duration(audio: &[u8], frame: usize, header: &FrameHeader) -> Option<i32> {
    let frames =
        xing_frame_count(audio, frame, header).or_else(|| vbri_frame_count(audio, frame))?;
    if frames <= 0 {
        return None;
    }
    let samples = i64::from(frames).checked_mul(i64::from(header.samples_per_frame))?;
    i32::try_from(samples / i64::from(header.sample_rate)).ok()
}

// LAME keeps the Xing/Info tag at the unprotected offset even when the CRC bit is set; the
// offset after the two CRC bytes (what the format says) is tried second.
fn xing_frame_count(audio: &[u8], frame: usize, header: &FrameHeader) -> Option<i32> {
    let unprotected = frame.checked_add(4 + header.side_info_length())?;
    let protected = header.crc.then(|| unprotected.checked_add(2)).flatten();
    [Some(unprotected), protected]
        .into_iter()
        .flatten()
        .find_map(|xing| xing_frame_count_at(audio, xing))
}

fn xing_frame_count_at(audio: &[u8], xing: usize) -> Option<i32> {
    if xing.checked_add(12)? > audio.len() {
        return None;
    }
    let tag = audio.get(xing..xing + 4)?;
    if tag != b"Xing" && tag != b"Info" {
        return None;
    }
    let flags = be_i32(audio, xing + 4)?;
    if flags & 0x1 == 0 {
        return None;
    }
    be_i32(audio, xing + 8)
}

fn vbri_frame_count(audio: &[u8], frame: usize) -> Option<i32> {
    let vbri = frame.checked_add(36)?;
    if audio.get(vbri..vbri + 4)? != b"VBRI" {
        return None;
    }
    be_i32(audio, vbri + 14)
}

fn id3v2_length(audio: &[u8]) -> usize {
    if audio.len() > 10 && audio.starts_with(b"ID3") {
        let size = (usize::from(audio[6] & 0x7F) << 21)
            | (usize::from(audio[7] & 0x7F) << 14)
            | (usize::from(audio[8] & 0x7F) << 7)
            | usize::from(audio[9] & 0x7F);
        let footer = if audio[5] & 0x10 != 0 { 10 } else { 0 };
        return 10 + size + footer;
    }
    0
}

fn find_first_frame(audio: &[u8], from: usize) -> Option<(usize, FrameHeader)> {
    let last = audio.len().checked_sub(4)?;
    (from..=last).find_map(|i| {
        if audio[i] == 0xFF && audio[i + 1] & 0xE0 == 0xE0 {
            parse_frame_header(audio, i).map(|header| (i, header))
        } else {
            None
        }
    })
}

fn parse_frame_header(audio: &[u8], frame: usize) -> Option<FrameHeader> {
    let b1 = *audio.get(frame + 1)?;
    let b2 = *audio.get(frame + 2)?;
    let b3 = *audio.get(frame + 3)?;
    let version = match (b1 >> 3) & 0x3 {
        0 => Version::Mpeg25,
        2 => Version::Mpeg2,
        3 => Version::Mpeg1,
        _ => return None,
    };
    if (b1 >> 1) & 0x3 != 1 {
        return None;
    }
    let bitrate_index = usize::from(b2 >> 4);
    let rate_index = usize::from((b2 >> 2) & 0x3);
    if bitrate_index == 0 || bitrate_index >= 15 || rate_index == 3 {
        return None;
    }
    let (bitrate_kbps, sample_rate, samples_per_frame) = match version {
        Version::Mpeg1 => (
            BITRATES_MPEG1[bitrate_index],
            [44100, 48000, 32000][rate_index],
            1152,
        ),
        Version::Mpeg2 => (
            BITRATES_MPEG2[bitrate_index],
            [22050, 24000, 16000][rate_index],
            576,
        ),
        Version::Mpeg25 => (
            BITRATES_MPEG2[bitrate_index],
            [11025, 12000, 8000][rate_index],
            576,
        ),
    };
    let channel_mode = b3 >> 6;
    Some(FrameHeader {
        version,
        sample_rate,
        bitrate_kbps,
        mono: channel_mode == 3,
        samples_per_frame,
        crc: b1 & 1 == 0,
    })
}
