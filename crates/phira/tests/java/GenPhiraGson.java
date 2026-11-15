package top.rymc.phira.main.util;

import top.rymc.phira.main.data.ChartInfo;
import top.rymc.phira.main.data.GameRecord;
import top.rymc.phira.main.data.UserInfo;

import java.io.PrintWriter;
import java.lang.reflect.Type;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Random;

/**
 * Golden vectors for {@code zphira_phira::model}: the real compiled model classes of the Java
 * server (UserInfo, ChartInfo, GameRecord, PhiraFetcher.LoginResult, PhiraFetcher.ChartListPage)
 * read and written by the real {@code GsonUtil} instances (Gson 2.12.1, naming policy
 * LOWER_CASE_WITH_UNDERSCORES, lenient, OffsetDateTime adapters).
 *
 * <p>Writes {@code gson_models.tsv}: one row per input document, TAB separated and escaped by
 * {@code Esc}: {@code kind, label, input, outcome, pretty, compact} where
 * <ul>
 *   <li>{@code kind}: user, chart, record, login, page, records (a {@code GameRecord[]}, as the
 *       recent-records call reads it);</li>
 *   <li>{@code outcome}: {@code ok} (then {@code pretty} is {@code GsonUtil.getGson().toJson} of
 *       the parsed object, {@code compact} is {@code getCompactGson().toJson}), {@code null} (Gson
 *       returned null), {@code EXC <class>} (Gson threw while reading) or {@code WRITE_EXC <class>}
 *       (it read the document but threw while writing the object back: a float read as infinity).</li>
 * </ul>
 *
 * <p>Run (see README.md): {@code java -cp $CP GenPhiraGson.java ...}; compile together with Esc.java.
 */
public final class GenPhiraGson {

    static final Random R = new Random(20261116L);
    static PrintWriter out;
    /**
     * Java records (LoginResult, ChartListPage) reject {@code null} for a primitive component
     * with a JsonParseException; the Rust structs keep the default instead (DESIGN.md 4.2), so the
     * random documents for them never carry one. A hand-written row records the Java behaviour.
     */
    static boolean recordInts;

    // ------------------------------------------------------------------ rows

    static void row(String kind, String label, String input) {
        Type type = switch (kind) {
            case "user" -> UserInfo.class;
            case "chart" -> ChartInfo.class;
            case "record" -> GameRecord.class;
            case "login" -> PhiraFetcher.LoginResult.class;
            case "page" -> PhiraFetcher.ChartListPage.class;
            case "records" -> GameRecord[].class;
            default -> throw new IllegalArgumentException(kind);
        };
        String outcome;
        String pretty = "";
        String compact = "";
        Object value = null;
        try {
            value = GsonUtil.getGson().fromJson(input, type);
            outcome = value == null ? "null" : "ok";
        } catch (Throwable t) {
            outcome = "EXC " + t.getClass().getSimpleName();
        }
        if (outcome.equals("ok")) {
            try {
                pretty = GsonUtil.getGson().toJson(value, type);
                compact = GsonUtil.getCompactGson().toJson(value, type);
            } catch (Throwable t) {
                // reading worked, writing did not (a float that overflowed to infinity)
                outcome = "WRITE_EXC " + t.getClass().getSimpleName();
                pretty = "";
                compact = "";
            }
        }
        out.println(kind + "\t" + Esc.esc(label) + "\t" + Esc.esc(input) + "\t" + Esc.esc(outcome) + "\t"
                + Esc.esc(pretty) + "\t" + Esc.esc(compact));
    }

    // ------------------------------------------------------------------ random documents

    enum K { INT, FLOAT, DOUBLE, BOOL, STRING, DATE, TAGS, DURATION, CHARTS }

    record F(String[] keys, K kind) {
        static F of(K kind, String... keys) {
            return new F(keys, kind);
        }
    }

    static String pick(String... options) {
        return options[R.nextInt(options.length)];
    }

    /** Values Gson accepts for the kind (including its coercions), and a few it rejects. */
    static String value(K kind) {
        return value(kind, R.nextInt(70) == 0);
    }

    static String value(K kind, boolean bad) {
        return switch (kind) {
            case INT, DURATION -> bad ? pick("2147483648", "\"abc\"", "12.5", "true", "\"\"")
                    : pick("0", "5", "-7", "123456", "2147483647", "-2147483648", "\"12\"", "\"-3\"", "12.0", "1e2",
                            recordInts ? "8" : "null", recordInts ? "6" : "null", "7", "9");
            case FLOAT -> bad ? pick("\"x\"", "true", "1e40")
                    : pick("0", "0.5", "3", "4.2", "0.84", "1e-4", "12345678.9", "3.4028235E38", "\"2.5\"", "null",
                            "-0.0", "0.1", "16.3", "100.0", "1.17549435E-38");
            case DOUBLE -> bad ? pick("\"y\"", "false")
                    : pick("0", "12.34", "15.123456789", "1e-7", "123456789012", "\"7.5\"", "null", "0.1", "3",
                            "1.7976931348623157E308", "-5e-324");
            case BOOL -> bad ? pick("1", "0", "[]")
                    : pick("true", "false", "\"true\"", "\"TRUE\"", "\"false\"", "\"x\"", "null", "true", "false");
            case STRING -> bad ? pick("[]", "{}")
                    : pick("\"abc\"", "\"\"", "null", "12", "1.50", "true", "\"<b>&'=\"",
                            "\"\\u00e9\\u4e2d\\ud83d\\ude00\"", "\"line\\nbreak\\ttab\"",
                            "\"https://example.com/a?b=c&d=e\"", "\"AT Lv.17\"", "\"a\\\"b\\\\c\"",
                            "\"\\u2028\\u2029\"", "\"x\"");
            case DATE -> bad ? pick("\"2026-02-30T00:00:00Z\"", "\"bad\"", "12")
                    : pick("\"2026-10-01T12:34:56Z\"", "\"2026-10-01T12:34:56.123456789+08:00\"",
                            "\"2026-10-01T12:34Z\"", "null", "\"2026-10-01T12:34:56.1Z\"",
                            "\"2026-10-01T12:34:56-05:30\"", "\"2024-02-29T23:59:59.999999999Z\"",
                            "\"2026-10-01T12:34:56+00:00\"", "\"2026-10-01T12:34:56.120Z\"");
            case TAGS -> bad ? pick("\"regular\"", "[1,2]")
                    : pick("[\"regular\"]", "[\"plain\",\"regular\"]", "[]", "null", "[\"\"]", "[\"a\\u00e9\",\"<x>\"]",
                            "[\"x\"]");
            case CHARTS -> {
                int n = R.nextInt(4);
                if (R.nextInt(6) == 0) {
                    yield "null";
                }
                StringBuilder b = new StringBuilder("[");
                for (int i = 0; i < n; i++) {
                    if (i > 0) {
                        b.append(',');
                    }
                    b.append(document(CHART));
                }
                yield b.append(']').toString();
            }
        };
    }

    static final List<F> USER = List.of(
            F.of(K.INT, "id"), F.of(K.STRING, "name"), F.of(K.STRING, "avatar"), F.of(K.STRING, "language"),
            F.of(K.STRING, "bio"), F.of(K.INT, "exp"), F.of(K.DOUBLE, "rks"), F.of(K.DATE, "joined"),
            F.of(K.DATE, "lastLogin", "last_login"), F.of(K.INT, "roles"), F.of(K.BOOL, "banned"),
            F.of(K.BOOL, "loginBanned", "login_banned"), F.of(K.INT, "followerCount", "follower_count"),
            F.of(K.INT, "followingCount", "following_count"), F.of(K.STRING, "email"));

    static final List<F> CHART = List.of(
            F.of(K.INT, "id"), F.of(K.STRING, "name"), F.of(K.STRING, "level"), F.of(K.FLOAT, "difficulty"),
            F.of(K.STRING, "charter"), F.of(K.STRING, "composer"), F.of(K.STRING, "illustrator"),
            F.of(K.STRING, "description"), F.of(K.BOOL, "ranked"), F.of(K.BOOL, "reviewed"), F.of(K.BOOL, "stable"),
            F.of(K.BOOL, "stableRequest", "stable_request"), F.of(K.STRING, "illustration"), F.of(K.STRING, "preview"),
            F.of(K.STRING, "file"), F.of(K.INT, "uploader"), F.of(K.TAGS, "tags"), F.of(K.FLOAT, "rating"),
            F.of(K.INT, "ratingCount", "rating_count"), F.of(K.DATE, "created"), F.of(K.DATE, "updated"),
            F.of(K.DATE, "chartUpdated", "chart_updated"), F.of(K.DURATION, "duration_seconds", "durationSeconds"));

    static final List<F> RECORD = List.of(
            F.of(K.INT, "id"), F.of(K.INT, "player"), F.of(K.INT, "chart"), F.of(K.INT, "score"),
            F.of(K.FLOAT, "accuracy"), F.of(K.INT, "perfect"), F.of(K.INT, "good"), F.of(K.INT, "bad"),
            F.of(K.INT, "miss"), F.of(K.FLOAT, "speed"), F.of(K.INT, "max_combo", "maxCombo"), F.of(K.BOOL, "best"),
            F.of(K.BOOL, "bestStd", "best_std"), F.of(K.INT, "mods"), F.of(K.BOOL, "fullCombo", "full_combo"),
            F.of(K.DATE, "time"), F.of(K.FLOAT, "std"), F.of(K.FLOAT, "stdScore", "std_score"));

    static final List<F> LOGIN = List.of(
            F.of(K.INT, "id"), F.of(K.STRING, "token"), F.of(K.STRING, "refresh_token", "refreshToken"),
            F.of(K.STRING, "expire_at", "expireAt"));

    static final List<F> PAGE = List.of(F.of(K.INT, "count"), F.of(K.CHARTS, "results"));

    /** A document for one of the field lists. Keys are mostly the right ones, sometimes the other style. */
    static String document(List<F> fields) {
        StringBuilder b = new StringBuilder("{");
        boolean first = true;
        for (F f : fields) {
            if (R.nextInt(8) == 0) {
                continue; // key absent
            }
            // A repeated key: Gson reads both occurrences in order, the later one wins. Both copies
            // are valid values (and the later one is not null for a primitive, which Gson ignores),
            // so the row tests "last wins" and not the order in which errors surface.
            int copies = R.nextInt(25) == 0 && f.kind() != K.CHARTS ? 2 : 1;
            boolean reference = f.kind() == K.STRING || f.kind() == K.DATE || f.kind() == K.TAGS;
            for (int c = 0; c < copies; c++) {
                String key = f.keys()[R.nextInt(4) == 0 ? R.nextInt(f.keys().length) : 0];
                if (!first) {
                    b.append(',');
                }
                first = false;
                String v;
                if (copies == 1) {
                    v = value(f.kind());
                } else {
                    do {
                        v = value(f.kind(), false);
                    } while (v.equals("null") && !reference);
                }
                b.append('"').append(key).append("\":").append(v);
            }
        }
        if (R.nextInt(5) == 0) {
            if (!first) {
                b.append(',');
            }
            b.append("\"unknown_key\":{\"a\":[1,2,{\"b\":null}]}");
        }
        return b.append('}').toString();
    }

    // ------------------------------------------------------------------ main

    public static void main(String[] args) throws Exception {
        Thread watchdog = new Thread(() -> {
            try {
                Thread.sleep(120_000);
            } catch (InterruptedException e) {
                return;
            }
            System.err.println("GenPhiraGson: watchdog fired");
            Runtime.getRuntime().halt(3);
        }, "watchdog");
        watchdog.setDaemon(true);
        watchdog.start();

        Path dir = Path.of(args[0]);
        Files.createDirectories(dir);
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(dir.resolve("gson_models.tsv"), StandardCharsets.UTF_8))) {
            out = w;
            handWritten();
            for (int i = 0; i < 80; i++) {
                row("user", "random " + i, document(USER));
                row("chart", "random " + i, document(CHART));
                row("record", "random " + i, document(RECORD));
                recordInts = true;
                row("login", "random " + i, document(LOGIN));
                row("page", "random " + i, document(PAGE));
                recordInts = false;
            }
            for (int i = 0; i < 40; i++) {
                int n = R.nextInt(4);
                List<String> docs = new ArrayList<>();
                for (int j = 0; j < n; j++) {
                    docs.add(document(RECORD));
                }
                row("records", "random " + i, "[" + String.join(",", docs) + "]");
            }
        }
        System.exit(0);
    }

    static void handWritten() {
        // Documents shaped like the real service (Phira-API.md of the Java repository).
        row("user", "real shape", "{\"id\":1001,\"name\":\"Alice\",\"avatar\":\"https://cdn.example/a.png\",\"language\":\"zh-CN\","
                + "\"bio\":\"hi\",\"exp\":1234,\"rks\":14.567891234,\"joined\":\"2021-03-04T05:06:07.123456Z\","
                + "\"lastLogin\":\"2026-10-01T12:00:00.5Z\",\"roles\":1,\"banned\":false,\"loginBanned\":false,"
                + "\"followerCount\":10,\"followingCount\":20,\"email\":\"a@b.c\"}");
        row("user", "snake keys for annotated fields are not read", "{\"id\":2,\"last_login\":\"2026-10-01T12:00:00Z\","
                + "\"login_banned\":true,\"follower_count\":3,\"following_count\":4}");
        row("user", "name null and absent", "{\"id\":3,\"name\":null}");
        row("user", "empty object", "{}");
        row("user", "empty document", "");
        row("user", "whitespace document", "   \n ");
        row("user", "null document", "null");
        row("user", "array instead of object", "[]");
        row("user", "trailing garbage", "{\"id\":1} x");
        row("user", "lenient syntax", "{id:5, name:'x', /* c */ rks:1.5}");
        row("user", "repeated key keeps the last", "{\"id\":1,\"id\":2}");

        row("chart", "real shape", "{\"id\":42,\"name\":\"Song <1>\",\"level\":\"AT Lv.17\",\"difficulty\":16.8,\"charter\":\"c\","
                + "\"composer\":\"co\",\"illustrator\":\"i\",\"description\":\"d\\nd\",\"ranked\":true,\"reviewed\":true,"
                + "\"stable\":false,\"stableRequest\":false,\"illustration\":\"https://x/y.png\",\"preview\":\"https://x/p.mp3\","
                + "\"file\":\"https://phira.5wyxi.com/files/abc\",\"uploader\":7,\"tags\":[\"regular\",\"plain\"],"
                + "\"rating\":0.87654,\"ratingCount\":123,\"created\":\"2022-01-02T03:04:05Z\",\"updated\":\"2026-09-30T23:59:59.999Z\","
                + "\"chartUpdated\":\"2026-09-30T23:59:59.999Z\"}");
        row("chart", "with duration", "{\"id\":42,\"duration_seconds\":321}");
        row("chart", "camelCase durationSeconds is not the key", "{\"id\":42,\"durationSeconds\":321}");
        row("chart", "duration as string and float", "{\"id\":1,\"duration_seconds\":\"77\"}");
        row("chart", "duration 77.0", "{\"id\":1,\"duration_seconds\":77.0}");
        row("chart", "duration null", "{\"id\":1,\"duration_seconds\":null}");
        row("chart", "duration negative and zero", "{\"id\":1,\"duration_seconds\":-5}");
        row("chart", "tags empty", "{\"id\":1,\"tags\":[]}");
        row("chart", "tags null", "{\"id\":1,\"tags\":null}");
        row("chart", "snake keys for annotated fields are not read",
                "{\"id\":1,\"stable_request\":true,\"rating_count\":9,\"chart_updated\":\"2026-01-01T00:00:00Z\"}");
        row("chart", "float digits", "{\"difficulty\":0.1,\"rating\":0.84}");
        row("chart", "float exponent", "{\"difficulty\":1.0E-4,\"rating\":12345678.0}");
        row("chart", "empty object", "{}");

        row("record", "list endpoint (snake_case)", "{\"id\":9,\"player\":1,\"chart\":2,\"score\":999999,\"accuracy\":0.98765,"
                + "\"perfect\":100,\"good\":2,\"bad\":1,\"miss\":0,\"speed\":1.0,\"max_combo\":103,\"best\":true,\"best_std\":false,"
                + "\"mods\":0,\"full_combo\":false,\"time\":\"2026-10-01T12:00:00.25Z\",\"std\":0.012,\"std_score\":987654.5}");
        row("record", "record endpoint (camelCase)", "{\"id\":9,\"player\":1,\"chart\":2,\"score\":999999,\"accuracy\":0.98765,"
                + "\"perfect\":100,\"good\":2,\"bad\":1,\"miss\":0,\"speed\":1.0,\"maxCombo\":103,\"best\":true,\"bestStd\":true,"
                + "\"mods\":0,\"fullCombo\":true,\"time\":\"2026-10-01T12:00:00.25Z\",\"std\":0.012,\"stdScore\":987654.5}");
        row("record", "both spellings, camel first", "{\"bestStd\":true,\"best_std\":false}");
        row("record", "both spellings, snake first", "{\"best_std\":true,\"bestStd\":false}");
        row("record", "both spellings of three fields", "{\"full_combo\":true,\"fullCombo\":false,\"std_score\":1.5,\"stdScore\":2.5}");
        row("record", "maxCombo camelCase is not read", "{\"maxCombo\":5}");
        row("record", "empty object", "{}");

        row("login", "real shape (camelCase from the service)", "{\"id\":5,\"token\":\"tok\",\"refreshToken\":\"r\",\"expireAt\":\"2026-12-01T00:00:00Z\"}");
        row("login", "snake keys", "{\"id\":5,\"token\":\"tok\",\"refresh_token\":\"r\",\"expire_at\":\"2026-12-01T00:00:00Z\"}");
        row("login", "no token", "{\"id\":5}");
        row("login", "empty document", "");
        row("login", "null for a primitive record component (Java throws, Rust keeps the default)", "{\"id\":null,\"token\":\"t\"}");
        row("page", "null count (Java throws, Rust keeps the default)", "{\"count\":null}");
        row("login", "html in token", "{\"id\":5,\"token\":\"a<b>&c='d'\"}");

        row("page", "empty page", "{\"count\":0,\"results\":[]}");
        row("page", "no results key", "{\"count\":3}");
        row("page", "results null", "{\"count\":3,\"results\":null}");
        row("page", "two charts", "{\"count\":2,\"results\":[{\"id\":1,\"name\":\"a\",\"rating\":0.5,\"tags\":[\"plain\"]},"
                + "{\"id\":2,\"name\":null,\"duration_seconds\":90}]}");
        row("page", "result is null element", "{\"count\":1,\"results\":[null]}");

        row("records", "empty", "[]");
        row("records", "null", "null");
        row("records", "empty document", "");
        row("records", "one", "[{\"id\":1,\"player\":2,\"chart\":3,\"score\":4,\"time\":\"2026-10-01T12:00:00Z\"}]");
        row("records", "null element", "[null]");
        row("records", "object instead of array", "{}");
    }
}
