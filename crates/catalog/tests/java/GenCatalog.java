import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import org.apache.logging.log4j.LogManager;
import top.rymc.phira.main.Server;
import top.rymc.phira.main.config.ServerArgs;
import top.rymc.phira.main.data.ChartInfo;
import top.rymc.phira.main.game.room.chart.ChartIndex;
import top.rymc.phira.main.game.room.chart.ChartPool;
import top.rymc.phira.main.game.room.chart.PoolCategory;
import top.rymc.phira.main.game.room.chart.ScreeningRule;
import top.rymc.phira.main.game.room.chart.SubmissionService;
import top.rymc.phira.main.http.DataTransferService;
import top.rymc.phira.main.util.PhiraFetcher;

import java.io.OutputStream;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Function;
import java.util.stream.Collectors;
import java.util.stream.Stream;

/**
 * Golden vectors for zphira-catalog, recorded by running the real compiled Java classes
 * (ChartPool, ChartIndex, SubmissionService, DataTransferService) in a scratch working directory
 * against a local Phira stub. Never contacts the real Phira service.
 *
 * Usage (cwd = a fresh, empty scratch directory): java -cp CP GenCatalog.java SCENARIO OUT_DIR
 * Writes OUT_DIR/SCENARIO/{input,output}/..., result.txt (one "key<TAB>value" per line) and
 * log.txt ("LEVEL message" per line, from logs/latest.log).
 */
public final class GenCatalog {

    static Path out;
    static final List<String> results = new ArrayList<>();
    static final List<String> requests = Collections.synchronizedList(new ArrayList<>());
    /** Scripted stub: request path (without leading slash) -> JSON body; absent means 404. */
    static final Map<String, String> stub = new ConcurrentHashMap<>();

    public static void main(String[] a) throws Exception {
        String scenario = a[0];
        out = Path.of(a[1]).toAbsolutePath().resolve(scenario);
        Files.createDirectories(out);

        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/", GenCatalog::serve);
        server.start();
        PhiraFetcher.setHost("http://127.0.0.1:" + server.getAddress().getPort() + "/");
        Field f = Server.class.getDeclaredField("args");
        f.setAccessible(true);
        f.set(Server.getInstance(), new ServerArgs(new String[0]));
        Files.createDirectories(Path.of("data"));

        switch (scenario) {
            case "pool_defaults" -> poolDefaults();
            case "pool_normalize" -> poolNormalize();
            case "pool_load_edges" -> poolLoadEdges();
            case "pool_ops" -> poolOps();
            case "index_save" -> indexSave();
            case "index_unreadable" -> indexUnreadable();
            case "sweep_fresh" -> sweepFresh();
            case "sweep_fresh_no_plain" -> sweepFreshNoPlain();
            case "sweep_incremental" -> sweepIncremental();
            case "sweep_division" -> sweepDivision();
            case "sweep_empty_page" -> sweepEmptyPage();
            case "submissions_ops" -> submissionsOps();
            case "submissions_odd" -> submissionsOdd();
            case "submissions_null_row" -> submissionsNullRow();
            case "import_pools" -> importPools();
            case "import_submissions" -> importSubmissions();
            default -> throw new IllegalArgumentException(scenario);
        }

        server.stop(0);
        Files.write(out.resolve("result.txt"), results, StandardCharsets.UTF_8);
        if (!stub.isEmpty()) {
            List<String> stubLines = stub.entrySet().stream().sorted(Map.Entry.comparingByKey())
                    .map(e -> e.getKey() + "\t" + e.getValue()).toList();
            Files.write(out.resolve("stub.tsv"), stubLines, StandardCharsets.UTF_8);
        }
        LogManager.shutdown();
        Path log = Path.of("logs", "latest.log");
        List<String> lines = new ArrayList<>();
        if (Files.exists(log)) {
            for (String line : Files.readAllLines(log, StandardCharsets.UTF_8)) {
                // [HH:mm:ss] [thread/LEVEL] [Logger]: message
                int slash = line.indexOf('/');
                int close = line.indexOf("] [", slash);
                int colon = line.indexOf("]: ", close);
                if (slash < 0 || close < 0 || colon < 0) {
                    lines.add(line); // continuation line of a stack trace
                    continue;
                }
                lines.add(line.substring(slash + 1, close) + " " + line.substring(colon + 3));
            }
        }
        Files.write(out.resolve("log.txt"), lines, StandardCharsets.UTF_8);
        System.exit(0);
    }

    static void serve(HttpExchange ex) throws java.io.IOException {
        String path = ex.getRequestURI().toString().substring(1);
        requests.add(path);
        String body = stub.get(path);
        byte[] bytes = body == null ? "{\"detail\":\"Not found\"}".getBytes(StandardCharsets.UTF_8)
                : body.getBytes(StandardCharsets.UTF_8);
        ex.sendResponseHeaders(body == null ? 404 : 200, bytes.length);
        try (OutputStream o = ex.getResponseBody()) {
            o.write(bytes);
        }
    }

    // ===== helpers =====

    static void result(String key, Object value) {
        results.add(key + "\t" + String.valueOf(value).replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t"));
    }

    static void input(String name, String content) throws Exception {
        Files.writeString(Path.of("data", name), content, StandardCharsets.UTF_8);
        Path target = out.resolve("input").resolve(name);
        Files.createDirectories(target.getParent());
        Files.writeString(target, content, StandardCharsets.UTF_8);
    }

    static void snapshot(String label, String... names) throws Exception {
        for (String name : names) {
            Path source = Path.of("data", name);
            Path target = out.resolve(label).resolve(name);
            Files.createDirectories(target.getParent());
            if (Files.exists(source)) {
                Files.copy(source, target, StandardCopyOption.REPLACE_EXISTING);
            }
        }
    }

    static String error(Runnable action) {
        try {
            action.run();
            return "ok";
        } catch (Throwable t) {
            return t.getClass().getSimpleName() + ": " + t.getMessage();
        }
    }

    interface IoAction {
        Object run() throws Exception;
    }

    static String attempt(IoAction action) {
        try {
            return "ok " + action.run();
        } catch (Throwable t) {
            return t.getClass().getSimpleName() + ": " + t.getMessage();
        }
    }

    static String pools(List<ChartPool.PoolSnapshot> pools) {
        return pools.stream().map(p -> p.id() + ":" + p.order() + ":" + p.category() + ":" + p.defaultFlag()
                + ":" + p.chartIds()).collect(Collectors.joining(" "));
    }

    /** A chart as Phira answers it. Durations only appear in our own files. */
    static String chart(int id, String name, String tag, double rating, int votes, double difficulty, Integer duration) {
        return "{\"id\":" + id + ",\"name\":" + quote(name) + ",\"level\":\"AT Lv.17\",\"difficulty\":" + difficulty
                + ",\"charter\":\"ch" + id + "\",\"ranked\":" + (id % 2 == 0) + ",\"stable\":true"
                + ",\"stableRequest\":false,\"file\":\"https://files.example/" + id + "\",\"uploader\":" + (id % 7)
                + ",\"tags\":[\"" + tag + "\"],\"rating\":" + rating + ",\"ratingCount\":" + votes
                + ",\"created\":\"2025-01-02T03:04:05.123456+08:00\",\"updated\":\"2025-03-04T05:06:07Z\""
                + ",\"chartUpdated\":\"2025-03-04T05:06:00Z\""
                + (duration == null ? "" : ",\"duration_seconds\":" + duration) + "}";
    }

    static String quote(String s) {
        StringBuilder b = new StringBuilder("\"");
        for (char c : s.toCharArray()) {
            if (c == '\\' || c == '"') {
                b.append('\\').append(c);
            } else if (c < 0x20) {
                b.append(String.format("\\u%04x", (int) c));
            } else {
                b.append(c);
            }
        }
        return b.append('"').toString();
    }

    static String page(int count, String... charts) {
        return "{\"count\":" + count + ",\"results\":[" + String.join(",", charts) + "]}";
    }

    /** Cache of the four default pool charts (so preload needs no network), deliberately unsorted. */
    static String defaultCache() {
        return "{\"52087\":" + chart(52087, "Fourth", "plain", 0.91, 12, 15.5, null)
                + ",\"30474\":" + chart(30474, "Spe<c>ial & 'q' = \"x\" 测试曲", "regular", 0.93999, 10, 17.5, 400)
                + ",\"33530\":" + chart(33530, "Third", "regular", 0.84, 5, 17.0, 359)
                + ",\"42058\":" + chart(42058, "Second\u2028line", "regular", 0.85, 5, 16.9, null)
                + ",\"99\":" + chart(99, "Ninety-nine", "plain", 1.0E-4, 0, 1.0E7, 361) + "}";
    }

    // ===== pools =====

    static void poolDefaults() throws Exception {
        input("chart-info-cache.json", defaultCache());
        ChartPool.preload();
        snapshot("output", "chart-pools.json", "chart-info-cache.json");
        result("listPools", pools(ChartPool.listPools()));
        result("defaultPools", pools(ChartPool.getDefaultPools()));
        result("chart30474", ChartPool.getChartInfo(30474).getDurationSeconds());
        result("missing", error(() -> ChartPool.getChartInfo(1)));
        result("isLoaded", ChartPool.isChartInfoLoaded(42058) + " " + ChartPool.isChartInfoLoaded(1));
    }

    static void poolNormalize() throws Exception {
        input("chart-info-cache.json", defaultCache());
        input("chart-pools.json", "{\"pools\":[null,{\"id\":5},"
                + "{\"id\":7,\"favorite_id\":3,\"default_flag\":true,\"chart_ids\":[30474,30474,42058,30474],"
                + "\"category\":\"BOGUS\",\"size_limit\":15,\"rounds_per_stay\":2,\"order\":20,\"submission_open\":true,"
                + "\"unknown\":1},"
                + "{\"id\":8,\"category\":\"TB\",\"chart_ids\":[33530],\"order\":20,\"favorite_id\":null},"
                + "{\"id\":2,\"category\":\"regular\",\"chart_ids\":[52087],\"order\":-5}]}");
        ChartPool.preload();
        snapshot("output", "chart-pools.json", "chart-info-cache.json");
        result("listPools", pools(ChartPool.listPools()));
        result("defaultPools", pools(ChartPool.getDefaultPools()));
        result("resolve", pools(ChartPool.resolvePools(List.of(8, 5, 7, 8, 99, 2))));
        result("byCategory", ChartPool.poolIdsByCategory(PoolCategory.MANUAL));
        result("open", pools(ChartPool.listOpenForSubmission()));
    }

    static void poolLoadEdges() throws Exception {
        input("chart-info-cache.json", defaultCache());
        String[][] cases = {
                {"null_pools", "{\"pools\":null}"},
                {"empty_file", ""},
                {"null_doc", "null"},
                {"empty_object", "{}"},
                {"empty_list", "{\"pools\":[]}"},
                {"only_null_rows", "{\"pools\":[null]}"},
                {"truncated", "{\"pools\":[{\"id\":10,\"chart_ids\":[30474]},{\"id\":11,\"cha"},
                {"bad_chart_id", "{\"pools\":[{\"id\":10,\"chart_ids\":[\"x\"]}]}"},
                {"null_chart_id", "{\"pools\":[{\"id\":10,\"chart_ids\":[30474,null]}]}"},
                {"string_numbers", "{\"pools\":[{\"id\":\"12\",\"chart_ids\":[\"30474\",42058.0],\"order\":\"3\",\"default_flag\":\"TRUE\"}]}"},
        };
        for (String[] c : cases) {
            Files.writeString(Path.of("data", "chart-pools.json"), c[1], StandardCharsets.UTF_8);
            Path target = out.resolve("input").resolve(c[0] + ".json");
            Files.createDirectories(target.getParent());
            Files.writeString(target, c[1], StandardCharsets.UTF_8);
            String r = attempt(() -> {
                ChartPool.preload();
                return pools(ChartPool.listPools());
            });
            result(c[0], r);
            Path written = out.resolve("output").resolve(c[0] + ".json");
            Files.createDirectories(written.getParent());
            Files.copy(Path.of("data", "chart-pools.json"), written, StandardCopyOption.REPLACE_EXISTING);
        }
    }

    static void poolOps() throws Exception {
        input("chart-info-cache.json", defaultCache());
        // The index generation draws from; ids 1000.. are regular, rated in descending order.
        StringBuilder index = new StringBuilder("{");
        for (int i = 0; i < 7; i++) {
            int id = 1000 + i;
            index.append(i == 0 ? "" : ",").append('"').append(id).append("\":")
                    .append(chart(id, "Gen" + id, "regular", 0.95 - i * 0.01, 10, 17.5, i == 3 ? 400 : null));
        }
        index.append(",\"2000\":").append(chart(2000, "Low", "regular", 0.5, 10, 17.5, null));
        index.append(",\"30474\":").append(chart(30474, "Spe<c>ial & 'q' = \"x\" 测试曲", "regular", 0.93999, 10, 17.5, 400));
        index.append("}");
        input("chart-index.json", index.toString());
        ChartPool.preload();
        ChartIndex.preload();

        result("addPool10", attempt(() -> {
            ChartPool.addPool(10, List.of(30474, 42058, 30474), PoolCategory.REGULAR, 15, 3);
            return "";
        }));
        result("addPoolDup", attempt(() -> {
            ChartPool.addPool(10, List.of(), null, null, null);
            return "";
        }));
        result("addPoolEmpty", attempt(() -> {
            ChartPool.addPool(20, List.of());
            return "";
        }));
        result("emptyPools", attempt(() -> ChartPool.createEmptyPools(2, PoolCategory.TB, 15, 1)));
        result("emptyPoolsNull", attempt(() -> ChartPool.createEmptyPools(1, null, null, null)));
        result("emptyPools0", attempt(() -> ChartPool.createEmptyPools(0, null, null, null)));
        result("emptyPools51", attempt(() -> ChartPool.createEmptyPools(51, null, null, null)));
        result("update10", error(() -> ChartPool.updatePool(10, PoolCategory.CONFIGURED, -1, 5, 7, true)));
        result("update10b", error(() -> ChartPool.updatePool(10, null, 9, null, -3, null)));
        result("update1", error(() -> ChartPool.updatePool(1, null, null, -1, null, false)));
        result("updateMissing", error(() -> ChartPool.updatePool(99, null, null, null, null, null)));
        result("favorite0", error(() -> ChartPool.setFavoriteId(0, 42)));
        result("favorite10", error(() -> ChartPool.setFavoriteId(10, null)));
        result("default1", error(() -> ChartPool.setDefaultFlag(1, true)));
        result("removeChart", error(() -> ChartPool.removeChart(0, 42058)));
        result("removeChartAbsent", error(() -> ChartPool.removeChart(0, 1)));
        result("removeChartMissing", error(() -> ChartPool.removeChart(99, 1)));
        result("addCharts", attempt(() -> ChartPool.addCharts(1, List.of(30474, 33530, 30474, 42058))));
        result("addChartsNone", attempt(() -> ChartPool.addCharts(1, List.of(30474))));
        result("addChartsMissingPool", attempt(() -> ChartPool.addCharts(99, List.of(30474))));
        List<Integer> many = new ArrayList<>();
        for (int i = 0; i < 501; i++) {
            many.add(100000 + i);
        }
        result("addCharts501", attempt(() -> ChartPool.addCharts(1, many)));
        result("generateBad", attempt(() -> ChartPool.generatePools(ScreeningRule.regular(), 0, null)));
        result("generate", attempt(() -> ChartPool.generatePools(ScreeningRule.regular(), 2, 3)));
        result("generateTb", attempt(() -> ChartPool.generatePools(ScreeningRule.tb(), 15, null)));
        result("generateNone", attempt(() -> ChartPool.generatePools(ScreeningRule.configured(), 15, null)));
        result("byCategory", ChartPool.poolIdsByCategory(PoolCategory.REGULAR));
        result("open", pools(ChartPool.listOpenForSubmission()));
        result("isOpen10", attempt(() -> ChartPool.isSubmissionOpen(10)));
        result("isOpenMissing", attempt(() -> ChartPool.isSubmissionOpen(99)));
        result("removePoolMissing", error(() -> ChartPool.removePool(99)));
        result("removePool20", error(() -> ChartPool.removePool(20)));
        result("default1off", error(() -> ChartPool.setDefaultFlag(1, false)));
        result("removeLastDefault0", error(() -> ChartPool.removePool(0)));
        result("default1on", error(() -> ChartPool.setDefaultFlag(1, true)));
        result("removeDefault0", error(() -> ChartPool.removePool(0)));
        result("default1unchanged", error(() -> ChartPool.setDefaultFlag(1, true)));
        result("listPools", pools(ChartPool.listPools()));
        result("defaultPools", pools(ChartPool.getDefaultPools()));
        snapshot("output", "chart-pools.json", "chart-info-cache.json");
        // Remove down to the last pool.
        List<ChartPool.PoolSnapshot> remaining = ChartPool.listPools();
        for (ChartPool.PoolSnapshot pool : remaining.subList(1, remaining.size())) {
            result("removeRest" + pool.id(), error(() -> ChartPool.removePool(pool.id())));
        }
        int last = remaining.get(0).id();
        result("removeLast", error(() -> ChartPool.removePool(last)));
        result("finalPools", pools(ChartPool.listPools()));
        snapshot("final", "chart-pools.json");
    }

    // ===== index =====

    static void indexSave() throws Exception {
        input("chart-index.json", "{\"1002\":" + chart(1002, "B", "plain", 0.9, 5, 3.0, null)
                + ",\"-5\":" + chart(-5, "Neg", "regular", 0.1, 1, 1.0, 12)
                + ",\"1000\":" + chart(1000, "A\u0000\u001f\u007f", "regular", 0.94, 10, 17.5, 400)
                + ",\"30474\":" + chart(30474, "Spe<c>ial & 'q' = \"x\" 测试曲", "regular", 0.93999, 10, 17.5, null)
                + ",\"7\":{\"id\":7}}");
        ChartIndex.preload();
        Method save = ChartIndex.class.getDeclaredMethod("save");
        save.setAccessible(true);
        save.invoke(null);
        snapshot("output", "chart-index.json");
        result("indexed", ChartIndex.indexedCount());
        result("search", ChartIndex.search(ScreeningRule.regular(), 10).stream().map(ChartInfo::getId).toList());
    }

    static void indexUnreadable() throws Exception {
        input("chart-index.json", "{\"1\":{\"id\":1},\"2\":");
        ChartIndex.preload();
        result("indexed", ChartIndex.indexedCount());
    }

    static void refreshAndRecord(String division) throws Exception {
        result("refresh", attempt(() -> ChartIndex.refresh(division)));
        result("requests", String.join(" ", requests));
        result("indexed", ChartIndex.indexedCount());
        result("remote", ChartIndex.remoteCount());
        result("refreshing", ChartIndex.isRefreshing());
        snapshot("output", "chart-index.json");
    }

    static String charts(int from, int to, String tag) {
        List<String> list = new ArrayList<>();
        for (int id = from; id <= to; id++) {
            list.add(chart(id, "C" + id, tag, 0.9, 10, 17.0, null));
        }
        return String.join(",", list);
    }

    /** Fresh deployment: the plain pass fills the index first, so the default pass is incremental. */
    static void sweepFresh() throws Exception {
        stub.put("chart?page=1&division=plain", page(5, charts(1, 3, "plain")));
        stub.put("chart?page=2&division=plain", page(5, charts(4, 5, "plain")));
        for (int p = 1; p <= 50; p++) {
            stub.put("chart?page=" + p, page(150, charts(100 + p * 3, 102 + p * 3, "regular")));
        }
        refreshAndRecord(null);
    }

    static void sweepFreshNoPlain() throws Exception {
        stub.put("chart?page=1&division=plain", page(0));
        for (int p = 1; p <= 5; p++) {
            stub.put("chart?page=" + p, page(8, charts(100 + p * 3, 102 + p * 3, "regular")));
        }
        refreshAndRecord(null);
    }

    static void sweepIncremental() throws Exception {
        input("chart-index.json", "{\"1\":" + chart(1, "P1", "plain", 0.9, 10, 3.0, 200)
                + ",\"10\":" + chart(10, "Old ten", "regular", 0.9, 10, 17.0, 333)
                + ",\"11\":" + chart(11, "Eleven", "regular", 0.9, 10, 17.0, null) + "}");
        ChartIndex.preload();
        stub.put("chart?page=1", page(500, chart(20, "New", "regular", 0.9, 10, 17.0, null),
                chart(21, "New2", "regular", 0.9, 10, 17.0, null), chart(10, "New ten", "regular", 0.95, 11, 17.0, null)));
        stub.put("chart?page=2", page(500, chart(11, "Eleven v2", "regular", 0.9, 10, 17.0, null),
                chart(10, "Ten v3", "regular", 0.9, 10, 17.0, null)));
        stub.put("chart?page=3", page(500, chart(30, "Never", "regular", 0.9, 10, 17.0, null)));
        refreshAndRecord(null);
    }

    static void sweepDivision() throws Exception {
        input("chart-index.json", "{\"1\":" + chart(1, "P1", "plain", 0.9, 10, 3.0, 200) + "}");
        ChartIndex.preload();
        stub.put("chart?page=1&division=plain", page(9, chart(1, "P1 again", "plain", 0.9, 10, 3.0, null),
                chart(2, "P2", "plain", 0.9, 10, 3.0, null)));
        stub.put("chart?page=2&division=plain", page(9, chart(1, "P1 again", "plain", 0.9, 10, 3.0, null)));
        stub.put("chart?page=3&division=plain", page(9, chart(3, "P3", "plain", 0.9, 10, 3.0, null)));
        refreshAndRecord("plain");
    }

    static void sweepEmptyPage() throws Exception {
        input("chart-index.json", "{\"1\":" + chart(1, "P1", "plain", 0.9, 10, 3.0, null) + "}");
        ChartIndex.preload();
        stub.put("chart?page=1", page(100, charts(50, 52, "regular")));
        stub.put("chart?page=2", "{\"count\":100,\"results\":[]}");
        stub.put("chart?page=3", page(100, charts(60, 62, "regular")));
        refreshAndRecord(null);
    }

    // ===== submissions =====

    static String views(List<SubmissionService.View> views) {
        return views.stream().map(v -> v.poolId() + "/" + v.chartId() + "/" + v.status() + "/"
                + v.submitters().stream().map(s -> s.userId() + "=" + s.name()).toList() + "/"
                + v.reviewerId() + "/" + v.reason()).collect(Collectors.joining(" "));
    }

    static String submitResults(List<SubmissionService.Result> list) {
        return list.stream().map(r -> r.chartId() + ":" + r.accepted() + ":" + r.reason())
                .collect(Collectors.joining(" "));
    }

    static void submissionsOps() throws Exception {
        SubmissionService.preload();
        result("submit1", submitResults(SubmissionService.submit(0, List.of(1, 2, 3, 2), 7, "Alice")));
        result("submit2", submitResults(SubmissionService.submit(0, List.of(2), 8, "Bob <&> \"q\"")));
        result("submit3", submitResults(SubmissionService.submit(5, List.of(2), 8, null)));
        result("approve", SubmissionService.review(0, 1, true, 99, null));
        result("approveMissing", SubmissionService.review(0, 42, true, 99, null));
        result("submitApproved", submitResults(SubmissionService.submit(0, List.of(1), 8, "Bob")));
        result("reject", SubmissionService.review(0, 3, false, 98, "太短"));
        result("submitRejected", submitResults(SubmissionService.submit(0, List.of(3), 9, "Carol")));
        result("withdraw", SubmissionService.withdraw(0, 2, 7));
        result("withdrawApproved", SubmissionService.withdraw(0, 1, 7));
        result("withdrawMissing", SubmissionService.withdraw(5, 5, 7));
        result("withdrawNotMine", SubmissionService.withdraw(0, 2, 7));
        result("withdrawLast", SubmissionService.withdraw(5, 2, 8));
        for (int u = 0; u < 200; u++) {
            SubmissionService.submit(1, List.of(9), 1000 + u, "u" + u);
        }
        result("submitFull", submitResults(SubmissionService.submit(1, List.of(9), 5000, "late")));
        result("byPool0", views(SubmissionService.listByPool(0, null)));
        result("byPool0Pending", views(SubmissionService.listByPool(0, SubmissionService.Status.PENDING)));
        result("pending", SubmissionService.listPending().size());
        result("bySubmitter8", views(SubmissionService.listBySubmitter(8)));
        snapshot("output", "submissions.json");
    }

    static void submissionsOdd() throws Exception {
        input("submissions.json", "{\"submissions\":["
                + "{\"pool_id\":1,\"chart_id\":5,\"status\":null,\"submitters\":[{\"user_id\":3,\"name\":\"X\",\"at\":\"2025-01-01T00:00:00.500+08:00\"}],\"created_at\":\"2025-01-01T00:00Z\"},"
                + "{\"pool_id\":1,\"chart_id\":6,\"status\":\"BOGUS\",\"created_at\":\"2025-01-02T00:00Z\"},"
                + "{\"pool_id\":1,\"chart_id\":7,\"submitters\":[{\"user_id\":4}]},"
                + "{\"pool_id\":0,\"chart_id\":9,\"status\":\"APPROVED\",\"reviewer_id\":2,\"reason\":\"r1\",\"reviewed_at\":\"2025-02-01T10:00:00+09:00\",\"submitters\":[{\"user_id\":4,\"name\":\"Y\"}],\"created_at\":\"2025-01-01T23:00-01:00\"},"
                + "{\"pool_id\":0,\"chart_id\":9,\"status\":\"REJECTED\",\"reason\":\"later row wins\",\"submitters\":[{\"user_id\":4,\"name\":\"Y\"}],\"created_at\":\"2025-01-01T00:00Z\"}"
                + "]}");
        SubmissionService.preload();
        result("byPool1", views(SubmissionService.listByPool(1, null)));
        result("byPool0", views(SubmissionService.listByPool(0, null)));
        result("pending", views(SubmissionService.listPending()));
        result("bySubmitter4", views(SubmissionService.listBySubmitter(4)));
        // A no-op withdraw saves nothing; a successful one rewrites the whole file.
        result("withdraw", SubmissionService.withdraw(1, 5, 99));
        result("withdrawReal", SubmissionService.withdraw(1, 7, 4));
        snapshot("output", "submissions.json");
    }

    static void submissionsNullRow() throws Exception {
        input("submissions.json", "{\"submissions\":[null]}");
        SubmissionService.preload();
        result("pending", SubmissionService.listPending().size());
    }

    // ===== import =====

    static void importPools() throws Exception {
        input("chart-info-cache.json", defaultCache());
        ChartPool.preload();
        result("merge", attempt(() -> DataTransferService.importJson(DataTransferService.Kind.POOLS,
                "{\"pools\":[{\"id\":1,\"chart_ids\":[30474],\"category\":\"TB\",\"order\":4},{\"id\":77,\"chart_ids\":[42058,42058]}]}",
                true)));
        snapshot("after_merge", "chart-pools.json");
        result("afterMerge", pools(ChartPool.listPools()));
        result("replace", attempt(() -> DataTransferService.importJson(DataTransferService.Kind.POOLS,
                "{\"pools\":[{\"id\":3,\"chart_ids\":[33530],\"default_flag\":true},{\"id\":4}]}", false)));
        snapshot("after_replace", "chart-pools.json");
        result("afterReplace", pools(ChartPool.listPools()));
        result("export", DataTransferService.exportJson(DataTransferService.Kind.POOLS));
    }

    static void importSubmissions() throws Exception {
        SubmissionService.preload();
        result("exportEmpty", DataTransferService.exportJson(DataTransferService.Kind.SUBMISSIONS));
        SubmissionService.submit(0, List.of(1008, 1009), 7, "Alice");
        String doc = "{\"submissions\":[{\"pool_id\":0,\"chart_id\":1008,\"status\":\"REJECTED\",\"submitters\":[{\"user_id\":8,\"name\":\"Bob\"}],\"created_at\":\"2025-01-01T00:00Z\"},"
                + "{\"pool_id\":2,\"chart_id\":1,\"submitters\":[]}]}";
        result("merge", attempt(() -> DataTransferService.importJson(DataTransferService.Kind.SUBMISSIONS, doc, true)));
        result("afterMerge", views(SubmissionService.listByPool(0, null)) + " | " + views(SubmissionService.listByPool(2, null)));
        result("replace", attempt(() -> DataTransferService.importJson(DataTransferService.Kind.SUBMISSIONS, doc, false)));
        result("afterReplace", views(SubmissionService.listByPool(0, null)) + " | " + views(SubmissionService.listByPool(2, null)));
        snapshot("after_replace", "submissions.json");
    }
}
