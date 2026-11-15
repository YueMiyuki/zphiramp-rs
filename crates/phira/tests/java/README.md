# Golden-vector generators for zphira-phira

Both generators run the **real compiled classes** of the Java server (`jphira-mp/build/classes`,
read-only) and write into `../fixtures/`. The Rust tests in `../*.rs` assert against those files.
Rows are TAB separated and escaped by `Esc.java` (reversed by `../support/mod.rs`).

Recorded with OpenJDK 27 (Homebrew build), Gson 2.12.1, Caffeine 3.2.3, `-Duser.timezone=UTC`
(the zip entry timestamps follow the zone, nothing else does).

```sh
cd crates/phira
JP=/path/to/jphira-mp
M2=~/.gradle/caches/modules-2/files-2.1
GSON=$(find $M2/com.google.code.gson/gson/2.12.1 -name gson-2.12.1.jar)
CAFFEINE=$(find $M2/com.github.ben-manes.caffeine/caffeine/3.2.3 -name caffeine-3.2.3.jar)
PROTO=$(find $M2/com.github.lRENyaaa/jphira-mp-protocol/2.2.1 -name jphira-mp-protocol-2.2.1.jar)
CP="$JP/build/classes/java/main:$GSON:$CAFFEINE:$PROTO"
OUT=$(mktemp -d)                     # scratch class output, never inside jphira-mp

javac -cp "$CP" -d "$OUT" tests/java/GenDuration.java tests/java/GenPhiraGson.java tests/java/GenHeaders.java tests/java/Esc.java
java -Duser.timezone=UTC -cp "$OUT:$CP" top.rymc.phira.main.util.GenDuration  tests/fixtures
java -Duser.timezone=UTC -cp "$OUT:$CP" top.rymc.phira.main.util.GenPhiraGson tests/fixtures
java -Duser.timezone=UTC -cp "$OUT:$CP" top.rymc.phira.main.util.GenHeaders   tests/fixtures
```

Each run takes about one second. Both mains end with `System.exit` and start a daemon watchdog that
halts the JVM after 120 s, so nothing can hang; wrap the commands in `timeout` where it exists.

## `GenDuration.java` -> `duration_parse.tsv`, `duration_probe.tsv`, `zip_*.zip`, `bare_*.bin`

* `duration_parse.tsv`: crafted inputs for `parseMp3Duration`, `parseOggDuration` and
  `parseWavDuration` (package-private in Java, reachable because the generator lives in the same
  package). Columns: category, kind, declared size, input (hex), Java result, note.
* `duration_probe.tsv`: `ChartDuration.probe(url)` against a local `com.sun.net.httpserver` that
  answers `/files/<key>` with `303` to `/cdn/<key>?sign=abc`, which honours `Range` (206 with
  `Content-Range`) like the real CDN. Columns: key, category, payload, Java result.
* Categories: `same` Java is right and Rust must return the same value; `exc` Java threw (Rust
  must not panic and fails the probe); `A-02`, `A-14`, `A-17`, `A-18`, `A-27`, `A-28` Java is wrong
  for the reason named in `audit/java-admin.md`: the recorded value is Java's flawed result, the
  Rust test asserts the corrected one next to it. `N-29` (WAV walk stops at an empty chunk), `N-30`
  (MP3 scan gives up at the first sync pattern; Xing tag in a CRC protected frame) and `N-31`
  (ZIP walk takes a `__MACOSX/._x.mp3` stub) are the flaws of `audit/decisions.md` found during the
  port; `O-07` (the archive's `info.yml`/`info.txt` names the song, Java measures the first `.mp3`/`.ogg`, here a preview in front of the song) is the owner's change from `audit/owner-decisions.md`; in the N-30 rows of the parse table the note ends in `expect=<value>`, taken from the real
  Java parser run on the same bytes without the decoy where there is one.

## Real encoders: `make_real_audio.sh` -> `fixtures/real/`, `real_audio.tsv`

`make_real_audio.sh` (needs `lame`, `ffmpeg`, `ffprobe`; run from `crates/phira`) encodes a synthetic
24 s signal with LAME 4.0 (MPEG-1, MPEG-2 and MPEG-2.5, VBR with Xing, CBR with and without the
Info tag, an ID3v2 cover larger than the first window) and ffmpeg 9.0 (Vorbis, WAV), and keeps the
first bytes of every file in `fixtures/real/*.head` with its total size and ffprobe's duration in
`fixtures/real/manifest.tsv`. It is recorded once; the tests do not run the encoders. After adding
cases, rerun `GenDuration`: it reads the manifest and writes `real_audio.tsv` (the real Java
parsers' result per file) and the `zip_real_cover` probe row. `tests/real_audio.rs` compares the
Rust parsers with ffprobe's duration and with Java's.

## `GenHeaders.java` -> `request_headers.tsv`

The real `PhiraFetcher` (all request kinds) and `ChartDuration.probe` against a local server that
dumps the request headers it receives; `tests/request_headers.rs` makes the same calls with the Rust
client and compares header names and values (N-32). Over plain http the JDK client adds its
cleartext HTTP/2 upgrade trio (`connection`, `upgrade`, `http2-settings`); the test ignores those
and the `host` value.

## `tests/fixtures/tls/`

A test CA and a `localhost`/`127.0.0.1` leaf (EC P-256, DER) for the TLS tests of
`src/duration/http.rs`, made once with `openssl ecparam`, `req -x509` (CA:TRUE), `req -new` and
`x509 -req` with SAN `DNS:localhost,IP:127.0.0.1`, `-days 36500`, then `-outform DER` and
`pkcs8 -topk8 -nocrypt -outform DER` for the key. The tests trust the CA through a WebPKI verifier
(the macOS platform verifier refuses leaf certificates valid for more than 398 days); the production
verifier is exercised by a test that it rejects these certificates.

## `GenPhiraGson.java` -> `gson_models.tsv`

`GsonUtil.getGson()` / `getCompactGson()` reading and writing the real `UserInfo`, `ChartInfo`,
`GameRecord`, `PhiraFetcher.LoginResult`, `PhiraFetcher.ChartListPage` and `GameRecord[]` from
hand-written documents (shapes of `Phira-API.md`, alternate names, nulls, lenient syntax) and
random ones (fixed seed) that mix key spellings, Gson's coercions and a few invalid values.
Columns: kind, label, input, outcome (`ok`, `null`, `EXC <class>` when reading throws, `WRITE_EXC <class>` when the document reads but writing it back throws), pretty output, compact output.
Java records reject `null` for a primitive component (`JsonParseException`); the Rust structs keep
the default (DESIGN.md 4.2), so the random documents avoid it and two hand-written rows document it.
