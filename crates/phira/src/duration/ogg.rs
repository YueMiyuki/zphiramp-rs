use super::bytes::le_i32;

const OGG_PAGE_HEADER: usize = 27;

pub fn parse_ogg_duration(data: &[u8], declared_size: u64) -> Option<i32> {
    if data.len() < 47 || data[0] != 0x4F || data[1] != 0x67 {
        return None;
    }
    let segment_count = usize::from(data[26]);
    let body = OGG_PAGE_HEADER + segment_count;
    if body + 28 > data.len() || data[body] != 1 {
        return None;
    }
    if data.get(body + 1..body + 7)? != b"vorbis" {
        return None;
    }
    let nominal = le_i32(data, body + 20)?;
    if nominal <= 0 || declared_size == 0 {
        return None;
    }
    let seconds = declared_size.checked_mul(8)? / u64::try_from(nominal).ok()?;
    i32::try_from(seconds.max(1)).ok()
}
