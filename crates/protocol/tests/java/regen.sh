#!/bin/sh
# Regenerates crates/protocol/tests/fixtures/ from the real jphira-mp-protocol 2.2.1 jar, running on the
# Netty 4.2.3.Final jars the server ships with (taken from the Gradle cache).
#
#   crates/protocol/tests/java/regen.sh
#
# Needs `java` and `javac` (17+) and a populated Gradle cache (run the Java server's build once).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cache=${GRADLE_USER_HOME:-$HOME/.gradle}/caches/modules-2/files-2.1

jar() { # group/artifact/version, file name
    found=$(find "$cache/$1" -name "$2" ! -name '*-sources.jar' ! -name '*-javadoc.jar' 2>/dev/null | head -n 1)
    [ -n "$found" ] || { echo "missing jar $1/$2 in $cache" >&2; exit 1; }
    echo "$found"
}

NETTY=4.2.3.Final
cp=$(jar com.github.lRENyaaa/jphira-mp-protocol/2.2.1 jphira-mp-protocol-2.2.1.jar)
for m in netty-buffer netty-common netty-codec-base netty-transport netty-resolver; do
    cp="$cp:$(jar io.netty/$m/$NETTY $m-$NETTY.jar)"
done

out="$root/target/protocol-gen/classes"
mkdir -p "$out"
javac -encoding UTF-8 -cp "$cp" -d "$out" "$here"/*.java
# The fixed heap makes the "huge declared list length" cases repeatable: Java allocates the list
# before reading any element, so such an input ends in OutOfMemoryError or in a decode error
# depending on the heap. With 3g, declared counts up to about 2^29 get as far as reading elements;
# the generator records anything larger as OutOfMemoryError (outcome code M), which the Rust tests
# treat as "some failure".
java -Xmx3g -Dfile.encoding=UTF-8 -cp "$cp:$out" GenFixtures "$here/../fixtures"
