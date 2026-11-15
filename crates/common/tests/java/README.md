# Golden-vector generators

Each generator runs the real Java library and writes fixtures to `../fixtures/`. The Rust
tests in `../*.rs` assert against those files. Regenerate from `crates/common`:

```sh
M2=~/.gradle/caches/modules-2/files-2.1
GSON=$(find $M2/com.google.code.gson/gson/2.12.1 -name gson-2.12.1.jar)
JOPT=$(find $M2/net.sf.jopt-simple/jopt-simple/5.0.4 -name jopt-simple-5.0.4.jar)

java tests/java/GenNumbers.java tests/fixtures          # Float/Double.toString, %.Nf, parseDouble/parseInt
java tests/java/GenTime.java tests/fixtures             # OffsetDateTime parse/toString/compareTo
java -cp "$GSON" tests/java/GenGson.java tests/fixtures # Gson 2.12.1 configured like GsonUtil
java -cp "$JOPT" tests/java/GenArgs.java tests/fixtures # jopt-simple 5.0.4 configured like ServerArgs
```

Recorded with OpenJDK 27 (Homebrew build), `user.language=en`, `user.country=US`.

Format: one record per line, fields separated by TAB, escaped by `Esc.java` (reversed by
`../support/mod.rs`).

The Java model classes in `GenGson.java` are mirrored by the Rust structs in `../gson.rs`;
keep them in sync. `GenArgs.java` copies the parser construction and field reads of
`config/ServerArgs.java`; its help output was checked byte for byte against the real
`Server --help`.

`args.tsv` records Java's behaviour for a bare `--proxy-protocol` (false). The Rust parser
deliberately returns true there (finding G-17); `../args.rs` applies that adjustment.
`GenArgs.java` also declares `--max-pending-submissions` and `--login-rate-limit` (owner
decisions O-01, O-02), which the Java server does not have, so the real jopt-simple decides
their parsing, the prefixes they affect and the help layout; their value validation in the
generator follows the owner's specification.

The log rollover rules in `src/logging/rolling.rs` were recorded by running the real server
(`top.rymc.phira.main.Server --help`, which initialises log4j with the real `log4j2.xml`) in
a scratch directory, nine times in a row and with a back-dated, an empty and a gapped
`logs/` directory; they are asserted by the unit tests in that file rather than by a fixture.
One rule was changed on purpose (owner decision O-04): log4j keeps at most 7 archives per
date and deletes the oldest; the Rust appender never deletes or renumbers archives.
