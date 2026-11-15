#!/bin/sh
# Regenerates tests/fixtures/<scenario>.json from the real Java ApiServer (Javalin 7.2.3, Jetty
# 12.1.12, jjwt 0.13.0, Gson 2.12.1) and the compiled classes in ../../../jphira-mp/build.
# Run from crates/http: tests/java/gen.sh [scenario...]
# Recorded with OpenJDK 27 (Homebrew), user.language=en, user.country=US.
#
# Each scenario runs in its own JVM and its own empty working directory under target/http-gen
# (the Java classes use relative data/ and frontend/dist paths and static state). The working
# directory starts as a copy of tests/fixtures/seed. A local stub (com.sun.net.httpserver)
# answers every Phira request; the real service is never contacted. Logging is made
# synchronous so the log lines of each request can be attributed to it.
#
# tests/java/GenJwt.java (run by this script too) records jjwt tokens and parse results.
set -eu
M2="$HOME/.gradle/caches/modules-2/files-2.1"
JAVA_MAIN=../../../jphira-mp/build/classes/java/main
JAVA_RES=../../../jphira-mp/build/resources/main
CP="$(cd "$JAVA_MAIN" && pwd):$(cd "$JAVA_RES" && pwd):$(cd ../../../jphira-mp/jphira-mp-logger/build/libs && pwd)/jphira-mp-logger.jar"
for jar in \
    com.google.code.gson/gson/2.12.1/*/gson-2.12.1.jar \
    com.github.ben-manes.caffeine/caffeine/3.2.3/*/caffeine-3.2.3.jar \
    com.github.lRENyaaa/jphira-mp-protocol/2.2.1/*/jphira-mp-protocol-2.2.1.jar \
    com.google.guava/guava/33.6.0-jre/*/guava-33.6.0-jre.jar \
    com.lmax/disruptor/4.0.0/*/disruptor-4.0.0.jar \
    io.javalin/javalin/7.2.3/*/javalin-7.2.3.jar \
    io.jsonwebtoken/jjwt-api/0.13.0/*/jjwt-api-0.13.0.jar \
    io.jsonwebtoken/jjwt-gson/0.13.0/*/jjwt-gson-0.13.0.jar \
    io.jsonwebtoken/jjwt-impl/0.13.0/*/jjwt-impl-0.13.0.jar \
    io.netty/netty-buffer/4.2.3.Final/*/netty-buffer-4.2.3.Final.jar \
    io.netty/netty-codec-base/4.2.3.Final/*/netty-codec-base-4.2.3.Final.jar \
    io.netty/netty-codec-haproxy/4.2.3.Final/*/netty-codec-haproxy-4.2.3.Final.jar \
    io.netty/netty-common/4.2.3.Final/*/netty-common-4.2.3.Final.jar \
    io.netty/netty-handler/4.2.3.Final/*/netty-handler-4.2.3.Final.jar \
    io.netty/netty-resolver/4.2.3.Final/*/netty-resolver-4.2.3.Final.jar \
    io.netty/netty-transport/4.2.3.Final/*/netty-transport-4.2.3.Final.jar \
    jakarta.servlet/jakarta.servlet-api/6.0.0/*/jakarta.servlet-api-6.0.0.jar \
    meteordevelopment/orbit/0.2.3/*/orbit-0.2.3.jar \
    net.minecrell/terminalconsoleappender/1.3.0/*/terminalconsoleappender-1.3.0.jar \
    net.sf.jopt-simple/jopt-simple/5.0.4/*/jopt-simple-5.0.4.jar \
    org.apache.logging.log4j/log4j-api/2.25.4/*/log4j-api-2.25.4.jar \
    org.apache.logging.log4j/log4j-core/2.25.4/*/log4j-core-2.25.4.jar \
    org.apache.logging.log4j/log4j-iostreams/2.25.4/*/log4j-iostreams-2.25.4.jar \
    org.apache.logging.log4j/log4j-jul/2.25.4/*/log4j-jul-2.25.4.jar \
    org.apache.logging.log4j/log4j-slf4j2-impl/2.25.4/*/log4j-slf4j2-impl-2.25.4.jar \
    org.eclipse.jetty.ee/jetty-ee-webapp/12.1.12/*/jetty-ee-webapp-12.1.12.jar \
    org.eclipse.jetty.ee10.websocket/jetty-ee10-websocket-jetty-server/12.1.12/*/jetty-ee10-websocket-jetty-server-12.1.12.jar \
    org.eclipse.jetty.ee10.websocket/jetty-ee10-websocket-servlet/12.1.12/*/jetty-ee10-websocket-servlet-12.1.12.jar \
    org.eclipse.jetty.ee10/jetty-ee10-servlet/12.1.12/*/jetty-ee10-servlet-12.1.12.jar \
    org.eclipse.jetty.ee10/jetty-ee10-webapp/12.1.12/*/jetty-ee10-webapp-12.1.12.jar \
    org.eclipse.jetty.websocket/jetty-websocket-core-common/12.1.12/*/jetty-websocket-core-common-12.1.12.jar \
    org.eclipse.jetty.websocket/jetty-websocket-core-server/12.1.12/*/jetty-websocket-core-server-12.1.12.jar \
    org.eclipse.jetty.websocket/jetty-websocket-jetty-api/12.1.12/*/jetty-websocket-jetty-api-12.1.12.jar \
    org.eclipse.jetty.websocket/jetty-websocket-jetty-common/12.1.12/*/jetty-websocket-jetty-common-12.1.12.jar \
    org.eclipse.jetty.websocket/jetty-websocket-jetty-server/12.1.12/*/jetty-websocket-jetty-server-12.1.12.jar \
    org.eclipse.jetty/jetty-http/12.1.12/*/jetty-http-12.1.12.jar \
    org.eclipse.jetty/jetty-io/12.1.12/*/jetty-io-12.1.12.jar \
    org.eclipse.jetty/jetty-security/12.1.12/*/jetty-security-12.1.12.jar \
    org.eclipse.jetty/jetty-server/12.1.12/*/jetty-server-12.1.12.jar \
    org.eclipse.jetty/jetty-session/12.1.12/*/jetty-session-12.1.12.jar \
    org.eclipse.jetty/jetty-util/12.1.12/*/jetty-util-12.1.12.jar \
    org.eclipse.jetty/jetty-xml/12.1.12/*/jetty-xml-12.1.12.jar \
    org.jetbrains.kotlin/kotlin-stdlib/2.2.20/*/kotlin-stdlib-2.2.20.jar \
    org.jline/jline-native/4.1.0/*/jline-native-4.1.0.jar \
    org.jline/jline-reader/3.20.0/*/jline-reader-3.20.0.jar \
    org.jline/jline-terminal-jni/4.1.0/*/jline-terminal-jni-4.1.0.jar \
    org.jline/jline-terminal/4.1.0/*/jline-terminal-4.1.0.jar \
    org.slf4j/slf4j-api/2.0.18/*/slf4j-api-2.0.18.jar \
    ; do
    CP="$CP:$(ls $M2/$jar | head -1)"
done
OUT="$(pwd)/tests/fixtures"
SEED="$(pwd)/tests/fixtures/seed"
GEN="$(pwd)/tests/java/GenHttp.java"
GENJWT="$(pwd)/tests/java/GenJwt.java"
SCRATCH="${SCRATCH:-$(pwd)/../../target/http-gen}"
JOPTS="-Duser.language=en -Duser.country=US -Dlog4j2.contextSelector=org.apache.logging.log4j.core.selector.ClassLoaderContextSelector"
SCENARIOS="${*:-jwt framework pools submissions charts data admin records bind}"
for s in $SCENARIOS; do
    rm -rf "$SCRATCH/$s"
    mkdir -p "$SCRATCH/$s"
    if [ "$s" = jwt ]; then
        # target/http-gen/rust-tokens.tsv: tokens issued by the Rust service, written by
        # `cargo test -p zphira-http --test jwt -- --ignored jwt_write_rust_tokens`.
        RUST_TOKENS="$SCRATCH/rust-tokens.tsv"
        [ -f "$RUST_TOKENS" ] || { echo "missing $RUST_TOKENS (run the jwt_write_rust_tokens test first)"; exit 1; }
        (cd "$SCRATCH/$s" && java $JOPTS -cp "$CP" "$GENJWT" "$OUT" "$RUST_TOKENS" > console.txt 2>&1) \
            || { echo "scenario $s failed"; cat "$SCRATCH/$s/console.txt"; exit 1; }
    else
        (cd "$SCRATCH/$s" && java $JOPTS -cp "$CP" "$GEN" "$s" "$SEED" "$OUT" > console.txt 2>&1) \
            || { echo "scenario $s failed"; tail -50 "$SCRATCH/$s/console.txt"; exit 1; }
    fi
    echo "recorded $s"
done
