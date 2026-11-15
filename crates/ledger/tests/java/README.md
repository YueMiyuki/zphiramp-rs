# Golden vectors for zphira-ledger

Both generators write into `../fixtures/`. The tests in `../*.rs` assert against those files.

## `GenLedgerGson.java`: plain Gson 2.12.1 (needs only the Gson jar)

Writes `gson_points.json`, `gson_records.json`, `gson_admins.json` (and `*_empty.json`): the three
data files as Gson configured like `GsonUtil.getGson()` writes them, from the model shapes of
DESIGN.md section 2 (`RoundRecord`, `PlayerResult`) and of `PointData`.

```sh
M2=~/.gradle/caches/modules-2/files-2.1
GSON=$(find $M2/com.google.code.gson/gson/2.12.1 -name gson-2.12.1.jar)
java -cp "$GSON" tests/java/GenLedgerGson.java tests/fixtures
```

## `GenLedger.java`: the real server classes

Runs the compiled `PlayerPointService`, `RoundRecordService`, `AdminService` and
`DataTransferService` of `../jphira-mp` (build output, read-only) with the real Gson, one JVM per
mode because every store is a static singleton. Writes `ledger_points.tsv`, `ledger_records.tsv`,
`ledger_admins.tsv` (one row per step: label, then fields, escaped by `Esc.java`) and the raw files
the Java server wrote (`points_java.json`, `records_java.json`, `admins_java.json`,
`points_seed.json`). The stores read and write `data/` relative to the working directory, so run it
in an empty scratch directory (never inside `jphira-mp`):

```sh
CP=...   # jphira-mp/build/classes/java/main:jphira-mp/build/resources/main: plus its dependency jars
OUT=$PWD/tests/fixtures
for mode in points records admins; do
  d=$(mktemp -d) && (cd "$d" && java -cp "$CP" $OLDPWD/tests/java/GenLedger.java $mode "$OUT")
done
```

Recorded with OpenJDK 27 (Homebrew build) and Gson 2.12.1.
