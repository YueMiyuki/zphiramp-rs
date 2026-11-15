# zphira-phira public API

## Models (`zphira_phira::model`, re-exported at the root)

All derive `Debug, Clone, Serialize, Deserialize` with `#[serde(default)]`. Nullable fields are
`Option`. JSON keys are the snake_case field names, except `lastLogin`, `loginBanned`,
`followerCount`, `followingCount`, `stableRequest`, `ratingCount`, `chartUpdated`, `bestStd`,
`fullCombo` and `stdScore`.

```rust
pub struct UserInfo {
    pub id: i32, pub name: Option<String>, pub avatar: Option<String>,
    pub language: Option<String>, pub bio: Option<String>, pub exp: i32, pub rks: f64,
    pub joined: Option<JavaOffsetDateTime>, pub last_login: Option<JavaOffsetDateTime>,
    pub roles: i32, pub banned: bool, pub login_banned: bool,
    pub follower_count: i32, pub following_count: i32, pub email: Option<String>,
}
impl UserInfo { pub fn name_or_null(&self) -> &str }

pub struct ChartInfo {
    pub id: i32, pub name: Option<String>, pub level: Option<String>, pub difficulty: f32,
    pub charter: Option<String>, pub composer: Option<String>, pub illustrator: Option<String>,
    pub description: Option<String>, pub ranked: bool, pub reviewed: bool, pub stable: bool,
    pub stable_request: bool, pub illustration: Option<String>, pub preview: Option<String>,
    pub file: Option<String>, pub uploader: i32, pub tags: Option<Vec<String>>,
    pub rating: f32, pub rating_count: i32,
    pub created: Option<JavaOffsetDateTime>, pub updated: Option<JavaOffsetDateTime>,
    pub chart_updated: Option<JavaOffsetDateTime>,
    /* private */ duration_seconds: DurationCell,
}
impl ChartInfo {
    pub fn name_or_null(&self) -> &str;
    pub fn level_or_null(&self) -> &str;
    pub fn has_tag(&self, tag: &str) -> bool;
    pub fn duration_seconds(&self) -> Option<i32>;
    pub fn set_duration_seconds(&self, value: Option<i32>);   // &self: shared through Arc
}

pub struct GameRecord {
    pub id: i32, pub player: i32, pub chart: i32, pub score: i32, pub accuracy: f32,
    pub perfect: i32, pub good: i32, pub bad: i32, pub miss: i32, pub speed: f32,
    pub max_combo: i32, pub best: bool, pub best_std: bool, pub mods: i32,
    pub full_combo: bool, pub time: Option<JavaOffsetDateTime>, pub std: f32, pub std_score: f32,
}

pub struct LoginResult { pub id: i32, pub token: Option<String>,
                         pub refresh_token: Option<String>, pub expire_at: Option<String> }
pub struct ChartListPage { pub count: i32, pub results: Option<Vec<Arc<ChartInfo>>> }
```

- `duration_seconds` is written as the optional integer `duration_seconds` (left out by the pretty
  writer when unknown, `null` in the compact one) and cloned by value.
- `GameRecord` is read with a hand-written `Deserialize`: `bestStd`/`best_std`,
  `fullCombo`/`full_combo` and `stdScore`/`std_score` are the same field and the last key wins.
  `max_combo` is read from `max_combo` only. It is written as `bestStd`, `fullCombo`, `stdScore`.
- A `null` element inside `tags` or `ChartListPage::results` makes the document a decode error.

## Client

```rust
pub const DEFAULT_BASE_URL: &str = "https://phira.5wyxi.com/";

#[derive(Clone)]
pub struct PhiraClient { .. }

impl PhiraClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, PhiraError>;     // adds a missing trailing `/`
    pub fn with_options(base_url: impl Into<String>, options: Options) -> Result<Self, PhiraError>;

    pub async fn user_by_token(&self, token: &str) -> Result<Arc<UserInfo>, PhiraError>;
    pub async fn user_by_id(&self, id: i32) -> Result<Arc<UserInfo>, PhiraError>;
    pub async fn chart(&self, id: i32) -> Result<Arc<ChartInfo>, PhiraError>;
    pub async fn record(&self, id: i32) -> Result<Arc<GameRecord>, PhiraError>;
    pub async fn recent_records(&self, player: i32) -> Result<Arc<Vec<GameRecord>>, PhiraError>;
    pub async fn login(&self, email: &str, password: &str) -> Result<LoginResult, PhiraError>;
    pub async fn chart_page(&self, page: i32, division: Option<&str>) -> Result<ChartListPage, PhiraError>;
    pub async fn probe_duration(&self, chart_id: i32) -> Result<i32, PhiraError>;
}

pub struct PhiraError { .. }
impl PhiraError {
    pub fn status(&self) -> Option<u16>;       // HTTP status of a non-2xx answer
    pub fn body(&self) -> Option<&str>;        // its body
    pub fn is_transport(&self) -> bool;        // no complete answer arrived
}

#[derive(Debug, Clone)]
pub struct Options {
    pub max_attempts: u32,            // 8
    pub retry_delay: Duration,        // 150 ms
    pub connect_timeout: Duration,    // 5 s
    pub request_timeout: Duration,    // 10 s
    pub probe_timeout: Duration,      // 20 s, each range request of the probe
    pub recent_record_ttl: Duration,  // 10 s
}
```

- Caches (successes only): token 10 min / 10000, user 10 min / 5000, chart 30 min / 10000,
  record 30 min / 50000, recent records 10 s / 5000, probed durations 6 h / 20000.
- A non-2xx answer is a `PhiraError` whose `Display` is `HTTP <status>: <body>`.
- Transport errors, 5xx and 429 are retried up to `max_attempts` times, waiting `retry_delay * n`
  after the n-th failure; any other status fails at once. Probe requests are not retried.
- Redirects are followed up to four times and never from https to http; the 3xx answer is then
  the error.
- API requests send `User-Agent: JPhira/1` and `Accept: application/json`, plus
  `Authorization: Bearer <token>` for `user_by_token` and a JSON body for `login`. The probe's
  range requests send `User-Agent` and `Range` only.

## Chart duration (`zphira_phira::duration`)

`probe_duration` reads the head, then the tail of the archive (central directory), then the info
file's data when the archive has one and an audio entry to choose from, then the song's first
window (and a larger one when the first does not parse). A bare audio file takes the head only.
The song is the file named by the archive's `info.yml` (`music:`) or, without it, `info.txt`
(`Music:` or `Song:`) at the root or in the single top-level folder, matched exactly and then in
any case; with no usable info file the first `.mp3` or `.ogg` entry outside `__MACOSX/` and `._*`.
A named file the prober cannot time (anything but `.mp3`/`.ogg`) fails the probe.

The parsing is pure, takes byte slices and never panics:

```rust
pub const TAIL_BYTES: usize = 512 * 1024;
pub const AUDIO_PROBE_BYTES: usize = 32 * 1024;
pub const AUDIO_FULL_BYTES: usize = 512 * 1024;
pub const INFO_MAX_BYTES: u64 = 64 * 1024;
pub const METHOD_STORED: u16 = 0;
pub const METHOD_DEFLATE: u16 = 8;

pub struct ZipEntry { pub name: String /* lower case */, pub path: String, pub local_offset: u64,
                      pub size: u64, pub compressed_size: u64, pub method: u16 }
impl ZipEntry { pub fn is_audio(&self) -> bool; pub fn is_directory(&self) -> bool;
                pub fn needs_zip64(&self) -> bool }

pub fn is_zip(head: &[u8]) -> bool;
pub fn list_entries(tail: &[u8]) -> Vec<ZipEntry>;
pub fn first_audio_entry(entries: &[ZipEntry]) -> Option<&ZipEntry>;
pub fn find_audio_entry(tail: &[u8]) -> Option<ZipEntry>;
pub fn inflate_entry(raw: &[u8], entry: &ZipEntry, limit: usize) -> Vec<u8>;

pub enum InfoFormat { Yml, Txt }
pub struct InfoFile { pub entry: ZipEntry, pub format: InfoFormat }
pub enum SongChoice { Entry(ZipEntry), Unsupported(ZipEntry), NoAudio }
pub fn find_info_file(entries: &[ZipEntry]) -> Option<InfoFile>;
pub fn info_music_range(info: &InfoFile) -> Option<(u64, usize)>;
pub fn read_info_music(info: &InfoFile, raw: &[u8]) -> Option<String>;
pub fn parse_info_music(format: InfoFormat, bytes: &[u8]) -> Option<String>;
pub fn parse_info_yml_music(bytes: &[u8]) -> Option<String>;
pub fn parse_info_txt_music(bytes: &[u8]) -> Option<String>;
pub fn music_entry(entries: &[ZipEntry], info: &InfoFile, music: &str) -> Option<ZipEntry>;
pub fn select_song(entries: &[ZipEntry], info: Option<&InfoFile>, music: Option<&str>) -> SongChoice;

pub fn parse_audio_entry(entry: &ZipEntry, audio: &[u8]) -> Option<i32>;
pub fn parse_bare_audio(head: &[u8], total_size: u64) -> Option<i32>;     // total_size 0 = unknown
pub fn parse_mp3_duration(audio: &[u8], declared_size: u64) -> Option<i32>;
pub fn parse_ogg_duration(data: &[u8], declared_size: u64) -> Option<i32>;
pub fn parse_wav_duration(data: &[u8]) -> Option<i32>;
```
