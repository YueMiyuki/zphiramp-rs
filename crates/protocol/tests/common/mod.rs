#![allow(dead_code)]

use std::fmt::Debug;
use std::path::PathBuf;

pub const NO_CAP: usize = usize::MAX;

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"))
}

pub fn rows(name: &str) -> Vec<Vec<String>> {
    fixture_text(name)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

pub fn lines(name: &str) -> Vec<String> {
    fixture_text(name)
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(s.len() % 2 == 0, "odd hex length: {s}");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex digit"))
        .collect()
}

pub fn crc32(data: &[u8]) -> String {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    format!("{:08x}", !crc)
}

// splitmix64; mutate() and the stream recipes must draw in the order the recorded fixtures were made with.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const INTERESTING: [u8; 7] = [0x00, 0x01, 0x7f, 0x80, 0xff, 0xfe, 0x0f];

pub fn mutate(r: &mut Rng, base: &[u8], ops: usize) -> Vec<u8> {
    let mut b = base.to_vec();
    for _ in 0..ops {
        match r.below(5) {
            0 => {
                if !b.is_empty() {
                    let i = r.below(b.len());
                    b[i] = r.next() as u8;
                }
            }
            1 => {
                if !b.is_empty() {
                    let n = r.below(b.len() + 1);
                    b.truncate(n);
                }
            }
            2 => {
                let i = r.below(b.len() + 1);
                let v = r.next() as u8;
                b.insert(i, v);
            }
            3 => {
                if !b.is_empty() {
                    let i = r.below(b.len());
                    b.remove(i);
                }
            }
            _ => {
                if !b.is_empty() {
                    let i = r.below(b.len());
                    b[i] = INTERESTING[r.below(INTERESTING.len())];
                }
            }
        }
    }
    b
}

pub fn assert_same<T: PartialEq + Debug>(got: &T, want: &T, what: &str) {
    if got != want {
        let (g, w) = (format!("{got:?}"), format!("{want:?}"));
        assert_eq!(g, w, "{what}");
    }
}

pub fn assert_bytes(got: &[u8], want: &[u8], what: &str) {
    if got == want {
        return;
    }
    let at = got
        .iter()
        .zip(want)
        .position(|(a, b)| a != b)
        .unwrap_or(got.len().min(want.len()));
    let window = |b: &[u8]| hex(&b[at.saturating_sub(8)..(at + 16).min(b.len())]);
    panic!(
        "{what}: bytes differ at offset {at} (got {} bytes, want {} bytes)\n  got  ..{}\n  want ..{}",
        got.len(),
        want.len(),
        window(got),
        window(want)
    );
}
