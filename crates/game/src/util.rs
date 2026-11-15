use std::sync::{Mutex, MutexGuard, PoisonError};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub(crate) fn below(&mut self, bound: usize) -> usize {
        if bound <= 1 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}

pub fn mask_token(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 5 {
        return "***".to_owned();
    }
    let head: String = chars[..2].iter().collect();
    let tail: String = chars[chars.len() - 3..].iter().collect();
    format!("{head}***{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_mask() {
        assert_eq!(mask_token("abcdefxyz"), "ab***xyz");
        assert_eq!(mask_token("abcde"), "***");
        assert_eq!(mask_token(""), "***");
        assert_eq!(mask_token("abcdef"), "ab***def");
    }
}
