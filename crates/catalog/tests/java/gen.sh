#!/bin/sh
# Regenerates tests/fixtures from the real compiled Java classes (ChartPool, ChartIndex,
# SubmissionService, DataTransferService in ../../../jphira-mp/build/classes, Gson 2.12.1 and
# log4j from ~/.gradle). Run from crates/catalog: tests/java/gen.sh [scenario...]
# Recorded with OpenJDK 27 (Homebrew), user.language=en, user.country=US.
#
# Each scenario runs in its own JVM and its own empty working directory under
# target/catalog-gen (the Java classes use relative data/ paths and static state). A local
# stub (com.sun.net.httpserver) answers every Phira request; the real service is never
# contacted. Per scenario: input/ (files given to Java), output/ (files Java wrote),
# result.txt (key<TAB>value), log.txt (LEVEL message), stub.tsv (pages the stub served).
# The Rust tests replay the same inputs and operations and compare.
set -eu
M2="$HOME/.gradle/caches/modules-2/files-2.1"
JAVA_MAIN=../../../jphira-mp/build/classes/java/main
JAVA_RES=../../../jphira-mp/build/resources/main
CP="$(cd "$JAVA_MAIN" && pwd):$(cd "$JAVA_RES" && pwd):$(cd ../../../jphira-mp/jphira-mp-logger/build/libs && pwd)/jphira-mp-logger.jar"
for jar in \
    com.google.code.gson/gson/2.12.1/*/gson-2.12.1.jar \
    com.github.ben-manes.caffeine/caffeine/3.2.3/*/caffeine-3.2.3.jar \
    com.github.lRENyaaa/jphira-mp-protocol/2.2.1/*/jphira-mp-protocol-2.2.1.jar \
    org.apache.logging.log4j/log4j-api/2.25.4/*/log4j-api-2.25.4.jar \
    org.apache.logging.log4j/log4j-core/2.25.4/*/log4j-core-2.25.4.jar \
    org.apache.logging.log4j/log4j-iostreams/2.25.4/*/log4j-iostreams-2.25.4.jar \
    org.apache.logging.log4j/log4j-jul/2.25.4/*/log4j-jul-2.25.4.jar \
    net.minecrell/terminalconsoleappender/1.3.0/*/terminalconsoleappender-1.3.0.jar \
    net.sf.jopt-simple/jopt-simple/5.0.4/*/jopt-simple-5.0.4.jar \
    com.lmax/disruptor/4.0.0/*/disruptor-4.0.0.jar \
    io.netty/netty-common/4.2.3.Final/*/netty-common-4.2.3.Final.jar \
    io.netty/netty-transport/4.2.3.Final/*/netty-transport-4.2.3.Final.jar \
    io.netty/netty-buffer/4.2.3.Final/*/netty-buffer-4.2.3.Final.jar \
    io.netty/netty-resolver/4.2.3.Final/*/netty-resolver-4.2.3.Final.jar \
    meteordevelopment/orbit/0.2.3/*/orbit-0.2.3.jar \
    com.google.guava/guava/33.6.0-jre/*/guava-33.6.0-jre.jar \
    org.jline/jline-terminal/4.1.0/*/jline-terminal-4.1.0.jar \
    org.jline/jline-reader/3.20.0/*/jline-reader-3.20.0.jar \
    org.jline/jline-native/4.1.0/*/jline-native-4.1.0.jar \
    org.jline/jline-terminal-jni/4.1.0/*/jline-terminal-jni-4.1.0.jar \
    ; do
    CP="$CP:$(ls $M2/$jar | head -1)"
done
OUT="$(pwd)/tests/fixtures"
GEN="$(pwd)/tests/java/GenCatalog.java"
SCRATCH="${SCRATCH:-$(pwd)/../../target/catalog-gen}"
SCENARIOS="${*:-pool_defaults pool_normalize pool_load_edges pool_ops index_save index_unreadable sweep_fresh sweep_fresh_no_plain sweep_incremental sweep_division sweep_empty_page submissions_ops submissions_odd submissions_null_row import_pools import_submissions}"
for s in $SCENARIOS; do
    rm -rf "$SCRATCH/$s" "$OUT/$s"
    mkdir -p "$SCRATCH/$s"
    (cd "$SCRATCH/$s" && java -Duser.language=en -Duser.country=US -cp "$CP" "$GEN" "$s" "$OUT" > console.txt 2>&1) \
        || { echo "scenario $s failed"; cat "$SCRATCH/$s/console.txt"; exit 1; }
    echo "recorded $s"
done
