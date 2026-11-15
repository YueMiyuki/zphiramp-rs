import com.google.gson.FieldNamingPolicy;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonDeserializer;
import com.google.gson.JsonPrimitive;
import com.google.gson.JsonSerializer;
import com.google.gson.Strictness;
import com.google.gson.reflect.TypeToken;

import java.io.IOException;
import java.lang.reflect.Type;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.OffsetDateTime;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The three data files as plain Gson 2.12.1, configured exactly like GsonUtil.getGson(), with the
 * model shapes of DESIGN.md section 2 and of PlayerPointService.PointData, so the fixtures can be
 * regenerated with nothing but the Gson jar (GenLedger.java needs the compiled server classes).
 * The Rust tests assert that both generators produce the same bytes.
 *
 * <p>Run: {@code java -cp <gson-2.12.1.jar> GenLedgerGson.java <out dir>}
 */
public class GenLedgerGson {

    static final Gson GSON = new GsonBuilder()
            .registerTypeAdapter(OffsetDateTime.class,
                    (JsonSerializer<OffsetDateTime>) (src, type, context) -> new JsonPrimitive(src.toString()))
            .registerTypeAdapter(OffsetDateTime.class,
                    (JsonDeserializer<OffsetDateTime>) (json, type, context) -> OffsetDateTime.parse(json.getAsString()))
            .setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES)
            .setStrictness(Strictness.LENIENT)
            .setPrettyPrinting()
            .create();

    /** PlayerPointService.PointData: name, then points. */
    static final class PointData {
        private volatile String name;
        private volatile int points;

        PointData(String name, int points) {
            this.name = name;
            this.points = points;
        }
    }

    record RoundRecord(String id, String roomId, int chartId, String chartName,
                       long startedAt, long finishedAt, List<PlayerResult> results) {
        record PlayerResult(int playerId, String playerName, int rank, int score,
                            double accuracy, double std, int gainedPoints, int totalPoints) {
        }
    }

    static RoundRecord.PlayerResult res(int pid, String name, int rank, int score, float acc, float std, int gained, int total) {
        // float to double, as RoomPlaying does
        return new RoundRecord.PlayerResult(pid, name, rank, score, acc, std, gained, total);
    }

    public static void main(String[] args) throws IOException {
        Path out = Path.of(args[0]);
        Files.createDirectories(out);

        // the points of GenLedger's scenario after its flush, keys sorted like save() does
        Map<Integer, PointData> points = new LinkedHashMap<>();
        points.put(-4, new PointData("Neg", 100));
        points.put(1, new PointData("Ann2", 150));
        points.put(3, new PointData("Cy3", 25));
        points.put(5, new PointData("Eve2", 300));
        points.put(7, new PointData("Seven", 150));
        points.put(9, new PointData(null, 0));
        points.put(20, new PointData("Zed<>&'", 250));
        points.put(42, new PointData("Newbie", 10));
        Files.writeString(out.resolve("gson_points.json"), GSON.toJson(points));
        Files.writeString(out.resolve("gson_points_empty.json"), GSON.toJson(new LinkedHashMap<Integer, PointData>()));

        String name = "Spasmodic <Lv.15> & 'co' é";
        List<RoundRecord> records = new ArrayList<>();
        records.add(new RoundRecord("room-a-1700000000000-1", "room-a", 101, name, 1699999990000L, 1700000000000L, List.of(
                res(3, "Alice", 1, 990000, 0.9876f, 0.0123f, 100, 100),
                res(4, "Bob 中文😀", 2, 950000, 0.95f, 0.02f, 75, 175))));
        records.add(new RoundRecord("room-b-1700000100000-2", "room-b", 102, null, 1700000050000L, 1700000100000L, List.of(
                res(5, "Eve", 1, 1000000, 1.0f, 0.0f, 100, 300))));
        records.add(new RoundRecord("room-a-1700000200000-3", "room-a", 101, name, 1700000150000L, 1700000200000L, List.of()));
        records.add(new RoundRecord("room-a-1700000300000-4", "room-a", 103, "Tiny", 1700000250000L, 1700000300000L, List.of(
                res(3, "Alice", 1, 123456, 0.99999994f, 1.0E-4f, 100, 200),
                res(6, "Dan", 1, 123456, 0.99999994f, 1.0E-4f, 100, 100),
                res(7, "Fay", 3, 1, 0.0f, 12345678f, 50, 50))));
        records.add(new RoundRecord("room-c-1700000400000-5", "room-c", 104, "Long \"quoted\" name\nline", 1700000350000L, 1700000400000L, List.of(
                res(3, "Alice", 2, 500000, 0.5f, 0.123456789f, 75, 275),
                res(5, "Eve", 1, 600000, 0.87654321f, 1.5f, 100, 400))));
        Files.writeString(out.resolve("gson_records.json"), GSON.toJson(records));
        Files.writeString(out.resolve("gson_records_empty.json"), GSON.toJson(new ArrayList<RoundRecord>()));

        Files.writeString(out.resolve("gson_admins.json"), GSON.toJson(List.of(3, 5, 9)));
        Files.writeString(out.resolve("gson_admins_empty.json"), GSON.toJson(List.<Integer>of()));

        // reading back and writing again must not change a byte (what the Java server does on
        // every save after a load)
        Type pointType = new TypeToken<Map<Integer, PointData>>() { }.getType();
        Type recordType = new TypeToken<List<RoundRecord>>() { }.getType();
        Type adminType = new TypeToken<List<Integer>>() { }.getType();
        roundTrip(out.resolve("gson_points.json"), pointType);
        roundTrip(out.resolve("gson_records.json"), recordType);
        roundTrip(out.resolve("gson_admins.json"), adminType);
    }

    static void roundTrip(Path file, Type type) throws IOException {
        String text = Files.readString(file);
        Object model = GSON.fromJson(text, type);
        check(text, GSON.toJson(model));
    }

    static void check(String a, String b) {
        if (!a.equals(b)) {
            throw new IllegalStateException("not stable under read and write:\n" + a + "\n" + b);
        }
    }
}
