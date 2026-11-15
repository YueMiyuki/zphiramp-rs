use std::time::Duration;

use super::http::{ProbeHttp, next_chunk, timed_out};
use super::{
    AUDIO_FULL_BYTES, AUDIO_PROBE_BYTES, SongChoice, TAIL_BYTES, find_info_file, first_audio_entry,
    inflate_entry, info_music_range, is_zip, list_entries, parse_audio_entry, parse_bare_audio,
    read_info_music, select_song,
};
use crate::PhiraError;
use crate::error::Kind;

const MAX_STREAMED: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wanted {
    Span { start: u64, len: usize },
    Suffix { len: usize },
}

impl Wanted {
    fn range_header(self) -> String {
        match self {
            Wanted::Span { start, len } => format!("bytes={start}-{}", start + len as u64 - 1),
            Wanted::Suffix { len } => format!("bytes=-{len}"),
        }
    }
}

#[derive(Debug)]
struct Fetched {
    body: Vec<u8>,
    total: Option<u64>,
}

pub(crate) async fn probe(
    http: &ProbeHttp,
    url: &str,
    timeout: Duration,
) -> Result<Option<i32>, PhiraError> {
    let url = reqwest::Url::parse(url)
        .map_err(|error| PhiraError::new(Kind::InvalidUrl(format!("{url}: {error}"))))?;
    let fetcher = Fetcher { http, url, timeout };

    let head = fetcher
        .fetch(Wanted::Span {
            start: 0,
            len: AUDIO_PROBE_BYTES,
        })
        .await?;
    if !is_zip(&head.body) {
        return Ok(parse_bare_audio(&head.body, head.total.unwrap_or(0)));
    }

    let tail = fetcher.fetch(Wanted::Suffix { len: TAIL_BYTES }).await?;
    let entries = list_entries(&tail.body);

    let info = find_info_file(&entries).filter(|_| first_audio_entry(&entries).is_some());
    let music = match info
        .as_ref()
        .and_then(|info| Some((info, info_music_range(info)?)))
    {
        Some((info, (start, len))) => {
            let raw = fetcher.fetch(Wanted::Span { start, len }).await?;
            read_info_music(info, &raw.body)
        }
        None => None,
    };
    let entry = match select_song(&entries, info.as_ref(), music.as_deref()) {
        SongChoice::Entry(entry) => entry,
        SongChoice::Unsupported(_) | SongChoice::NoAudio => return Ok(None),
    };
    if entry.needs_zip64() {
        return Ok(None);
    }
    for window in [AUDIO_PROBE_BYTES, AUDIO_FULL_BYTES] {
        let raw = fetcher
            .fetch(Wanted::Span {
                start: entry.local_offset,
                // the range end is inclusive
                len: window + 1,
            })
            .await?;
        let audio = inflate_entry(&raw.body, &entry, window);
        if let Some(seconds) = parse_audio_entry(&entry, &audio) {
            return Ok(Some(seconds));
        }
    }
    Ok(None)
}

struct Fetcher<'a> {
    http: &'a ProbeHttp,
    url: reqwest::Url,
    timeout: Duration,
}

impl Fetcher<'_> {
    async fn fetch(&self, wanted: Wanted) -> Result<Fetched, PhiraError> {
        tokio::time::timeout(self.timeout, self.fetch_unbounded(wanted))
            .await
            .map_err(|_| timed_out())?
    }

    async fn fetch_unbounded(&self, wanted: Wanted) -> Result<Fetched, PhiraError> {
        let mut response = self.http.get(&self.url, &wanted.range_header()).await?;
        let status = response.status;
        if status != 200 && status != 206 {
            return Err(PhiraError::probe(format!(
                "HTTP {status} for chart archive"
            )));
        }
        let content_range_total = response.content_range_total;
        let content_length = response.content_length;

        let mut window = Window::new(wanted, status == 200);
        while let Some(chunk) = next_chunk(&mut response.body).await? {
            if window.push(&chunk)? == Flow::Done {
                break;
            }
        }

        let total = match (status, content_range_total) {
            (_, Some(total)) => Some(total),
            (200, None) => content_length.or(window.streamed_to_end()),
            (_, None) => match wanted {
                Wanted::Span { start: 0, len } if window.kept() < len => Some(window.kept() as u64),
                _ => None,
            },
        };
        Ok(Fetched {
            body: window.into_body(),
            total,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    More,
    Done,
}

struct Window {
    wanted: Wanted,
    skip: u64,
    buffer: Vec<u8>,
    streamed: u64,
    ended: bool,
}

impl Window {
    fn new(wanted: Wanted, whole_file: bool) -> Self {
        let skip = match wanted {
            Wanted::Span { start, .. } if whole_file => start,
            _ => 0,
        };
        Self {
            wanted,
            skip,
            buffer: Vec::new(),
            streamed: 0,
            ended: true,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Flow, PhiraError> {
        self.streamed += chunk.len() as u64;
        if self.streamed > MAX_STREAMED {
            return Err(PhiraError::new(Kind::TooLarge(MAX_STREAMED as usize)));
        }
        let mut chunk = chunk;
        match self.wanted {
            Wanted::Span { len, .. } => {
                let skipped = usize::try_from(self.skip)
                    .unwrap_or(usize::MAX)
                    .min(chunk.len());
                self.skip -= skipped as u64;
                chunk = &chunk[skipped..];
                let room = len - self.buffer.len();
                if chunk.len() >= room {
                    self.buffer.extend_from_slice(&chunk[..room]);
                    self.ended = false;
                    return Ok(Flow::Done);
                }
                self.buffer.extend_from_slice(chunk);
            }
            Wanted::Suffix { len } => {
                self.buffer.extend_from_slice(chunk);
                // compacting lazily keeps at most twice the window in memory
                if self.buffer.len() > 2 * len {
                    let excess = self.buffer.len() - len;
                    self.buffer.drain(..excess);
                }
            }
        }
        Ok(Flow::More)
    }

    fn kept(&self) -> usize {
        match self.wanted {
            Wanted::Suffix { len } => self.buffer.len().min(len),
            Wanted::Span { .. } => self.buffer.len(),
        }
    }

    fn streamed_to_end(&self) -> Option<u64> {
        self.ended.then_some(self.streamed)
    }

    fn into_body(mut self) -> Vec<u8> {
        if let Wanted::Suffix { len } = self.wanted
            && self.buffer.len() > len
        {
            let excess = self.buffer.len() - len;
            self.buffer.drain(..excess);
        }
        self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_headers_are_the_java_ones() {
        assert_eq!(
            Wanted::Span {
                start: 0,
                len: AUDIO_PROBE_BYTES
            }
            .range_header(),
            "bytes=0-32767"
        );
        assert_eq!(
            Wanted::Suffix { len: TAIL_BYTES }.range_header(),
            "bytes=-524288"
        );
        assert_eq!(
            Wanted::Span {
                start: 1000,
                len: AUDIO_PROBE_BYTES + 1
            }
            .range_header(),
            "bytes=1000-33768"
        );
    }

    fn run(wanted: Wanted, whole_file: bool, file: &[u8], chunk: usize) -> (Vec<u8>, Option<u64>) {
        let mut window = Window::new(wanted, whole_file);
        for piece in file.chunks(chunk) {
            if window.push(piece).unwrap() == Flow::Done {
                break;
            }
        }
        let total = window.streamed_to_end();
        (window.into_body(), total)
    }

    #[test]
    fn span_of_a_whole_file_skips_and_stops_early() {
        let file: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        for chunk in [1, 7, 4096, 20_000] {
            let (body, total) = run(
                Wanted::Span {
                    start: 100,
                    len: 50,
                },
                true,
                &file,
                chunk,
            );
            assert_eq!(body, &file[100..150], "chunk {chunk}");
            assert_eq!(total, None, "stopped before the end");
            let (body, total) = run(
                Wanted::Span {
                    start: 9990,
                    len: 50,
                },
                true,
                &file,
                chunk,
            );
            assert_eq!(body, &file[9990..]);
            assert_eq!(total, Some(10_000));
            let (body, total) = run(
                Wanted::Span {
                    start: 20_000,
                    len: 50,
                },
                true,
                &file,
                chunk,
            );
            assert!(body.is_empty());
            assert_eq!(total, Some(10_000));
        }
    }

    #[test]
    fn span_of_a_partial_answer_takes_the_prefix() {
        let file: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let (body, _) = run(
            Wanted::Span {
                start: 500,
                len: 100,
            },
            false,
            &file,
            64,
        );
        assert_eq!(body, &file[..100], "a 206 body starts at the span");
    }

    #[test]
    fn suffix_keeps_only_the_last_bytes_in_bounded_memory() {
        let file: Vec<u8> = (0..100_000u32).map(|i| (i % 253) as u8).collect();
        for chunk in [1, 999, 4096, 200_000] {
            let mut window = Window::new(Wanted::Suffix { len: 1000 }, true);
            for piece in file.chunks(chunk) {
                window.push(piece).unwrap();
                assert!(
                    window.buffer.len() <= 2 * 1000 + chunk,
                    "bounded: {}",
                    window.buffer.len()
                );
            }
            assert_eq!(window.streamed_to_end(), Some(100_000));
            assert_eq!(window.into_body(), &file[99_000..], "chunk {chunk}");
        }
        let (body, total) = run(Wanted::Suffix { len: 1000 }, true, &file[..300], 50);
        assert_eq!(body, &file[..300]);
        assert_eq!(total, Some(300));
    }
}
