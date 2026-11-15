import top.rymc.phira.main.data.RoundRecord;
import top.rymc.phira.main.data.RoundRecord.PlayerResult;
import top.rymc.phira.main.game.player.Player;
import top.rymc.phira.main.game.point.PlayerPointService;
import top.rymc.phira.main.game.record.RoundRecordService;
import top.rymc.phira.main.http.AdminService;
import top.rymc.phira.main.http.DataTransferService;
import top.rymc.phira.main.http.DataTransferService.Kind;
import top.rymc.phira.main.data.UserInfo;
import top.rymc.phira.main.game.player.operations.PlayerOperations;
import top.rymc.phira.main.game.room.Room;

import java.io.IOException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.stream.Collectors;

/**
 * Golden vectors for zphira-ledger, recorded by running the REAL compiled classes of the Java
 * server (PlayerPointService, RoundRecordService, AdminService, DataTransferService) with the
 * REAL Gson. One JVM per mode, because every store is a static singleton.
 *
 * <p>Run (see README.md): the working directory must be an empty scratch directory (the stores
 * read and write {@code data/} relative to it), the class path is the jphira-mp build output plus
 * its dependency jars.
 *
 * <pre>
 *   java -cp "$CP" GenLedger.java points  OUT
 *   java -cp "$CP" GenLedger.java records OUT
 *   java -cp "$CP" GenLedger.java admins  OUT
 * </pre>
 */
public class GenLedger {

    static PrintWriter w;

    static void rec(String label, String... fields) {
        StringBuilder b = new StringBuilder(Esc.esc(label));
        for (String f : fields) {
            b.append('\t').append(Esc.esc(f));
        }
        w.println(b);
    }

    static String err(Throwable e) {
        return "ERR " + e.getClass().getName() + ": " + e.getMessage();
    }

    static String text(String file) throws IOException {
        return Files.readString(Path.of("data", file));
    }

    static void put(String file, String content) throws IOException {
        Files.createDirectories(Path.of("data"));
        Files.writeString(Path.of("data", file), content);
    }

    public static void main(String[] args) throws Exception {
        String mode = args[0];
        Path out = Path.of(args[1]);
        Files.createDirectories(out);
        try (PrintWriter pw = new PrintWriter(Files.newBufferedWriter(out.resolve("ledger_" + mode + ".tsv"), StandardCharsets.UTF_8))) {
            w = pw;
            switch (mode) {
                case "points" -> points(out);
                case "records" -> records(out);
                case "admins" -> admins(out);
                default -> throw new IllegalArgumentException(mode);
            }
        }
        System.exit(0);
    }

    // ================================================================== points

    static Player player(int id, String name) {
        return new Player() {
            public Optional<Room> getRoom() { return Optional.empty(); }
            public void kick() { }
            public UserInfo getUserInfo() { return null; }
            public Optional<PlayerOperations> operations() { return Optional.empty(); }
            public boolean isOnline() { return true; }
            @Override public int getId() { return id; }
            @Override public String getName() { return name; }
        };
    }

    static String pointsSnapshot(int limit) {
        return PlayerPointService.getRankingSnapshot(limit).stream()
                .map(e -> e.playerId() + ":" + e.name() + ":" + e.points() + ":" + e.rank())
                .collect(Collectors.joining(";"));
    }

    static String ranks(int... ids) {
        StringBuilder b = new StringBuilder();
        for (int id : ids) {
            if (b.length() > 0) b.append(';');
            b.append(id).append(':').append(PlayerPointService.getRank(id));
        }
        return b.toString();
    }

    static final int[] RANK_IDS = {1, 3, 5, 9, 20, 777, -4, 42, 7, 100, 2};

    static void points(Path out) throws Exception {
        String seed = "{\"5\":{\"name\":\"Eve\",\"points\":300},\"1\":{\"name\":\"Ann\",\"points\":100},"
                + "\"3\":{\"name\":\"Cy\",\"points\":100},\"9\":{\"points\":0},"
                + "\"20\":{\"name\":\"Zed<>&'\",\"points\":250},\"-4\":{\"name\":\"Neg\",\"points\":100}}";
        put("player-points.json", seed);
        Files.writeString(out.resolve("points_seed.json"), seed);

        // First use of the class runs the static initializer, which loads the seed.
        rec("snapshot_load", pointsSnapshot(100));
        rec("snapshot_limit2", pointsSnapshot(2));
        rec("ranks_load", ranks(RANK_IDS));
        rec("lines_first", String.join("\n", PlayerPointService.getTopRankingLines(10)));

        // touch of a brand new player: not in the cached ranking, rank = cached size + 1
        PlayerPointService.PointSummary s = PlayerPointService.getSummary(player(42, "Newbie"));
        rec("summary_new", s.points() + ":" + s.rank());
        // a rename through touch is visible at once, the order is not recomputed
        s = PlayerPointService.getSummary(player(5, "Eve2"));
        rec("summary_rename", s.points() + ":" + s.rank());
        rec("snapshot_after_touch", pointsSnapshot(100));
        rec("rank_new_again", ranks(42));

        Map<Integer, Integer> gained = new LinkedHashMap<>();
        Map<Integer, String> names = new LinkedHashMap<>();
        gained.put(7, 150);
        gained.put(1, 50);
        gained.put(42, 10);
        names.put(7, "Seven");
        names.put(1, "Ann2");
        Map<Integer, Integer> totals = PlayerPointService.addPointsBatch(gained, names);
        rec("batch_totals", totals.entrySet().stream().map(e -> e.getKey() + ":" + e.getValue()).collect(Collectors.joining(";")));
        rec("snapshot_after_batch", pointsSnapshot(100));
        rec("ranks_after_batch", ranks(RANK_IDS));
        // within 60 s the cached lines are returned whatever happened since
        rec("lines_cached", String.join("\n", PlayerPointService.getTopRankingLines(10)));
        rec("lines_cached_other_limit", String.join("\n", PlayerPointService.getTopRankingLines(3)));

        int total = PlayerPointService.addPoints(player(3, "Cy3"), -1000);
        rec("add_points_floor", total + "");
        total = PlayerPointService.addPoints(player(3, "Cy3"), 25);
        rec("add_points_after_floor", total + "");
        rec("snapshot_after_add", pointsSnapshot(100));
        PlayerPointService.flush();
        String file1 = text("player-points.json");
        Files.writeString(out.resolve("points_java.json"), file1);
        rec("file_after_flush", file1);

        // --- imports through the real DataTransferService
        // base state for each scenario: a replace import of BASE
        String base = "{\"1\":{\"name\":\"Ann\",\"points\":100},\"2\":{\"name\":\"Bo\",\"points\":300},\"3\":{\"points\":50}}";
        importCase("import_base", Kind.POINTS, false, base);
        rec("snapshot_base", pointsSnapshot(100));
        importCase("import_merge_overlap", Kind.POINTS, true,
                "{\"3\":{\"name\":\"Cy\",\"points\":400},\"10\":{\"name\":\"Ten\",\"points\":1},\"2\":{\"points\":7}}");
        rec("snapshot_merge_overlap", pointsSnapshot(100));
        importCase("import_replace_small", Kind.POINTS, false,
                "{\"8\":{\"name\":\"Eight\",\"points\":80},\"4\":{\"name\":\"Four\",\"points\":80}}");
        rec("snapshot_replace_small", pointsSnapshot(100));
        importCase("import_replace_empty", Kind.POINTS, false, "{}");
        rec("snapshot_replace_empty", pointsSnapshot(100));
        importCase("import_merge_into_empty", Kind.POINTS, true, "{\"1\":{\"name\":\"One\",\"points\":1}}");
        rec("snapshot_merge_into_empty", pointsSnapshot(100));
        // the cached top lines are not invalidated by an import
        rec("lines_after_import", String.join("\n", PlayerPointService.getTopRankingLines(10)));

        // --- what the loader accepts (reload clears memory and reads the file)
        String[][] docs = {
                {"empty_file", ""},
                {"null_doc", "null"},
                {"empty_obj", "{}"},
                {"ok_basic", "{\"1\":{\"name\":\"A\",\"points\":5}}"},
                {"key_float", "{\"1.0\":{\"points\":5}}"},
                {"key_leading_zero", "{\"01\":{\"points\":5}}"},
                {"key_dup_coerce", "{\"1\":{\"points\":5},\"1.0\":{\"points\":6}}"},
                {"key_nonint", "{\"abc\":{\"points\":5}}"},
                {"key_big", "{\"2147483648\":{\"points\":5}}"},
                {"key_negative", "{\"-5\":{\"points\":5}}"},
                {"value_null", "{\"1\":null}"},
                {"value_number", "{\"1\":5}"},
                {"value_array", "{\"1\":[]}"},
                {"points_string", "{\"1\":{\"points\":\"7\"}}"},
                {"points_float", "{\"1\":{\"points\":7.5}}"},
                {"points_float_int", "{\"1\":{\"points\":7.0}}"},
                {"points_null", "{\"1\":{\"points\":null,\"name\":null}}"},
                {"points_negative", "{\"1\":{\"points\":-5}}"},
                {"points_overflow", "{\"1\":{\"points\":2147483648}}"},
                {"name_number", "{\"1\":{\"name\":5,\"points\":1}}"},
                {"name_bool", "{\"1\":{\"name\":true,\"points\":1}}"},
                {"name_obj", "{\"1\":{\"name\":{},\"points\":1}}"},
                {"unknown_field", "{\"1\":{\"points\":1,\"extra\":[1,2]}}"},
                {"empty_row", "{\"1\":{}}"},
                {"array_top", "[]"},
                {"string_top", "\"x\""},
                {"trailing_garbage", "{} x"},
                {"unquoted", "{1:{points:3}}"},
                {"single_quotes", "{'1':{'name':'q','points':3}}"},
                {"comment", "/*c*/{\"1\":{\"points\":3}}"},
                {"dup_raw_key", "{\"1\":{\"points\":1},\"1\":{\"points\":2}}"},
                {"pairs_array", "[[\"1\",{\"points\":3}]]"},
                {"truncated", "{\"1\":{\"points\":3}"},
        };
        for (String[] d : docs) {
            put("player-points.json", d[1]);
            String result;
            try {
                PlayerPointService.reload();
                result = "OK " + pointsSnapshot(100);
            } catch (Throwable t) {
                result = err(t);
            }
            rec("load_" + d[0], d[1], result);
        }
    }

    // ================================================================== records

    static RoundRecord round(String id, String room, int chart, String chartName, long started, long finished,
                             PlayerResult... results) {
        return new RoundRecord(id, room, chart, chartName, started, finished, List.of(results));
    }

    static PlayerResult res(int pid, String name, int rank, int score, float acc, float std, int gained, int total) {
        // float to double, as RoomPlaying does with record.getAccuracy()
        return new PlayerResult(pid, name, rank, score, acc, std, gained, total);
    }

    static String ids(List<RoundRecord> list) {
        return list.stream().map(RoundRecord::id).collect(Collectors.joining(","));
    }

    static String allIds() {
        return ids(RoundRecordService.query(100000, 0, null, null));
    }

    static void records(Path out) throws Exception {
        List<RoundRecord> made = new ArrayList<>();
        made.add(round(RoundRecordService.nextId("room-a", 1700000000000L), "room-a", 101,
                "Spasmodic <Lv.15> & 'co' é", 1699999990000L, 1700000000000L,
                res(3, "Alice", 1, 990000, 0.9876f, 0.0123f, 100, 100),
                res(4, "Bob 中文😀", 2, 950000, 0.95f, 0.02f, 75, 175)));
        made.add(round(RoundRecordService.nextId("room-b", 1700000100000L), "room-b", 102, null,
                1700000050000L, 1700000100000L,
                res(5, "Eve", 1, 1000000, 1.0f, 0.0f, 100, 300)));
        made.add(round(RoundRecordService.nextId("room-a", 1700000200000L), "room-a", 101, "Spasmodic <Lv.15> & 'co' é",
                1700000150000L, 1700000200000L));
        made.add(round(RoundRecordService.nextId("room-a", 1700000300000L), "room-a", 103, "Tiny",
                1700000250000L, 1700000300000L,
                res(3, "Alice", 1, 123456, 0.99999994f, 1.0E-4f, 100, 200),
                res(6, "Dan", 1, 123456, 0.99999994f, 1.0E-4f, 100, 100),
                res(7, "Fay", 3, 1, 0.0f, 12345678f, 50, 50)));
        made.add(round(RoundRecordService.nextId("room-c", 1700000400000L), "room-c", 104, "Long \"quoted\" name\nline",
                1700000350000L, 1700000400000L,
                res(3, "Alice", 2, 500000, 0.5f, 0.123456789f, 75, 275),
                res(5, "Eve", 1, 600000, 0.87654321f, 1.5f, 100, 400)));
        for (RoundRecord r : made) {
            RoundRecordService.append(r);
        }
        rec("next_ids", made.stream().map(RoundRecord::id).collect(Collectors.joining(",")));
        RoundRecordService.flush();
        String file1 = text("round-records.json");
        Files.writeString(out.resolve("records_java.json"), file1);
        rec("file_after_flush", file1);
        rec("size", RoundRecordService.size() + "");

        int[][] qn = {{10, 0}, {2, 1}, {0, 0}, {10, 99}, {1, 4}};
        String[] rooms = {null, "room-a", "room-c", "nope"};
        Integer[] players = {null, 3, 5, 99};
        for (int[] q : qn) {
            for (String room : rooms) {
                for (Integer player : players) {
                    rec("query", q[0] + "," + q[1] + "," + room + "," + player,
                            ids(RoundRecordService.query(q[0], q[1], room, player)));
                }
            }
        }
        for (int pid : new int[]{3, 5, 99}) {
            for (int limit : new int[]{10, 2, 1, 0}) {
                rec("by_player", pid + "," + limit, RoundRecordService.byPlayer(pid, limit).stream()
                        .map(r -> r.recordId() + "|" + r.roomId() + "|" + r.chartId() + "|" + r.chartName() + "|"
                                + r.finishedAt() + "|" + r.result().playerId() + "|" + r.result().rank())
                        .collect(Collectors.joining(";")));
            }
        }
        for (RoundRecord r : made) {
            rec("find", r.id(), RoundRecordService.find(r.id()).map(RoundRecord::chartId).map(String::valueOf).orElse("none"));
        }
        rec("find", "nope", RoundRecordService.find("nope").map(RoundRecord::id).orElse("none"));
        rec("next_id_after", RoundRecordService.nextId("zz", 5L));

        // --- imports through the real DataTransferService
        String r1 = "{\"id\":\"i1\",\"room_id\":\"x\",\"chart_id\":1,\"chart_name\":\"One\",\"started_at\":10,\"finished_at\":20,\"results\":"
                + "[{\"player_id\":1,\"player_name\":\"A\",\"rank\":1,\"score\":5,\"accuracy\":0.5,\"std\":0.25,\"gained_points\":100,\"total_points\":100}]}";
        String r2 = "{\"id\":\"i2\",\"room_id\":\"x\",\"chart_id\":2,\"started_at\":30,\"finished_at\":40,\"results\":[]}";
        String r3 = "{\"id\":\"i3\",\"room_id\":\"y\",\"chart_id\":3,\"started_at\":50,\"finished_at\":60,\"results\":[]}";
        String r2b = "{\"id\":\"i2\",\"room_id\":\"z\",\"chart_id\":22,\"started_at\":300,\"finished_at\":400,\"results\":[]}";
        String r9 = "{\"id\":\"i9\",\"room_id\":\"x\",\"chart_id\":9,\"started_at\":1,\"finished_at\":2,\"results\":[]}";
        importCase("import_base", Kind.RECORDS, false, "[" + r1 + "," + r2 + "," + r3 + "]");
        rec("state_base", allIds());
        importCase("import_merge_overlap", Kind.RECORDS, true, "[" + r2b + "," + r9 + "]");
        rec("state_merge_overlap", allIds());
        rec("size_merge_overlap", RoundRecordService.size() + "");
        importCase("import_replace_small", Kind.RECORDS, false, "[" + r9 + "]");
        rec("state_replace_small", allIds());
        importCase("import_replace_empty", Kind.RECORDS, false, "[]");
        rec("state_replace_empty", allIds());
        importCase("import_merge_into_empty", Kind.RECORDS, true, "[" + r3 + "," + r1 + "]");
        rec("state_merge_into_empty", allIds());
        // duplicated ids inside one document
        importCase("import_replace_dups", Kind.RECORDS, false, "[" + r1 + "," + r1 + "]");
        rec("state_replace_dups", allIds());
        importCase("import_merge_dups", Kind.RECORDS, true, "[" + r9 + "," + r9 + "]");
        rec("state_merge_dups", allIds());
        // import of more rows than the cap: kept until the next append
        StringBuilder many = new StringBuilder("[");
        for (int i = 0; i < 1003; i++) {
            if (i > 0) many.append(',');
            many.append("{\"id\":\"m").append(i).append("\",\"room_id\":\"m\",\"chart_id\":1,\"started_at\":")
                    .append(i).append(",\"finished_at\":").append(i + 1).append(",\"results\":[]}");
        }
        many.append(']');
        importCase("import_over_cap", Kind.RECORDS, false, many.toString());
        rec("size_over_cap", RoundRecordService.size() + "");
        RoundRecordService.append(round("after-cap", "m", 1, "n", 1, 2));
        rec("size_after_append", RoundRecordService.size() + "");
        rec("newest_after_append", RoundRecordService.query(1, 0, null, null).get(0).id());
        rec("oldest_after_append", RoundRecordService.query(1, 999, null, null).get(0).id());

        // --- what the loader accepts
        String full = "{\"id\":\"a\",\"room_id\":\"r\",\"chart_id\":7,\"chart_name\":\"c\",\"started_at\":5,\"finished_at\":6,"
                + "\"results\":[{\"player_id\":3,\"player_name\":\"p\",\"rank\":1,\"score\":10,\"accuracy\":0.5,\"std\":0.25,\"gained_points\":100,\"total_points\":100}]}";
        String[][] docs = {
                {"empty_file", ""},
                {"null_doc", "null"},
                {"empty_arr", "[]"},
                {"ok_full", "[" + full + "]"},
                {"missing_fields", "[{\"id\":\"a\"}]"},
                {"empty_row", "[{}]"},
                {"null_primitive", "[{\"id\":\"a\",\"chart_id\":null}]"},
                {"null_string", "[{\"id\":null,\"room_id\":null,\"chart_name\":null}]"},
                {"string_ints", "[{\"id\":\"a\",\"chart_id\":\"7\",\"started_at\":\"5\",\"results\":[{\"player_id\":\"3\",\"score\":\"10\",\"accuracy\":\"0.5\"}]}]"},
                {"float_int", "[{\"id\":\"a\",\"chart_id\":7.0}]"},
                {"float_frac", "[{\"id\":\"a\",\"chart_id\":7.5}]"},
                {"element_null", "[null]"},
                {"object_top", "{}"},
                {"results_not_list", "[{\"id\":\"a\",\"results\":{}}]"},
                {"results_null", "[{\"id\":\"a\",\"results\":null}]"},
                {"results_null_element", "[{\"id\":\"a\",\"results\":[null]}]"},
                {"results_null_primitive", "[{\"id\":\"a\",\"results\":[{\"player_id\":null}]}]"},
                {"dup_ids", "[{\"id\":\"a\",\"chart_id\":1},{\"id\":\"a\",\"chart_id\":2}]"},
                {"unknown_fields", "[{\"id\":\"a\",\"zzz\":{\"q\":1},\"results\":[{\"player_id\":3,\"q\":[]}]}]"},
                {"camel_case_keys", "[{\"id\":\"a\",\"roomId\":\"r\",\"chartId\":7}]"},
                {"doubles", "[{\"id\":\"a\",\"results\":[{\"player_id\":1,\"accuracy\":1,\"std\":1e-5},{\"player_id\":2,\"accuracy\":\"0.1\",\"std\":123456789.5}]}]"},
                {"truncated", "[{\"id\":\"a\""},
                {"trailing_garbage", "[] x"},
        };
        for (String[] d : docs) {
            put("round-records.json", d[1]);
            String result;
            try {
                RoundRecordService.reload();
                result = "OK " + RoundRecordService.size() + " " + allIds();
            } catch (Throwable t) {
                result = err(t);
            }
            rec("load_" + d[0], d[1], result);
        }
    }

    // ================================================================== admins

    static void admins(Path out) throws Exception {
        // First use runs the static initializer: the file does not exist yet and is created.
        rec("list_initial", AdminService.getAdmins().toString());
        rec("file_created", text("admins.json"));
        rec("add", AdminService.addAdmin(5) + "", AdminService.addAdmin(3) + "", AdminService.addAdmin(9) + "",
                AdminService.addAdmin(3) + "");
        rec("list_after_add", AdminService.getAdmins().toString());
        String file1 = text("admins.json");
        Files.writeString(out.resolve("admins_java.json"), file1);
        rec("file_after_add", file1);
        rec("remove", AdminService.removeAdmin(5) + "", AdminService.removeAdmin(5) + "", AdminService.removeAdmin(77) + "");
        rec("list_after_remove", AdminService.getAdmins().toString());
        rec("is_admin", AdminService.isAdmin(3) + "", AdminService.isAdmin(5) + "");
        AdminService.addAdmin(-2);
        AdminService.addAdmin(0);
        AdminService.addAdmin(2147483647);
        rec("list_odd_ids", AdminService.getAdmins().toString());
        rec("file_odd_ids", text("admins.json"));

        // --- imports
        importCase("import_base", Kind.ADMINS, false, "[1,2,3]");
        rec("state_base", AdminService.getAdmins().toString());
        String[][] cases = {
                {"merge_overlap", "true", "[3,4]"},
                {"replace_small", "false", "[7]"},
                {"replace_dups", "false", "[7,7,8]"},
                {"merge_float_dup", "true", "[1.0,4]"},
                {"merge_dups", "true", "[5,5]"},
                {"replace_string", "false", "[\"9\"]"},
                {"replace_empty", "false", "[]"},
                {"merge_into_empty", "true", "[2,1]"},
                {"replace_float_frac", "false", "[1.5]"},
                {"replace_null_elem", "false", "[null]"},
                {"replace_bool", "false", "[true]"},
        };
        for (String[] c : cases) {
            importCase("import_base", Kind.ADMINS, false, "[1,2,3]");
            importCase("import_" + c[0], Kind.ADMINS, c[1].equals("true"), c[2]);
            rec("state_" + c[0], AdminService.getAdmins().toString());
        }

        // --- loader / reload behaviour: start from [7], then replace the file
        String[][] docs = {
                {"empty_file", ""},
                {"null_doc", "null"},
                {"empty_arr", "[]"},
                {"unsorted", "[3,1,2]"},
                {"dups", "[1,1]"},
                {"string", "[\"5\"]"},
                {"float_int", "[5.0]"},
                {"float_frac", "[5.5]"},
                {"null_elem", "[null]"},
                {"bool", "[true]"},
                {"object", "{}"},
                {"string_top", "\"x\""},
                {"truncated", "[1,2"},
                {"missing_comma", "[1 2]"},
                {"overflow", "[2147483648]"},
                {"trailing_garbage", "[1] x"},
                {"unquoted_str", "[abc]"},
        };
        for (String[] d : docs) {
            importCase("import_base", Kind.ADMINS, false, "[7]");
            put("admins.json", d[1]);
            String result;
            try {
                AdminService.reload();
                result = "OK " + AdminService.getAdmins();
            } catch (Throwable t) {
                result = err(t) + " | still " + AdminService.getAdmins();
            }
            rec("load_" + d[0], d[1], result);
        }
    }

    // ================================================================== shared

    /** Long generated documents are not stored; the Rust test generates the same one. */
    static String shown(String doc) {
        return doc.length() > 2000 ? "<generated " + doc.length() + " chars>" : doc;
    }

    static void importCase(String label, Kind kind, boolean merge, String doc) {
        try {
            DataTransferService.ImportResult r = DataTransferService.importJson(kind, doc, merge);
            rec(label, kind.key() + (merge ? " merge" : " replace"), shown(doc), "OK", r.imported() + "", r.total() + "");
        } catch (Throwable t) {
            rec(label, kind.key() + (merge ? " merge" : " replace"), shown(doc), err(t));
        }
    }
}
