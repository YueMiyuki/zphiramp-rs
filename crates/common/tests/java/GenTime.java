import java.io.IOException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;
import java.time.OffsetDateTime;
import java.time.ZoneOffset;
import java.util.ArrayList;
import java.util.List;
import java.util.Random;

/**
 * Golden vectors for zphira_common::time::JavaOffsetDateTime.
 *
 * <p>Run: {@code java crates/common/tests/java/GenTime.java crates/common/tests/fixtures}
 */
public class GenTime {

    public static void main(String[] args) throws IOException {
        Path out = Path.of(args[0]);
        Files.createDirectories(out);
        parse(out.resolve("offset_date_time_parse.tsv"));
        format(out.resolve("offset_date_time_format.tsv"));
        compare(out.resolve("offset_date_time_compare.tsv"));
    }

    static PrintWriter open(Path p) throws IOException {
        return new PrintWriter(Files.newBufferedWriter(p, StandardCharsets.UTF_8));
    }

    /** input -> toString, epoch millis, offset seconds, epoch second, nano; or ERR message. */
    static void parse(Path p) throws IOException {
        List<String> inputs = new ArrayList<>(List.of(
                "2026-11-14T15:26:46Z", "2026-11-14T15:26Z", "2026-11-14T15:26:00Z", "2026-11-14T15:26:00.000Z",
                "2026-11-14T15:26:46.Z", "2026-11-14T15:26:46.1Z", "2026-11-14T15:26:46.12Z",
                "2026-11-14T15:26:46.123Z", "2026-11-14T15:26:46.1234Z", "2026-11-14T15:26:46.12345Z",
                "2026-11-14T15:26:46.123456Z", "2026-11-14T15:26:46.1234567Z", "2026-11-14T15:26:46.12345678Z",
                "2026-11-14T15:26:46.123456789Z", "2026-11-14T15:26:46.1234567890Z", "2026-11-14T15:26:46.100Z",
                "2026-11-14T15:26:46.100000Z", "2026-11-14T15:26:46.000001Z", "2026-11-14T15:26:46.000000001Z",
                "2026-11-14T15:26:00.5Z", "2026-11-14T15:26:46,5Z", "2026-11-14T15:26.5Z",
                "2026-11-14T15:26:46+08:00", "2026-11-14T15:26:46-05:30", "2026-11-14T15:26:46+00:00",
                "2026-11-14T15:26:46-00:00", "2026-11-14T15:26:46+0800", "2026-11-14T15:26:46+08",
                "2026-11-14T15:26:46+8", "2026-11-14T15:26:46+08:00:30", "2026-11-14T15:26:46+080030",
                "2026-11-14T15:26:46+08:00:00", "2026-11-14T15:26:46+08:0", "2026-11-14T15:26:46+18:00",
                "2026-11-14T15:26:46-18:00", "2026-11-14T15:26:46+18:01", "2026-11-14T15:26:46+19:00",
                "2026-11-14T15:26:46+08:60", "2026-11-14T15:26:46+08:00:60", "2026-11-14T15:26:46z",
                "2026-11-14t15:26:46Z", "2026-11-14T15:26:46", "2026-11-14T15:26:46ZZ",
                "2026-11-14T15:26:46Z ", " 2026-11-14T15:26:46Z", "2026-11-14 15:26:46Z", "2026-11-14T15:26:46UTC",
                "2026-11-14T15:26:46+08:00[Asia/Shanghai]", "2026-11-14T24:00Z", "2026-11-14T23:59:59Z",
                "2026-11-14T23:59:60Z", "2026-11-14T23:60Z", "2026-11-14T1:00Z", "2026-11-14T15:5Z",
                "2026-1-14T15:00Z", "2026-11-4T15:00Z", "2026-13-01T00:00Z", "2026-00-01T00:00Z",
                "2026-11-00T00:00Z", "2026-02-29T00:00Z", "2024-02-29T00:00Z", "2100-02-29T00:00Z",
                "2000-02-29T00:00Z", "2026-04-31T00:00Z", "2026-11-31T00:00Z", "0000-01-01T00:00Z",
                "-0001-01-01T00:00Z", "-0001-12-31T23:59:59.999Z", "+0001-01-01T00:00Z", "0001-01-01T00:00Z",
                "1970-01-01T00:00Z", "1969-12-31T23:59:59.999Z", "9999-12-31T23:59:59.999999999Z",
                "+10000-01-01T00:00Z", "10000-01-01T00:00Z", "+999999999-12-31T23:59:59.999999999-18:00",
                "-999999999-01-01T00:00+18:00", "+1000000000-01-01T00:00Z", "999-01-01T00:00Z",
                "+999-01-01T00:00Z", "02026-11-14T15:26Z", "2026-11-14T15:26:46.123456789+05:45",
                "1900-01-01T00:00:00.000000001+14:00", "2023-06-09T12:01:03.586771Z",
                "2023-06-09T12:01:03.586771+00:00", "", "2026", "2026-11-14", "2026-11-14T", "2026-11-14T15",
                "2026-11-14T15:", "2026-11-14T15:26:", "2026-11-14T15:26:4Z", "2026-11-14T15:26:46.+08:00",
                "２026-11-14T15:26Z", "2026-11-14T15:26:46-08:00:30", "2026-11-14T00:00:00Z",
                "2026-11-14T00:00:00.000000000Z", "2026-11-14T00:00:00.000000000+01:00",
                "-999-01-01T00:00Z", "-10000-01-01T00:00Z", "+10000000000-01-01T00:00Z", "-0000-01-01T00:00Z",
                "+0000-01-01T00:00Z", "+00000-01-01T00:00Z", "-00001-01-01T00:00Z", "+2026-11-14T15:26Z",
                "2026-11-14T15:26:46.123456789", "2026-11-14T15:26:46+08:00:3", "2026-11-14T15:26:46+08:3",
                "2026-11-14T15:26:46+08:30:", "2026-11-14T15:26:46+08:", "2026-11-14T15:26:46+", "2026-11-14T15:26:46-",
                "2026-11-14T15:26:46+00", "2026-11-14T15:26:46-00", "2026-11-14T15:26:46+00:00:01",
                "2026-11-14T15:26:46+24:00", "2026-11-14T15:26:46+08:00z", "2026-11-14T15:26:46.123z",
                "2026-11-14T15:26:46.123+08", "2026-02-28T23:59:59.999999999+18:00", "1582-10-10T00:00Z",
                "2026-11-14T15:26:46+1", "2026-11-14T15:26:46+123", "2026-11-14T15:26:46+08:00:30.5",
                "2026-11-14T15:26:46+08:00Z", "2026-11-14T15:26:46Z+08:00", "2026-11-14Z", "2026-11-14T15:26+08:00",
                "2026-11-14T15:26z", "-2026-11-14T15:26Z", "2026-11-14T15:26:46.1234567891Z"));
        try (PrintWriter w = open(p)) {
            for (String s : inputs) {
                String result;
                try {
                    OffsetDateTime t = OffsetDateTime.parse(s);
                    Instant i = t.toInstant();
                    String millis;
                    try {
                        millis = Long.toString(i.toEpochMilli());
                    } catch (ArithmeticException e) {
                        millis = "OVERFLOW";
                    }
                    result = t.toString() + "\t" + millis + "\t" + t.getOffset().getTotalSeconds() + "\t"
                            + i.getEpochSecond() + "\t" + i.getNano();
                } catch (Exception e) {
                    result = "ERR " + e.getClass().getSimpleName() + ": " + e.getMessage();
                }
                w.println(Esc.esc(s) + "\t" + Esc.esc(result));
            }
        }
    }

    /** epoch second, nano, offset seconds -> toString (and toEpochMilli). */
    static void format(Path p) throws IOException {
        Random r = new Random(4242L);
        int[] offsets = {0, 3600, -3600, 8 * 3600, 5 * 3600 + 45 * 60, -(9 * 3600 + 30 * 60), 18 * 3600, -18 * 3600,
                45, -1, 3661};
        int[] nanos = {0, 1, 1000, 1_000_000, 123_000_000, 123_456_000, 123_456_789, 999_999_999, 100, 10_000,
                500_000_000};
        try (PrintWriter w = open(p)) {
            for (int i = 0; i < 1500; i++) {
                long sec;
                switch (i % 5) {
                    case 0 -> sec = r.nextLong(-62_135_596_800L * 2, 253_402_300_799L * 2);
                    case 1 -> sec = r.nextLong(1_500_000_000L, 2_000_000_000L);
                    case 2 -> sec = (r.nextLong(1_500_000_000L, 2_000_000_000L) / 60) * 60;
                    case 3 -> sec = r.nextLong(-31_557_014_167_219_200L, 31_556_889_864_403_199L - 86400);
                    default -> sec = r.nextLong(-100_000_000_000L, 100_000_000_000L);
                }
                int nano = (i % 3 == 0) ? nanos[r.nextInt(nanos.length)] : r.nextInt(1_000_000_000);
                int off = offsets[r.nextInt(offsets.length)];
                OffsetDateTime t;
                try {
                    t = OffsetDateTime.ofInstant(Instant.ofEpochSecond(sec, nano), ZoneOffset.ofTotalSeconds(off));
                } catch (Exception e) {
                    continue;
                }
                String millis;
                try {
                    millis = Long.toString(t.toInstant().toEpochMilli());
                } catch (ArithmeticException e) {
                    millis = "OVERFLOW";
                }
                w.println(sec + "\t" + nano + "\t" + off + "\t" + t + "\t" + millis);
            }
        }
    }

    /** Pairs compared with compareTo / equals / isEqual. */
    static void compare(Path p) throws IOException {
        String[] values = {"2026-11-14T15:26:46Z", "2026-11-14T23:26:46+08:00", "2026-11-14T15:26:46.000000001Z",
                "2026-11-14T15:26:45.999999999Z", "2026-11-14T10:26:46-05:00", "2026-11-14T15:26:46+00:00",
                "2026-11-15T00:00Z", "2026-11-14T16:00+00:01", "2026-11-14T15:59Z", "1970-01-01T00:00Z",
                "1970-01-01T08:00+08:00"};
        try (PrintWriter w = open(p)) {
            for (String a : values) {
                for (String b : values) {
                    OffsetDateTime x = OffsetDateTime.parse(a);
                    OffsetDateTime y = OffsetDateTime.parse(b);
                    w.println(a + "\t" + b + "\t" + Integer.signum(x.compareTo(y)) + "\t" + x.equals(y) + "\t"
                            + x.isEqual(y));
                }
            }
        }
    }
}
