use super::bytes::le_i32;

pub fn parse_wav_duration(data: &[u8]) -> Option<i32> {
    if data.len() < 44 {
        return None;
    }
    let mut byte_rate: i32 = 0;
    let mut data_size: i32 = 0;
    let mut position: usize = 12;
    while position.checked_add(8)? <= data.len() {
        let id = data.get(position..position + 4)?;
        let size = le_i32(data, position + 4)?;
        let fmt_complete = position
            .checked_add(20)
            .is_some_and(|end| end <= data.len());
        if id == b"fmt " && fmt_complete {
            byte_rate = le_i32(data, position + 16)?;
        } else if id == b"data" {
            data_size = size;
            break;
        }
        // An empty chunk advances by its 8 byte header; only a negative size must end the walk.
        if size < 0 {
            break;
        }
        let step = usize::try_from(size)
            .ok()?
            .checked_add(8 + (size as usize & 1))?;
        position = position.checked_add(step)?;
    }
    if byte_rate > 0 && data_size > 0 {
        let seconds = (i64::from(data_size) / i64::from(byte_rate)).max(1);
        i32::try_from(seconds).ok()
    } else {
        None
    }
}
