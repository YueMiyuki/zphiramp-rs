import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.LogEvent;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.appender.AbstractAppender;
import org.apache.logging.log4j.core.config.Configuration;
import org.apache.logging.log4j.core.config.Property;
import top.rymc.phira.main.Server;
import top.rymc.phira.main.config.ServerArgs;
import top.rymc.phira.main.game.record.RoundRecordService;
import top.rymc.phira.main.game.room.chart.ChartIndex;
import top.rymc.phira.main.game.room.chart.ChartPool;
import top.rymc.phira.main.game.room.chart.SubmissionService;
import top.rymc.phira.main.http.ApiServer;
import top.rymc.phira.main.http.DataTransferService;
import top.rymc.phira.main.http.JwtService;
import top.rymc.phira.main.util.PhiraFetcher;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.reflect.Field;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.zip.GZIPInputStream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

/**
 * Golden HTTP exchanges for zphira-http, recorded from the real ApiServer (Javalin 7.2.3 on
 * Jetty 12) running the real compiled classes against a local Phira stub. Never contacts the
 * real Phira service.
 *
 * Usage (cwd = a fresh, empty scratch directory):
 *   java -cp CP GenHttp.java SCENARIO SEED_DIR OUT_DIR
 *
 * The scratch directory receives SEED_DIR/data as data/ and SEED_DIR/frontend as frontend/.
 * Writes OUT_DIR/SCENARIO.json: every request as sent (with placeholders such as {admin} left
 * unresolved) and the raw response (status, headers except Date, body), plus the log lines
 * emitted while it was handled.
 *
 * Placeholders: {admin} {player} = bearer tokens of users 1 (admin) and 2; {user5} = user 5
 * with an isAdmin=false claim; {fakeadmin2} = user 2 with an isAdmin=true claim;
 * {backup:KIND:N} = the N-th name of the current backup listing of KIND.
 */
public final class GenHttp {

    static int stubPort;
    static int apiPort;
    static final Map<String, String> vars = new ConcurrentHashMap<>();
    static final List<String> logs = Collections.synchronizedList(new ArrayList<>());
    static final JsonArray exchanges = new JsonArray();
    static JsonObject indexCharts;

    public static void main(String[] a) throws Exception {
        String scenario = a[0];
        Path seed = Path.of(a[1]).toAbsolutePath();
        Path out = Path.of(a[2]).toAbsolutePath();
        copyDir(seed.resolve("data"), Path.of("data"));
        copyDir(seed.resolve("frontend"), Path.of("frontend"));
        if (scenario.equals("static_missing")) {
            // the web manager was never deployed: no frontend/dist at all
            try (var walk = Files.walk(Path.of("frontend", "dist"))) {
                for (Path p : walk.sorted(java.util.Comparator.reverseOrder()).toList()) Files.delete(p);
            }
        }
        indexCharts = JsonParser.parseString(Files.readString(Path.of("data", "chart-index.json"))).getAsJsonObject();

        startStub();
        PhiraFetcher.setHost("http://127.0.0.1:" + stubPort + "/");
        Field f = Server.class.getDeclaredField("args");
        f.setAccessible(true);
        f.set(Server.getInstance(), new ServerArgs(new String[0]));

        ChartPool.preload();
        ChartIndex.preload();
        SubmissionService.preload();
        RoundRecordService.preload();

        if (scenario.equals("bind")) {
            installCapture();
            bind();
            writeOut(scenario, out);
            System.exit(0);
        }
        try (ServerSocket socket = new ServerSocket(0)) {
            apiPort = socket.getLocalPort();
        }
        ApiServer.start("127.0.0.1", apiPort);
        ApiServer.awaitStarted();
        installCapture();

        vars.put("admin", JwtService.issueToken(1, true));
        vars.put("player", JwtService.issueToken(2, false));
        vars.put("user5", JwtService.issueToken(5, false));
        vars.put("fakeadmin2", JwtService.issueToken(2, true));

        switch (scenario) {
            case "framework" -> framework();
            case "pools" -> pools();
            case "submissions" -> submissions();
            case "charts" -> charts();
            case "data" -> data();
            case "admin" -> admin();
            case "records" -> records();
            case "rooms" -> rooms();
            case "static_missing" -> staticMissing();
            default -> throw new IllegalArgumentException(scenario);
        }

        writeOut(scenario, out);
        System.exit(0);
    }

    static void writeOut(String scenario, Path out) throws Exception {
        JsonObject doc = new JsonObject();
        doc.addProperty("scenario", scenario);
        doc.add("exchanges", exchanges);
        Gson gson = new GsonBuilder().setPrettyPrinting().disableHtmlEscaping().serializeNulls().create();
        Files.createDirectories(out);
        Files.writeString(out.resolve(scenario + ".json"), gson.toJson(doc) + "\n");
    }

    // ===================================================================== scenarios

    static final String[][] ADMIN = {{"Authorization", "Bearer {admin}"}};
    static final String[][] PLAYER = {{"Authorization", "Bearer {player}"}};
    static final String[][] JSON_ADMIN = {{"Authorization", "Bearer {admin}"}, {"Content-Type", "application/json"}};
    static final String[][] JSON_PLAYER = {{"Authorization", "Bearer {player}"}, {"Content-Type", "application/json"}};
    static final String[][] NONE = {};

    static void framework() throws Exception {
        // ---- authentication and CORS
        ex("auth: no header", "GET", "/api/v1/pool/list", NONE, null);
        ex("auth: lower-case scheme", "GET", "/api/v1/pool/list", h("Authorization", "bearer {admin}"), null);
        ex("auth: empty token", "GET", "/api/v1/pool/list", h("Authorization", "Bearer "), null);
        ex("auth: scheme only", "GET", "/api/v1/pool/list", h("Authorization", "Bearer"), null);
        ex("auth: garbage token", "GET", "/api/v1/pool/list", h("Authorization", "Bearer abc.def.ghi"), null);
        ex("auth: two spaces", "GET", "/api/v1/pool/list", h("Authorization", "Bearer  {admin}"), null);
        ex("auth: trailing space", "GET", "/api/v1/pool/list", h("Authorization", "Bearer {admin} "), null);
        ex("auth: basic", "GET", "/api/v1/pool/list", h("Authorization", "Basic dXNlcjpwYXNz"), null);
        ex("auth: admin", "GET", "/api/v1/pool/list", ADMIN, null);
        ex("auth: player is not admin", "GET", "/api/v1/pool/list", PLAYER, null);
        ex("auth: isAdmin claim not trusted", "GET", "/api/v1/pool/list", h("Authorization", "Bearer {fakeadmin2}"), null);
        ex("auth: user route as player", "GET", "/api/v1/submission/mine", PLAYER, null);
        ex("auth: origin header", "GET", "/api/v1/pool/list", h("Authorization", "Bearer {admin}", "Origin", "http://evil.example"), null);

        // ---- unknown routes, wrong methods
        ex("unknown api GET no auth", "GET", "/api/v1/nope", NONE, null);
        ex("unknown api GET", "GET", "/api/v1/nope", ADMIN, null);
        ex("unknown api POST", "POST", "/api/v1/nope", ADMIN, "{}");
        ex("unknown api POST accept json", "POST", "/api/v1/nope", h("Authorization", "Bearer {admin}", "Accept", "application/json"), null);
        ex("unknown api POST accept html", "POST", "/api/v1/nope", h("Authorization", "Bearer {admin}", "Accept", "text/html,application/xhtml+xml,*/*"), null);
        ex("unknown api POST accept any", "POST", "/api/v1/nope", h("Authorization", "Bearer {admin}", "Accept", "*/*"), null);
        ex("wrong method PUT pool/list", "PUT", "/api/v1/pool/list", ADMIN, "{}");
        ex("wrong method PATCH pool/list", "PATCH", "/api/v1/pool/list", ADMIN, "{}");
        ex("wrong method DELETE login", "DELETE", "/api/v1/login", NONE, null);
        ex("wrong method GET login", "GET", "/api/v1/login", NONE, null);
        ex("unknown method", "FOO", "/api/v1/pool/list", ADMIN, null);
        ex("method override", "POST", "/api/v1/pool/list", h("Authorization", "Bearer {admin}", "X-HTTP-Method-Override", "GET"), null);
        ex("method override unknown", "POST", "/api/v1/pool/list", h("Authorization", "Bearer {admin}", "X-HTTP-Method-Override", "BREW"), null);
        ex("api root no slash", "GET", "/api/v1", NONE, null);
        ex("api root slash", "GET", "/api/v1/", NONE, null);
        ex("api root slash auth", "GET", "/api/v1/", ADMIN, null);
        ex("api bare", "GET", "/api", NONE, null);
        ex("api slash", "GET", "/api/", NONE, null);
        ex("api other version", "GET", "/api/v2/pool/list", NONE, null);
        ex("apix", "GET", "/apix", NONE, null);
        ex("POST root", "POST", "/", NONE, "{}");
        ex("DELETE static", "DELETE", "/index.html", NONE, null);

        // ---- trailing slashes, case, doubled slashes
        ex("trailing slash", "GET", "/api/v1/pool/list/", ADMIN, null);
        ex("double trailing slash", "GET", "/api/v1/pool/list//", ADMIN, null);
        ex("double slash inside", "GET", "/api/v1//pool/list", ADMIN, null);
        ex("leading double slash", "GET", "//api/v1/pool/list", ADMIN, null);
        ex("upper case path", "GET", "/api/v1/Pool/List", ADMIN, null);
        ex("login trailing slash", "POST", "/api/v1/login/", NONE, "{\"email\":\"user1@x\",\"password\":\"right\"}");
        ex("record id trailing slash", "GET", "/api/v1/record/r1-1735689600000-1/", ADMIN, null);
        ex("record player empty", "GET", "/api/v1/record/player/", ADMIN, null);

        // ---- HEAD
        ex("HEAD api auth", "HEAD", "/api/v1/pool/list", ADMIN, null);
        ex("HEAD api no auth", "HEAD", "/api/v1/pool/list", NONE, null);
        ex("HEAD api player", "HEAD", "/api/v1/pool/list", PLAYER, null);
        ex("HEAD unknown api", "HEAD", "/api/v1/nope", ADMIN, null);
        ex("HEAD delete-only route", "HEAD", "/api/v1/admin/5", ADMIN, null);
        ex("HEAD root", "HEAD", "/", NONE, null);
        ex("HEAD static file", "HEAD", "/style.css", NONE, null);
        ex("HEAD missing file", "HEAD", "/missing.png", NONE, null);
        ex("HEAD login", "HEAD", "/api/v1/login", NONE, null);

        // ---- OPTIONS
        ex("OPTIONS api", "OPTIONS", "/api/v1/pool/list", NONE, null);
        ex("OPTIONS preflight", "OPTIONS", "/api/v1/pool/1",
                h("Origin", "http://localhost:3000", "Access-Control-Request-Method", "DELETE",
                        "Access-Control-Request-Headers", "authorization,content-type"), null);
        ex("OPTIONS unknown api", "OPTIONS", "/api/v1/nope/deeper", NONE, null);
        ex("OPTIONS api root no slash", "OPTIONS", "/api/v1", NONE, null);
        ex("OPTIONS root", "OPTIONS", "/", NONE, null);
        ex("OPTIONS star", "OPTIONS", "*", NONE, null);

        // ---- request bodies (POST /pool/generate parses a body and answers without side effects)
        ex("body: empty", "POST", "/api/v1/pool/generate", JSON_ADMIN, "");
        ex("body: blank", "POST", "/api/v1/pool/generate", JSON_ADMIN, " \r\n\t ");
        ex("body: no content-length", "POST", "/api/v1/pool/generate", JSON_ADMIN, null);
        ex("body: null", "POST", "/api/v1/pool/generate", JSON_ADMIN, "null");
        ex("body: array", "POST", "/api/v1/pool/generate", JSON_ADMIN, "[1]");
        ex("body: string", "POST", "/api/v1/pool/generate", JSON_ADMIN, "\"MANUAL\"");
        ex("body: number", "POST", "/api/v1/pool/generate", JSON_ADMIN, "5");
        ex("body: syntax error", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":");
        ex("body: ok manual", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\"}");
        ex("body: trailing garbage", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\"} x");
        ex("body: trailing value", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\"}{}");
        ex("body: lenient", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{category:MANUAL,/*c*/'sizeLimit':'5'}");
        ex("body: lower-case enum", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"manual\"}");
        ex("body: unknown enum", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"BOGUS\"}");
        ex("body: enum number", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":5}");
        ex("body: enum bool", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":true}");
        ex("body: int field fraction", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\",\"sizeLimit\":1.5}");
        ex("body: int field string", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\",\"sizeLimit\":\"x\"}");
        ex("body: int field overflow", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\",\"sizeLimit\":2147483648}");
        ex("body: duplicate key", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"BOGUS\",\"category\":\"MANUAL\"}");
        ex("body: bom", "POST", "/api/v1/pool/generate", JSON_ADMIN, "﻿{\"category\":\"MANUAL\"}");
        ex("body: text/plain", "POST", "/api/v1/pool/generate", h("Authorization", "Bearer {admin}", "Content-Type", "text/plain"), "{\"category\":\"MANUAL\"}");
        ex("body: no content type", "POST", "/api/v1/pool/generate", ADMIN, "{\"category\":\"MANUAL\"}");
        ex("body: form content type", "POST", "/api/v1/pool/generate", h("Authorization", "Bearer {admin}", "Content-Type", "application/x-www-form-urlencoded"), "{\"category\":\"MANUAL\"}");
        ex("body: latin1 charset", "POST", "/api/v1/pool/generate", h("Authorization", "Bearer {admin}", "Content-Type", "application/json; charset=ISO-8859-1"),
                null, opt("bodyBase64", b64("{\"category\":\"MANÉ\"}".getBytes(StandardCharsets.ISO_8859_1))));
        ex("body: bogus charset", "POST", "/api/v1/pool/generate", h("Authorization", "Bearer {admin}", "Content-Type", "application/json; charset=bogus"), "{\"category\":\"MANUAL\"}");
        ex("body: invalid utf8", "POST", "/api/v1/pool/generate", JSON_ADMIN, null,
                opt("bodyBase64", b64(new byte[]{'{', '"', 'c', 'a', 't', 'e', 'g', 'o', 'r', 'y', '"', ':', '"', (byte) 0xC3, (byte) 0x28, '"', '}'})));
        ex("body: chunked", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\"}", opt("chunked", true));
        ex("body: exactly 1000000 bytes", "POST", "/api/v1/pool/generate", JSON_ADMIN, null,
                fill("{\"category\":\"MANUAL\"}", 1_000_000));
        ex("body: 1000001 bytes", "POST", "/api/v1/pool/generate", JSON_ADMIN, null,
                fill("{\"category\":\"MANUAL\"}", 1_000_001));
        ex("body: 1000001 bytes accept json", "POST", "/api/v1/pool/generate",
                h("Authorization", "Bearer {admin}", "Accept", "application/json"), null, fill("{\"category\":\"MANUAL\"}", 1_000_001));
        ex("body: 1000001 bytes chunked", "POST", "/api/v1/pool/generate", JSON_ADMIN, null,
                fill("{\"category\":\"MANUAL\"}", 1_000_001, "chunked"));
        ex("body: 1000001 bytes, not read by a GET", "GET", "/api/v1/pool/list", JSON_ADMIN, null,
                fill("{}", 1_000_001));
        ex("body: 1000001 bytes, no auth", "POST", "/api/v1/pool/generate", NONE, null,
                fill("{}", 1_000_001));
        ex("body: 1000001 bytes, favorite", "PUT", "/api/v1/pool/1/favorite", JSON_ADMIN, null,
                fill("{\"favoriteId\":1}", 1_000_001));
        ex("body: 1000001 bytes, import", "POST", "/api/v1/data/admins/import?mode=merge", JSON_ADMIN, null,
                fill("[1]", 1_000_001));

        // ---- path parameters
        ex("path int: ok", "GET", "/api/v1/record/player/101", ADMIN, null);
        ex("path int: letters", "GET", "/api/v1/record/player/abc", ADMIN, null);
        ex("path int: plus sign", "GET", "/api/v1/record/player/+101", ADMIN, null);
        ex("path int: encoded", "GET", "/api/v1/record/player/%31%30%31", ADMIN, null);
        ex("path int: encoded plus", "GET", "/api/v1/record/player/%2B101", ADMIN, null);
        ex("path int: space", "GET", "/api/v1/record/player/%20101", ADMIN, null);
        ex("path int: overflow", "GET", "/api/v1/record/player/2147483648", ADMIN, null);
        ex("path int: min", "GET", "/api/v1/record/player/-2147483648", ADMIN, null);
        ex("path int: unicode digits", "GET", "/api/v1/record/player/%E0%A5%A7%E0%A5%A6%E0%A5%A7", ADMIN, null);
        ex("path int: decimal", "GET", "/api/v1/record/player/1.0", ADMIN, null);
        ex("path: encoded slash", "GET", "/api/v1/record/a%2Fb", ADMIN, null);
        ex("path: bad escape", "GET", "/api/v1/record/%zz", ADMIN, null);
        ex("path: truncated escape", "GET", "/api/v1/record/abc%2", ADMIN, null);
        ex("path: plus kept", "GET", "/api/v1/record/a+b", ADMIN, null);
        ex("path: encoded record id", "GET", "/api/v1/record/r1%2D1735689600000-1", ADMIN, null);
        ex("path: missing segment", "DELETE", "/api/v1/pool//chart/5", ADMIN, null);
        ex("path: dot segment", "GET", "/api/v1/record/player/../list", ADMIN, null);
        ex("path: encoded dot segment", "GET", "/api/v1/record/player/%2e%2e/list", ADMIN, null);
        ex("path: semicolon", "GET", "/api/v1/record/list;jsessionid=1", ADMIN, null);
        ex("path: utf8 raw id", "GET", "/api/v1/record/%E4%B8%AD", ADMIN, null);

        // ---- query parameters
        ex("query: limit text", "GET", "/api/v1/record/list?limit=abc", ADMIN, null);
        ex("query: limit fraction", "GET", "/api/v1/record/list?limit=1.9", ADMIN, null);
        ex("query: limit negative", "GET", "/api/v1/record/list?limit=-5", ADMIN, null);
        ex("query: limit exponent", "GET", "/api/v1/record/list?limit=2e0", ADMIN, null);
        ex("query: limit NaN", "GET", "/api/v1/record/list?limit=NaN", ADMIN, null);
        ex("query: limit Infinity", "GET", "/api/v1/record/list?limit=Infinity", ADMIN, null);
        ex("query: limit blank", "GET", "/api/v1/record/list?limit=", ADMIN, null);
        ex("query: limit plus space", "GET", "/api/v1/record/list?limit=+2+", ADMIN, null);
        ex("query: limit encoded space", "GET", "/api/v1/record/list?limit=%202%20", ADMIN, null);
        ex("query: limit hex", "GET", "/api/v1/record/list?limit=0x10", ADMIN, null);
        ex("query: limit hex float", "GET", "/api/v1/record/list?limit=0x1p1", ADMIN, null);
        ex("query: limit suffix", "GET", "/api/v1/record/list?limit=1d", ADMIN, null);
        ex("query: limit twice", "GET", "/api/v1/record/list?limit=1&limit=3", ADMIN, null);
        ex("query: limit bad escape", "GET", "/api/v1/record/list?limit=%zz", ADMIN, null);
        ex("query: unrelated bad escape", "GET", "/api/v1/record/list?x=%zz", ADMIN, null);
        ex("query: no value", "GET", "/api/v1/record/list?limit", ADMIN, null);
        ex("query: offset negative", "GET", "/api/v1/record/list?offset=-3&limit=1", ADMIN, null);
        ex("query: offset huge", "GET", "/api/v1/record/list?offset=1e300", ADMIN, null);
        ex("query: playerId text", "GET", "/api/v1/record/list?playerId=x", ADMIN, null);
        ex("query: playerId fraction", "GET", "/api/v1/record/list?playerId=103.7", ADMIN, null);
        ex("query: roomId blank", "GET", "/api/v1/record/list?roomId=%20", ADMIN, null);
        ex("query: roomId", "GET", "/api/v1/record/list?roomId=r2", ADMIN, null);
        ex("query: error order", "GET", "/api/v1/record/list?playerId=x&offset=y&limit=z", ADMIN, null);
        ex("query: tag duplicate", "GET", "/api/v1/chart/search?tag=plain,plain", ADMIN, null);
        ex("query: utf8 key", "GET", "/api/v1/record/list?%E4%B8%AD=1", ADMIN, null);

        // ---- static files
        ex("static: root", "GET", "/", NONE, null);
        ex("static: index.html", "GET", "/index.html", NONE, null);
        ex("static: room fallback", "GET", "/room/abc", NONE, null);
        ex("static: room bare", "GET", "/room", NONE, null);
        ex("static: room prefix", "GET", "/roomy", NONE, null);
        ex("static: room dir", "GET", "/room/", NONE, null);
        ex("static: pool fallback", "GET", "/pool/7", NONE, null);
        ex("static: other route", "GET", "/records/list", NONE, null);
        ex("static: js", "GET", "/_next/static/chunks/app.js", NONE, null);
        ex("static: js gzip", "GET", "/_next/static/chunks/app.js", h("Accept-Encoding", "gzip"), null);
        ex("static: css", "GET", "/style.css", NONE, null);
        ex("static: jpg", "GET", "/logo.jpg", NONE, null);
        ex("static: json", "GET", "/data.json", NONE, null);
        ex("static: webmanifest", "GET", "/manifest.webmanifest", NONE, null);
        ex("static: txt", "GET", "/index.txt", NONE, null);
        ex("static: no extension", "GET", "/LICENSE", NONE, null);
        ex("static: upper-case extension", "GET", "/file.PNG", NONE, null);
        ex("static: space raw-encoded", "GET", "/a%20b.txt", NONE, null);
        ex("static: utf8 encoded", "GET", "/%E4%B8%AD%E6%96%87.txt", NONE, null);
        ex("static: nested", "GET", "/docs/readme.txt", NONE, null);
        ex("static: directory", "GET", "/docs", NONE, null);
        ex("static: directory slash", "GET", "/docs/", NONE, null);
        ex("static: missing with extension", "GET", "/missing.js", NONE, null);
        ex("static: missing nested with dot dir", "GET", "/v1.2/page", NONE, null);
        ex("static: query string", "GET", "/style.css?v=3", NONE, null);
        ex("static: traversal raw", "GET", "/../secret.txt", NONE, null);
        ex("static: traversal nested raw", "GET", "/room/../../secret.txt", NONE, null);
        ex("static: traversal encoded dots", "GET", "/%2e%2e/secret.txt", NONE, null);
        ex("static: traversal encoded slash", "GET", "/..%2fsecret.txt", NONE, null);
        ex("static: traversal backslash", "GET", "/..%5csecret.txt", NONE, null);
        ex("static: absolute", "GET", "//etc/passwd", NONE, null);
        ex("static: dot segment inside", "GET", "/docs/../style.css", NONE, null);
        ex("static: api unknown", "GET", "/api/unknown", NONE, null);
        ex("static: accept json", "GET", "/missing.js", h("Accept", "application/json"), null);
    }

    static void pools() throws Exception {
        ex("list", "GET", "/api/v1/pool/list", ADMIN, null);
        ex("list as player", "GET", "/api/v1/pool/list", PLAYER, null);
        ex("create: empty object", "POST", "/api/v1/pool", JSON_ADMIN, "{}");
        ex("create: no body", "POST", "/api/v1/pool", JSON_ADMIN, "");
        ex("create: as player", "POST", "/api/v1/pool", JSON_PLAYER, "{\"id\":50}");
        ex("create: full", "POST", "/api/v1/pool", JSON_ADMIN,
                "{\"id\":10,\"chartIds\":[1000,1002,1000],\"favoriteId\":77,\"category\":\"REGULAR\",\"sizeLimit\":15,\"roundsPerStay\":3}");
        ex("create: unknown chart", "POST", "/api/v1/pool", JSON_ADMIN, "{\"id\":11,\"chartIds\":[900001]}");
        ex("create: lower-case category", "POST", "/api/v1/pool", JSON_ADMIN, "{\"id\":12,\"category\":\"regular\",\"chartIds\":null,\"favoriteId\":null}");
        ex("create: unknown category", "POST", "/api/v1/pool", JSON_ADMIN, "{\"id\":13,\"category\":\"bogus\"}");
        ex("create: id text", "POST", "/api/v1/pool", JSON_ADMIN, "{\"id\":\"x\"}");
        ex("create: id null", "POST", "/api/v1/pool", JSON_ADMIN, "{\"id\":null}");
        ex("create: chart ids with null", "POST", "/api/v1/pool", JSON_ADMIN, "{\"id\":14,\"chartIds\":[1004,null]}");
        ex("list after create", "GET", "/api/v1/pool/list", ADMIN, null);
        ex("update: fields", "PUT", "/api/v1/pool/10", JSON_ADMIN,
                "{\"category\":\"TB\",\"sizeLimit\":-1,\"roundsPerStay\":5,\"order\":-5,\"submissionOpen\":true}");
        ex("update: no body", "PUT", "/api/v1/pool/10", JSON_ADMIN, "");
        ex("update: unknown pool", "PUT", "/api/v1/pool/999", JSON_ADMIN, "{}");
        ex("update: bad id", "PUT", "/api/v1/pool/abc", JSON_ADMIN, "{}");
        ex("update: lower-case category", "PUT", "/api/v1/pool/10", JSON_ADMIN, "{\"category\":\"configured\"}");
        ex("update: order huge", "PUT", "/api/v1/pool/1", JSON_ADMIN, "{\"order\":2147483647}");
        ex("batch: ok", "POST", "/api/v1/pool/batch", JSON_ADMIN, "{\"count\":2,\"category\":\"CONFIGURED\",\"sizeLimit\":10}");
        ex("batch: zero", "POST", "/api/v1/pool/batch", JSON_ADMIN, "{\"count\":0}");
        ex("batch: too many", "POST", "/api/v1/pool/batch", JSON_ADMIN, "{\"count\":51}");
        ex("batch: no body", "POST", "/api/v1/pool/batch", JSON_ADMIN, "");
        ex("chart add: ok", "POST", "/api/v1/pool/10/chart", JSON_ADMIN, "{\"chartId\":1004}");
        ex("chart add: unknown chart", "POST", "/api/v1/pool/10/chart", JSON_ADMIN, "{\"chartId\":900002}");
        ex("chart add: unknown pool", "POST", "/api/v1/pool/999/chart", JSON_ADMIN, "{\"chartId\":1004}");
        ex("chart add: no body", "POST", "/api/v1/pool/10/chart", JSON_ADMIN, "");
        ex("chart add: null id", "POST", "/api/v1/pool/10/chart", JSON_ADMIN, "{\"chartId\":null}");
        ex("charts add: ok", "POST", "/api/v1/pool/10/charts", JSON_ADMIN, "{\"chartIds\":[1004,1006,1006]}");
        ex("charts add: empty", "POST", "/api/v1/pool/10/charts", JSON_ADMIN, "{\"chartIds\":[]}");
        ex("charts add: missing", "POST", "/api/v1/pool/10/charts", JSON_ADMIN, "{}");
        ex("charts add: one fails", "POST", "/api/v1/pool/10/charts", JSON_ADMIN, "{\"chartIds\":[1008,900003]}");
        ex("charts add: unknown pool", "POST", "/api/v1/pool/999/charts", JSON_ADMIN, "{\"chartIds\":[1008]}");
        ex("chart remove: ok", "DELETE", "/api/v1/pool/10/chart/1004", ADMIN, null);
        ex("chart remove: absent chart", "DELETE", "/api/v1/pool/10/chart/1234", ADMIN, null);
        ex("chart remove: bad id", "DELETE", "/api/v1/pool/10/chart/abc", ADMIN, null);
        ex("chart remove: unknown pool", "DELETE", "/api/v1/pool/999/chart/1", ADMIN, null);
        ex("favorite: set", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":5}");
        ex("favorite: null", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":null}");
        ex("favorite: missing key", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{}");
        ex("favorite: no body", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "");
        ex("favorite: text", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":\"abc\"}");
        ex("favorite: numeric text", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":\"12\"}");
        ex("favorite: fraction", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":5.7}");
        ex("favorite: huge", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":1e10}");
        ex("favorite: one-element array", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":[8]}");
        ex("favorite: bool", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":true}");
        ex("favorite: object", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":{}}");
        ex("favorite: array body", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "[1]");
        ex("favorite: lenient", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{favoriteId:9}");
        ex("favorite: trailing", "PUT", "/api/v1/pool/10/favorite", JSON_ADMIN, "{\"favoriteId\":9} x");
        ex("favorite: unknown pool", "PUT", "/api/v1/pool/999/favorite", JSON_ADMIN, "{\"favoriteId\":1}");
        ex("default: on", "PUT", "/api/v1/pool/10/default", JSON_ADMIN, "{\"enabled\":true}");
        ex("default: missing key", "PUT", "/api/v1/pool/10/default", JSON_ADMIN, "{}");
        ex("default: null", "PUT", "/api/v1/pool/10/default", JSON_ADMIN, "{\"enabled\":null}");
        ex("default: text", "PUT", "/api/v1/pool/10/default", JSON_ADMIN, "{\"enabled\":\"TRUE\"}");
        ex("default: no body", "PUT", "/api/v1/pool/10/default", JSON_ADMIN, "");
        ex("default: unknown pool", "PUT", "/api/v1/pool/999/default", JSON_ADMIN, "{\"enabled\":true}");
        ex("list after edits", "GET", "/api/v1/pool/list", ADMIN, null);
        ex("remove: unknown", "DELETE", "/api/v1/pool/11", ADMIN, null);
        ex("remove: bad id", "DELETE", "/api/v1/pool/x", ADMIN, null);
        ex("remove: ok", "DELETE", "/api/v1/pool/1", ADMIN, null);
        ex("remove: last default", "DELETE", "/api/v1/pool/0", ADMIN, null);
        ex("list final", "GET", "/api/v1/pool/list", ADMIN, null);
    }

    static void submissions() throws Exception {
        ex("open pools", "GET", "/api/v1/submission/open-pools", PLAYER, null);
        ex("open pools no auth", "GET", "/api/v1/submission/open-pools", NONE, null);
        ex("submit: mixed", "POST", "/api/v1/pool/2/submissions", JSON_PLAYER, "{\"chartIds\":[1000,1001,1003,1000]}");
        ex("submit: again", "POST", "/api/v1/pool/2/submissions", JSON_PLAYER, "{\"chartIds\":[1000]}");
        ex("submit: empty", "POST", "/api/v1/pool/2/submissions", JSON_PLAYER, "{\"chartIds\":[]}");
        ex("submit: no body", "POST", "/api/v1/pool/2/submissions", JSON_PLAYER, "");
        ex("submit: too many", "POST", "/api/v1/pool/2/submissions", JSON_PLAYER,
                "{\"chartIds\":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21]}");
        ex("submit: unknown pool", "POST", "/api/v1/pool/77/submissions", JSON_PLAYER, "{\"chartIds\":[1000]}");
        ex("submit: closed pool", "POST", "/api/v1/pool/1/submissions", JSON_PLAYER, "{\"chartIds\":[1000]}");
        ex("submit: bad pool id", "POST", "/api/v1/pool/x/submissions", JSON_PLAYER, "{\"chartIds\":[1000]}");
        ex("submit: chart not indexed", "POST", "/api/v1/pool/2/submissions", JSON_PLAYER, "{\"chartIds\":[1002,424242]}");
        ex("submit: chart unknown to phira", "POST", "/api/v1/pool/2/submissions", JSON_PLAYER, "{\"chartIds\":[900005]}");
        ex("submit: as user 5", "POST", "/api/v1/pool/2/submissions", h("Authorization", "Bearer {user5}"), "{\"chartIds\":[1002]}");
        ex("mine", "GET", "/api/v1/submission/mine", PLAYER, null);
        ex("mine user 5", "GET", "/api/v1/submission/mine", h("Authorization", "Bearer {user5}"), null);
        ex("pending", "GET", "/api/v1/submission/pending", ADMIN, null);
        ex("pending as player", "GET", "/api/v1/submission/pending", PLAYER, null);
        ex("pool list", "GET", "/api/v1/pool/2/submissions", ADMIN, null);
        ex("pool list pending", "GET", "/api/v1/pool/2/submissions?status=pending", ADMIN, null);
        ex("pool list approved", "GET", "/api/v1/pool/2/submissions?status=%20APPROVED%20", ADMIN, null);
        ex("pool list bogus", "GET", "/api/v1/pool/2/submissions?status=bogus", ADMIN, null);
        ex("pool list blank", "GET", "/api/v1/pool/1/submissions?status=", ADMIN, null);
        ex("pool list as player", "GET", "/api/v1/pool/2/submissions", PLAYER, null);
        ex("withdraw: ok", "DELETE", "/api/v1/pool/2/submissions/1000", PLAYER, null);
        ex("withdraw: again", "DELETE", "/api/v1/pool/2/submissions/1000", PLAYER, null);
        ex("withdraw: approved", "DELETE", "/api/v1/pool/2/submissions/1003", h("Authorization", "Bearer {admin}"), null);
        ex("withdraw: bad id", "DELETE", "/api/v1/pool/2/submissions/x", PLAYER, null);
        ex("reject: reason", "POST", "/api/v1/submission/2/1001/reject", JSON_ADMIN, "{\"reason\":\"dup <x>\"}");
        ex("reject: no body", "POST", "/api/v1/submission/1/1005/reject", JSON_ADMIN, "");
        ex("reject: bad body", "POST", "/api/v1/submission/2/1001/reject", JSON_ADMIN, "[");
        ex("reject: unknown", "POST", "/api/v1/submission/2/9999/reject", JSON_ADMIN, "{}");
        ex("reject: as player", "POST", "/api/v1/submission/2/1001/reject", JSON_PLAYER, "{}");
        ex("approve: ok", "POST", "/api/v1/submission/2/1002/approve", ADMIN, null);
        ex("approve: unknown", "POST", "/api/v1/submission/2/9999/approve", ADMIN, null);
        ex("approve: upstream fails", "POST", "/api/v1/submission/2/900007/approve", ADMIN, null);
        ex("approve: bad id", "POST", "/api/v1/submission/abc/1/approve", ADMIN, null);
        ex("pending after", "GET", "/api/v1/submission/pending", ADMIN, null);
        ex("pool list after", "GET", "/api/v1/pool/2/submissions", ADMIN, null);
        ex("pool 2 after", "GET", "/api/v1/pool/list", ADMIN, null);
        ex("resubmit approved-failed", "POST", "/api/v1/pool/2/submissions", h("Authorization", "Bearer {user5}"), "{\"chartIds\":[900007]}");
    }

    static void charts() throws Exception {
        ex("search: default", "GET", "/api/v1/chart/search", ADMIN, null);
        ex("search: as player", "GET", "/api/v1/chart/search", PLAYER, null);
        ex("search: regular", "GET", "/api/v1/chart/search?category=REGULAR", ADMIN, null);
        ex("search: configured lower", "GET", "/api/v1/chart/search?category=configured", ADMIN, null);
        ex("search: tb", "GET", "/api/v1/chart/search?category=%20tb%20", ADMIN, null);
        ex("search: manual", "GET", "/api/v1/chart/search?category=MANUAL&limit=2", ADMIN, null);
        ex("search: bogus category", "GET", "/api/v1/chart/search?category=bogus&minRating=x", ADMIN, null);
        ex("search: tag", "GET", "/api/v1/chart/search?tag=plain", ADMIN, null);
        ex("search: tags", "GET", "/api/v1/chart/search?tag=plain,regular&minRating=0.9", ADMIN, null);
        ex("search: tag empty item", "GET", "/api/v1/chart/search?tag=,plain", ADMIN, null);
        ex("search: tag only comma", "GET", "/api/v1/chart/search?tag=,", ADMIN, null);
        ex("search: tag duplicate", "GET", "/api/v1/chart/search?tag=plain,plain", ADMIN, null);
        ex("search: tag blank", "GET", "/api/v1/chart/search?category=REGULAR&tag=%20", ADMIN, null);
        ex("search: min rating", "GET", "/api/v1/chart/search?minRating=0.95", ADMIN, null);
        ex("search: min rating count", "GET", "/api/v1/chart/search?minRatingCount=4&category=REGULAR&minRating=0", ADMIN, null);
        ex("search: min difficulty", "GET", "/api/v1/chart/search?minDifficulty=17&limit=3", ADMIN, null);
        ex("search: min duration", "GET", "/api/v1/chart/search?minDuration=300.9", ADMIN, null);
        ex("search: limit NaN", "GET", "/api/v1/chart/search?limit=NaN", ADMIN, null);
        ex("search: limit huge", "GET", "/api/v1/chart/search?limit=1e9&tag=regular", ADMIN, null);
        ex("search: bad numbers order", "GET", "/api/v1/chart/search?minRating=a&minDifficulty=b&minDuration=c", ADMIN, null);
        ex("search: bad limit", "GET", "/api/v1/chart/search?limit=a", ADMIN, null);
        ex("search: bad probe", "GET", "/api/v1/chart/search?probeDuration=a", ADMIN, null);
        ex("search: tb probe", "GET", "/api/v1/chart/search?category=TB&probeDuration=2", ADMIN, null);
        ex("search: tb probe again", "GET", "/api/v1/chart/search?category=TB&probeDuration=1.9", ADMIN, null);
        ex("duration: ok", "POST", "/api/v1/chart/duration", JSON_ADMIN, "{\"chartIds\":[1000,1003,999999]}");
        ex("duration: empty", "POST", "/api/v1/chart/duration", JSON_ADMIN, "{\"chartIds\":[]}");
        ex("duration: no body", "POST", "/api/v1/chart/duration", JSON_ADMIN, "");
        ex("duration: too many", "POST", "/api/v1/chart/duration", JSON_ADMIN,
                "{\"chartIds\":[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21]}");
        ex("generate: no body", "POST", "/api/v1/pool/generate", JSON_ADMIN, "");
        ex("generate: no category", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"sizeLimit\":3}");
        ex("generate: manual", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\"}");
        ex("generate: probe 200", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"TB\",\"sizeLimit\":15,\"roundsPerStay\":1,\"probeDuration\":200}");
        ex("generate: manual probe 200", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"MANUAL\",\"probeDuration\":200}");
        ex("generate: size zero", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"CONFIGURED\",\"sizeLimit\":0}");
        ex("generate: regular", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"REGULAR\",\"sizeLimit\":2,\"roundsPerStay\":3}");
        ex("generate: regular again", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"REGULAR\",\"sizeLimit\":2,\"roundsPerStay\":3}");
        ex("generate: tb probe 20", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"TB\",\"probeDuration\":20}");
        ex("generate: lower-case", "POST", "/api/v1/pool/generate", JSON_ADMIN, "{\"category\":\"configured\"}");
        ex("pools after generate", "GET", "/api/v1/pool/list", ADMIN, null);
        ex("search: refresh", "GET", "/api/v1/chart/search?refresh&division=plain&limit=1", ADMIN, null);
    }

    static void data() throws Exception {
        ex("export: bogus", "GET", "/api/v1/data/bogus/export", ADMIN, null);
        ex("export: as player", "GET", "/api/v1/data/admins/export", PLAYER, null);
        for (String kind : new String[]{"admins", "pools", "points", "records", "submissions"}) {
            ex("export: " + kind, "GET", "/api/v1/data/" + kind + "/export", ADMIN, null);
        }
        ex("backups: none yet", "GET", "/api/v1/data/admins/backups", ADMIN, null);
        ex("backups: bogus", "GET", "/api/v1/data/bogus/backups", ADMIN, null);
        ex("import: admins merge", "POST", "/api/v1/data/admins/import?mode=merge", JSON_ADMIN, "[5]");
        ex("import: admins replace", "POST", "/api/v1/data/admins/import?mode=replace", JSON_ADMIN, "[1,6]");
        ex("import: admins merge=true", "POST", "/api/v1/data/admins/import?merge=TRUE", JSON_ADMIN, "[7]");
        ex("import: admins default replace", "POST", "/api/v1/data/admins/import", JSON_ADMIN, "[1]");
        ex("import: blank", "POST", "/api/v1/data/admins/import?mode=merge", JSON_ADMIN, "  ");
        ex("import: empty", "POST", "/api/v1/data/admins/import?mode=merge", JSON_ADMIN, "");
        ex("import: primitive", "POST", "/api/v1/data/admins/import?mode=merge", JSON_ADMIN, "nope");
        ex("import: syntax", "POST", "/api/v1/data/admins/import?mode=merge", JSON_ADMIN, "{");
        ex("import: null", "POST", "/api/v1/data/admins/import?mode=merge", JSON_ADMIN, "null");
        ex("import: admins object", "POST", "/api/v1/data/admins/import?mode=merge", JSON_ADMIN, "{\"a\":1}");
        ex("import: pools array", "POST", "/api/v1/data/pools/import?mode=merge", JSON_ADMIN, "[]");
        ex("import: points array", "POST", "/api/v1/data/points/import?mode=merge", JSON_ADMIN, "[]");
        ex("import: submissions no field", "POST", "/api/v1/data/submissions/import?mode=merge", JSON_ADMIN, "{\"x\":[]}");
        ex("import: bogus kind", "POST", "/api/v1/data/bogus/import", JSON_ADMIN, "[]");
        ex("import: as player", "POST", "/api/v1/data/admins/import", JSON_PLAYER, "[2]");
        ex("import: points merge", "POST", "/api/v1/data/points/import?mode=merge", JSON_ADMIN, "{\"105\":{\"name\":\"Eve\",\"points\":50}}");
        ex("import: pools merge", "POST", "/api/v1/data/pools/import?mode=merge", JSON_ADMIN,
                "{\"pools\":[{\"id\":20,\"chart_ids\":[1000],\"category\":\"TB\"},{\"id\":1,\"chart_ids\":[1001,1005]}]}");
        ex("import: submissions merge", "POST", "/api/v1/data/submissions/import?mode=merge", JSON_ADMIN,
                "{\"submissions\":[{\"pool_id\":0,\"chart_id\":1008,\"status\":\"PENDING\",\"submitters\":[{\"user_id\":9,\"name\":\"n\",\"at\":\"2025-06-01T00:00:00Z\"}],\"created_at\":\"2025-06-01T00:00:00Z\"}]}");
        ex("import: records merge", "POST", "/api/v1/data/records/import?mode=merge", JSON_ADMIN,
                "[{\"id\":\"r9-1735689900000-9\",\"room_id\":\"r9\",\"chart_id\":1004,\"chart_name\":\"C1004\",\"started_at\":1735689850000,\"finished_at\":1735689900000,\"results\":[{\"player_id\":105,\"player_name\":\"Eve\",\"rank\":1,\"score\":1000000,\"accuracy\":1.0,\"std\":0.0,\"gained_points\":50,\"total_points\":50}]}]");
        ex("export after: points", "GET", "/api/v1/data/points/export", ADMIN, null);
        ex("export after: records", "GET", "/api/v1/data/records/export", ADMIN, null);
        ex("ranking after import", "GET", "/api/v1/point/ranking", ADMIN, null);
        ex("records after import", "GET", "/api/v1/record/list?limit=2", ADMIN, null);
        ex("pools after import", "GET", "/api/v1/pool/list", ADMIN, null);
        ex("backups: admins", "GET", "/api/v1/data/admins/backups", ADMIN, null);
        ex("backups: points", "GET", "/api/v1/data/points/backups", ADMIN, null);
        ex("restore: no body", "POST", "/api/v1/data/admins/restore", JSON_ADMIN, "");
        ex("restore: blank name", "POST", "/api/v1/data/admins/restore", JSON_ADMIN, "{\"backup\":\" \"}");
        ex("restore: missing", "POST", "/api/v1/data/admins/restore", JSON_ADMIN, "{\"backup\":\"admins-19990101-000000-000.json\"}");
        ex("restore: wrong kind", "POST", "/api/v1/data/points/restore", JSON_ADMIN, "{\"backup\":\"{backup:admins:0}\"}");
        ex("restore: traversal", "POST", "/api/v1/data/admins/restore", JSON_ADMIN, "{\"backup\":\"../admins.json\"}");
        ex("restore: traversal with prefix", "POST", "/api/v1/data/admins/restore", JSON_ADMIN, "{\"backup\":\"admins-/../../admins.json\"}");
        ex("restore: kind prefix bypass", "POST", "/api/v1/data/points/restore", JSON_ADMIN, "{\"backup\":\"points-/{backup:admins:0}\"}");
        ex("restore: ok", "POST", "/api/v1/data/admins/restore", JSON_ADMIN, "{\"backup\":\"{backup:admins:1}\"}");
        ex("admins after restore", "GET", "/api/v1/admin", ADMIN, null);
        ex("import: records wrong kind", "POST", "/api/v1/data/records/import?mode=replace", JSON_ADMIN, "[1,2]");
        ex("records after bad import", "GET", "/api/v1/record/list", ADMIN, null);
        ex("import: submissions null row", "POST", "/api/v1/data/submissions/import?mode=replace", JSON_ADMIN, "{\"submissions\":[null]}");
        ex("import: pools empty", "POST", "/api/v1/data/pools/import?mode=replace", JSON_ADMIN, "{\"pools\":[]}");
        ex("import: admins wrong rows", "POST", "/api/v1/data/admins/import?mode=replace", JSON_ADMIN, "[{\"id\":\"r1-1-1\",\"room_id\":\"r1\"}]");
        ex("admins after wrong rows", "GET", "/api/v1/admin", ADMIN, null);
    }

    static void admin() throws Exception {
        ex("list", "GET", "/api/v1/admin", ADMIN, null);
        ex("list as player", "GET", "/api/v1/admin", PLAYER, null);
        ex("user 5 before", "GET", "/api/v1/pool/list", h("Authorization", "Bearer {user5}"), null);
        ex("add 5", "POST", "/api/v1/admin", JSON_ADMIN, "{\"userId\":5}");
        ex("user 5 after add", "GET", "/api/v1/pool/list", h("Authorization", "Bearer {user5}"), null);
        ex("add 5 again", "POST", "/api/v1/admin", JSON_ADMIN, "{\"userId\":5}");
        ex("add zero", "POST", "/api/v1/admin", JSON_ADMIN, "{\"userId\":0}");
        ex("add negative", "POST", "/api/v1/admin", JSON_ADMIN, "{\"userId\":-3}");
        ex("add missing", "POST", "/api/v1/admin", JSON_ADMIN, "{}");
        ex("add no body", "POST", "/api/v1/admin", JSON_ADMIN, "");
        ex("add null", "POST", "/api/v1/admin", JSON_ADMIN, "{\"userId\":null}");
        ex("add text number", "POST", "/api/v1/admin", JSON_ADMIN, "{\"userId\":\"7\"}");
        ex("add as player", "POST", "/api/v1/admin", JSON_PLAYER, "{\"userId\":2}");
        ex("remove not admin", "DELETE", "/api/v1/admin/9", ADMIN, null);
        ex("remove 5", "DELETE", "/api/v1/admin/5", ADMIN, null);
        ex("user 5 after remove", "GET", "/api/v1/pool/list", h("Authorization", "Bearer {user5}"), null);
        ex("remove 7", "DELETE", "/api/v1/admin/7", ADMIN, null);
        ex("remove not admin, one left", "DELETE", "/api/v1/admin/9", ADMIN, null);
        ex("remove last", "DELETE", "/api/v1/admin/1", ADMIN, null);
        ex("remove bad id", "DELETE", "/api/v1/admin/x", ADMIN, null);
        ex("list final", "GET", "/api/v1/admin", ADMIN, null);
    }

    static void records() throws Exception {
        ex("list", "GET", "/api/v1/record/list", ADMIN, null);
        ex("list as player", "GET", "/api/v1/record/list", PLAYER, null);
        ex("list page", "GET", "/api/v1/record/list?limit=1&offset=1", ADMIN, null);
        ex("list room", "GET", "/api/v1/record/list?roomId=r1", ADMIN, null);
        ex("list player", "GET", "/api/v1/record/list?playerId=103", ADMIN, null);
        ex("list room and player", "GET", "/api/v1/record/list?roomId=r1&playerId=103", ADMIN, null);
        ex("list limit zero", "GET", "/api/v1/record/list?limit=0", ADMIN, null);
        ex("list limit huge", "GET", "/api/v1/record/list?limit=5000", ADMIN, null);
        ex("get", "GET", "/api/v1/record/r1-1735689600000-1", ADMIN, null);
        ex("get no chart name", "GET", "/api/v1/record/r2-1735689700000-2", ADMIN, null);
        ex("get missing", "GET", "/api/v1/record/nope", ADMIN, null);
        ex("get as player", "GET", "/api/v1/record/nope", PLAYER, null);
        ex("player", "GET", "/api/v1/record/player/101", ADMIN, null);
        ex("player limit", "GET", "/api/v1/record/player/101?limit=1", ADMIN, null);
        ex("player limit zero", "GET", "/api/v1/record/player/101?limit=0", ADMIN, null);
        ex("player unknown", "GET", "/api/v1/record/player/999", ADMIN, null);
        ex("player bad id", "GET", "/api/v1/record/player/x?limit=y", ADMIN, null);
        ex("player bad limit", "GET", "/api/v1/record/player/101?limit=y", ADMIN, null);
        ex("ranking", "GET", "/api/v1/point/ranking", ADMIN, null);
        ex("ranking limit", "GET", "/api/v1/point/ranking?limit=2", ADMIN, null);
        ex("ranking as player", "GET", "/api/v1/point/ranking", PLAYER, null);
        // login lives here because it needs no state
        ex("login: ok", "POST", "/api/v1/login", h("Content-Type", "application/json"), "{\"email\":\"user1@x\",\"password\":\"right\"}");
        ex("login: player", "POST", "/api/v1/login", h("Content-Type", "application/json"), "{\"email\":\"user2@x\",\"password\":\"right\"}");
        ex("login: wrong password", "POST", "/api/v1/login", h("Content-Type", "application/json"), "{\"email\":\"user1@x\",\"password\":\"wrong\"}");
        ex("login: missing password", "POST", "/api/v1/login", h("Content-Type", "application/json"), "{\"email\":\"user1@x\"}");
        ex("login: no body", "POST", "/api/v1/login", h("Content-Type", "application/json"), "");
        ex("login: bad body", "POST", "/api/v1/login", h("Content-Type", "application/json"), "[]");
        ex("login: no token from phira", "POST", "/api/v1/login", h("Content-Type", "application/json"), "{\"email\":\"notoken@x\",\"password\":\"right\"}");
        ex("login: empty answer from phira", "POST", "/api/v1/login", h("Content-Type", "application/json"), "{\"email\":\"empty@x\",\"password\":\"right\"}");
        ex("login: html answer from phira", "POST", "/api/v1/login", h("Content-Type", "application/json"), "{\"email\":\"html@x\",\"password\":\"right\"}");
        ex("login: with stale token", "POST", "/api/v1/login", h("Content-Type", "application/json", "Authorization", "Bearer junk"), "{\"email\":\"user2@x\",\"password\":\"right\"}");
    }

    /** Static hosting when frontend/dist does not exist. */
    static void staticMissing() throws Exception {
        ex("root", "GET", "/", NONE, null);
        ex("index.html", "GET", "/index.html", NONE, null);
        ex("room fallback", "GET", "/room/x", NONE, null);
        ex("pool fallback", "GET", "/pool/1", NONE, null);
        ex("other route", "GET", "/records", NONE, null);
        ex("asset with extension", "GET", "/missing.js", NONE, null);
        ex("nested asset", "GET", "/_next/static/chunks/app.js", NONE, null);
        ex("api path", "GET", "/api/x", NONE, null);
        ex("api v1 unknown", "GET", "/api/v1/nope", ADMIN, null);
        ex("traversal", "GET", "//etc/passwd", NONE, null);
        ex("HEAD root", "HEAD", "/", NONE, null);
        ex("root accept json", "GET", "/", h("Accept", "application/json"), null);
        ex("api still works", "GET", "/api/v1/admin", ADMIN, null);
    }

    /** The room routes that need no player in a room (members need a game client). */
    static void rooms() throws Exception {
        ex("list empty", "GET", "/api/v1/room/list", ADMIN, null);
        ex("list as player", "GET", "/api/v1/room/list", PLAYER, null);
        ex("list no auth", "GET", "/api/v1/room/list", NONE, null);
        ex("create default pools", "POST", "/api/v1/room/r1/create", JSON_ADMIN, "{}");
        ex("create again", "POST", "/api/v1/room/r1/create", JSON_ADMIN, "{}");
        ex("create exists before type", "POST", "/api/v1/room/r1/create", JSON_ADMIN, "{\"type\":\"remote\"}");
        ex("create bad id", "POST", "/api/v1/room/bad!/create", JSON_ADMIN, "{}");
        ex("create id too long", "POST", "/api/v1/room/abcdefghijklmnopqrstu/create", JSON_ADMIN, "{}");
        ex("create encoded id", "POST", "/api/v1/room/a%20b/create", JSON_ADMIN, "{}");
        ex("create type", "POST", "/api/v1/room/r2/create", JSON_ADMIN, "{\"type\":\"remote\"}");
        ex("create unknown pools", "POST", "/api/v1/room/r2/create", JSON_ADMIN, "{\"pools\":[77]}");
        ex("create bad body", "POST", "/api/v1/room/r2/create", JSON_ADMIN, "[");
        ex("create as player", "POST", "/api/v1/room/r2/create", JSON_PLAYER, "{}");
        ex("create pool list", "POST", "/api/v1/room/r2/create", JSON_ADMIN, "{\"type\":\"local\",\"pools\":[2,0,2]}");
        ex("create no body", "POST", "/api/v1/room/r3/create", JSON_ADMIN, "");
        ex("create null pool", "POST", "/api/v1/room/r4/create", JSON_ADMIN, "{\"pools\":[null]}");
        ex("get", "GET", "/api/v1/room/r1", ADMIN, null);
        ex("get slash", "GET", "/api/v1/room/r1/", ADMIN, null);
        ex("get as player", "GET", "/api/v1/room/r1", PLAYER, null);
        ex("get unknown", "GET", "/api/v1/room/nope", ADMIN, null);
        ex("get unknown as player", "GET", "/api/v1/room/nope", PLAYER, null);
        ex("list", "GET", "/api/v1/room/list", ADMIN, null);
        ex("update all", "PUT", "/api/v1/room/r1/update", JSON_ADMIN,
                "{\"live\":true,\"lock\":true,\"chatEnable\":false,\"minPlayer\":2,\"maxPlayer\":4,\"selectCountdown\":20,\"readyCountdown\":30,\"forceFinish\":600,\"interval\":3,\"adminIds\":[5,5,6]}");
        ex("get after update", "GET", "/api/v1/room/r1", ADMIN, null);
        ex("update partial then fail", "PUT", "/api/v1/room/r1/update", JSON_ADMIN,
                "{\"minPlayer\":7,\"maxPlayer\":3,\"chatEnable\":true,\"selectCountdown\":5}");
        ex("get after failed update", "GET", "/api/v1/room/r1", ADMIN, null);
        ex("update min above max", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "{\"minPlayer\":7,\"maxPlayer\":3}");
        ex("update admin id zero", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "{\"adminIds\":[3,0]}");
        ex("update min zero", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "{\"minPlayer\":0}");
        ex("update max zero", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "{\"maxPlayer\":0}");
        ex("update ready zero", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "{\"readyCountdown\":0}");
        ex("update force zero", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "{\"forceFinish\":0}");
        ex("update interval zero", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "{\"interval\":0}");
        ex("update empty object", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "{}");
        ex("update no body", "PUT", "/api/v1/room/r2/update", JSON_ADMIN, "");
        ex("update unknown", "PUT", "/api/v1/room/nope/update", JSON_ADMIN, "{}");
        ex("update as player", "PUT", "/api/v1/room/r2/update", JSON_PLAYER, "{}");
        ex("get r2", "GET", "/api/v1/room/r2", ADMIN, null);
        ex("pool", "GET", "/api/v1/room/r2/pool", ADMIN, null);
        ex("pool as player", "GET", "/api/v1/room/r2/pool", PLAYER, null);
        ex("pool unknown", "GET", "/api/v1/room/nope/pool", ADMIN, null);
        ex("switch not in room", "PUT", "/api/v1/room/r2/pool/switch", JSON_ADMIN, "{\"poolId\":1}");
        ex("switch", "PUT", "/api/v1/room/r2/pool/switch", JSON_ADMIN, "{\"poolId\":0}");
        ex("switch missing key", "PUT", "/api/v1/room/r1/pool/switch", JSON_ADMIN, "{}");
        ex("switch null", "PUT", "/api/v1/room/r2/pool/switch", JSON_ADMIN, "{\"poolId\":null}");
        ex("switch no body", "PUT", "/api/v1/room/r2/pool/switch", JSON_ADMIN, "");
        ex("switch unknown room", "PUT", "/api/v1/room/nope/pool/switch", JSON_ADMIN, "{\"poolId\":0}");
        ex("favorite", "PUT", "/api/v1/room/r2/pool/favorite", JSON_ADMIN, "{\"favoriteId\":99}");
        ex("favorite null", "PUT", "/api/v1/room/r1/pool/favorite", JSON_ADMIN, "{\"favoriteId\":null}");
        ex("favorite missing key", "PUT", "/api/v1/room/r2/pool/favorite", JSON_ADMIN, "{}");
        ex("favorite unknown room", "PUT", "/api/v1/room/nope/pool/favorite", JSON_ADMIN, "{\"favoriteId\":1}");
        ex("pool after", "GET", "/api/v1/room/r2/pool", ADMIN, null);
        ex("end not playing", "POST", "/api/v1/room/r1/end", ADMIN, null);
        ex("end unknown", "POST", "/api/v1/room/nope/end", ADMIN, null);
        ex("end as player", "POST", "/api/v1/room/r1/end", PLAYER, null);
        ex("say", "POST", "/api/v1/room/r1/say", JSON_ADMIN, "{\"message\":\"  hi\\r\\n\\nthere  \"}");
        ex("say long", "POST", "/api/v1/room/r1/say", JSON_ADMIN, "{\"message\":\"" + "长".repeat(210) + "\"}");
        ex("say blank", "POST", "/api/v1/room/r1/say", JSON_ADMIN, "{\"message\":\" \\n \"}");
        ex("say missing", "POST", "/api/v1/room/r1/say", JSON_ADMIN, "{}");
        ex("say no body", "POST", "/api/v1/room/r1/say", JSON_ADMIN, "");
        ex("say unknown room", "POST", "/api/v1/room/nope/say", JSON_ADMIN, "{\"message\":\"x\"}");
        ex("broadcast", "POST", "/api/v1/broadcast", JSON_ADMIN, "{\"message\":\"全体 <通知>\"}");
        ex("broadcast blank", "POST", "/api/v1/broadcast", JSON_ADMIN, "{\"message\":\"\"}");
        ex("broadcast as player", "POST", "/api/v1/broadcast", JSON_PLAYER, "{\"message\":\"x\"}");
        ex("clear", "POST", "/api/v1/room/r2/clear", ADMIN, null);
        ex("delete as player", "DELETE", "/api/v1/room/r1", PLAYER, null);
        ex("delete", "DELETE", "/api/v1/room/r1", ADMIN, null);
        ex("delete again", "DELETE", "/api/v1/room/r1", ADMIN, null);
        ex("list final", "GET", "/api/v1/room/list", ADMIN, null);
    }

    /** Port already taken: what start/awaitStarted report. Writes the exception message. */
    static void bind() throws Exception {
        try (ServerSocket taken = new ServerSocket(0, 50, java.net.InetAddress.getByName("127.0.0.1"))) {
            int port = taken.getLocalPort();
            logs.clear();
            ApiServer.start("127.0.0.1", port);
            String message;
            try {
                ApiServer.awaitStarted();
                message = "no exception";
            } catch (IllegalStateException e) {
                message = e.getClass().getName() + ": " + e.getMessage();
            }
            Thread.sleep(200);
            JsonObject entry = new JsonObject();
            entry.addProperty("name", "bind failure");
            entry.addProperty("port", port);
            entry.addProperty("awaitStarted", message.replace(String.valueOf(port), "{port}"));
            JsonArray lines = new JsonArray();
            synchronized (logs) {
                for (String line : logs) {
                    lines.add(line.replace(String.valueOf(port), "{port}"));
                }
            }
            entry.add("log", lines);
            exchanges.add(entry);
        }
    }

    // ===================================================================== exchanges

    static String[][] h(String... kv) {
        String[][] out = new String[kv.length / 2][];
        for (int i = 0; i < kv.length; i += 2) {
            out[i / 2] = new String[]{kv[i], kv[i + 1]};
        }
        return out;
    }

    static JsonObject opt(String key, Object value) {
        JsonObject o = new JsonObject();
        if (value instanceof Boolean b) o.addProperty(key, b);
        else o.addProperty(key, String.valueOf(value));
        return o;
    }

    /** A body of exactly {@code size} bytes: {@code prefix} padded with spaces. */
    static JsonObject fill(String prefix, int size, String... flags) {
        JsonObject o = new JsonObject();
        JsonObject fill = new JsonObject();
        fill.addProperty("prefix", prefix);
        fill.addProperty("size", size);
        o.add("fill", fill);
        for (String flag : flags) {
            o.addProperty(flag, true);
        }
        return o;
    }

    static String b64(byte[] bytes) {
        return Base64.getEncoder().encodeToString(bytes);
    }

    static void ex(String name, String method, String target, String[][] headers, String body) throws Exception {
        ex(name, method, target, headers, body, new JsonObject());
    }

    static void ex(String name, String method, String target, String[][] headers, String body, JsonObject opts) throws Exception {
        JsonObject req = new JsonObject();
        req.addProperty("method", method);
        req.addProperty("target", target);
        JsonArray hs = new JsonArray();
        for (String[] kv : headers) {
            JsonArray pair = new JsonArray();
            pair.add(kv[0]);
            pair.add(kv[1]);
            hs.add(pair);
        }
        req.add("headers", hs);
        if (body != null) req.addProperty("body", body);
        for (Map.Entry<String, JsonElement> e : opts.entrySet()) req.add(e.getKey(), e.getValue());

        byte[] bytes = null;
        if (body != null) bytes = resolve(body).getBytes(StandardCharsets.UTF_8);
        if (opts.has("bodyBase64")) bytes = Base64.getDecoder().decode(opts.get("bodyBase64").getAsString());
        if (opts.has("fill")) {
            JsonObject fill = opts.getAsJsonObject("fill");
            byte[] prefix = fill.get("prefix").getAsString().getBytes(StandardCharsets.UTF_8);
            bytes = new byte[fill.get("size").getAsInt()];
            java.util.Arrays.fill(bytes, (byte) ' ');
            System.arraycopy(prefix, 0, bytes, 0, prefix.length);
        }
        boolean chunked = opts.has("chunked") && opts.get("chunked").getAsBoolean();

        logs.clear();
        JsonObject resp = send(method, resolve(target), headers, bytes, chunked);
        Thread.sleep(30);
        JsonArray lines = new JsonArray();
        synchronized (logs) {
            for (String line : logs) lines.add(line);
        }
        JsonObject entry = new JsonObject();
        entry.addProperty("name", name);
        entry.add("request", req);
        entry.add("response", resp);
        entry.add("log", lines);
        exchanges.add(entry);
        System.out.println(name + " -> " + resp.get("status"));
    }

    static String resolve(String s) throws Exception {
        for (String key : new String[]{"admin", "player", "user5", "fakeadmin2"}) {
            s = s.replace("{" + key + "}", vars.get(key));
        }
        java.util.regex.Matcher m = java.util.regex.Pattern.compile("\\{backup:(\\w+):(\\d+)}").matcher(s);
        StringBuilder sb = new StringBuilder();
        while (m.find()) {
            List<String> names = DataTransferService.listBackups(DataTransferService.Kind.of(m.group(1)));
            int i = Integer.parseInt(m.group(2));
            m.appendReplacement(sb, i < names.size() ? names.get(i) : "none");
        }
        m.appendTail(sb);
        return sb.toString();
    }

    static JsonObject send(String method, String target, String[][] headers, byte[] body, boolean chunked) throws Exception {
        try (Socket socket = new Socket("127.0.0.1", apiPort)) {
            socket.setSoTimeout(60_000);
            StringBuilder head = new StringBuilder();
            head.append(method).append(' ').append(target).append(" HTTP/1.1\r\n");
            head.append("Host: 127.0.0.1:").append(apiPort).append("\r\n");
            head.append("Connection: close\r\n");
            for (String[] kv : headers) {
                head.append(kv[0]).append(": ").append(resolve(kv[1])).append("\r\n");
            }
            if (body != null) {
                if (chunked) head.append("Transfer-Encoding: chunked\r\n");
                else head.append("Content-Length: ").append(body.length).append("\r\n");
            }
            head.append("\r\n");
            OutputStream out = socket.getOutputStream();
            out.write(head.toString().getBytes(StandardCharsets.UTF_8));
            if (body != null) {
                if (chunked) {
                    int pos = 0;
                    while (pos < body.length) {
                        int n = Math.min(65536, body.length - pos);
                        out.write((Integer.toHexString(n) + "\r\n").getBytes(StandardCharsets.US_ASCII));
                        out.write(body, pos, n);
                        out.write("\r\n".getBytes(StandardCharsets.US_ASCII));
                        pos += n;
                    }
                    out.write("0\r\n\r\n".getBytes(StandardCharsets.US_ASCII));
                } else {
                    try {
                        out.write(body);
                    } catch (java.io.IOException e) {
                        // the server may answer and close before reading everything
                    }
                }
            }
            out.flush();
            ByteArrayOutputStream raw = new ByteArrayOutputStream();
            InputStream in = socket.getInputStream();
            byte[] buf = new byte[65536];
            try {
                int n;
                while ((n = in.read(buf)) != -1) raw.write(buf, 0, n);
            } catch (java.net.SocketException e) {
                // reset after the response: keep what arrived
            }
            return parse(raw.toByteArray(), method.equals("HEAD"));
        }
    }

    static JsonObject parse(byte[] raw, boolean head) throws Exception {
        JsonObject resp = new JsonObject();
        int split = indexOf(raw, "\r\n\r\n".getBytes(StandardCharsets.US_ASCII));
        if (split < 0) {
            resp.addProperty("status", 0);
            resp.addProperty("raw", new String(raw, StandardCharsets.ISO_8859_1));
            return resp;
        }
        String[] lines = new String(raw, 0, split, StandardCharsets.ISO_8859_1).split("\r\n");
        String[] status = lines[0].split(" ", 3);
        resp.addProperty("status", Integer.parseInt(status[1]));
        resp.addProperty("reason", status.length > 2 ? status[2] : "");
        JsonArray hs = new JsonArray();
        boolean chunked = false;
        boolean gzip = false;
        for (int i = 1; i < lines.length; i++) {
            int colon = lines[i].indexOf(':');
            String name = lines[i].substring(0, colon);
            String value = lines[i].substring(colon + 1).trim();
            if (name.equalsIgnoreCase("Date")) continue;
            if (name.equalsIgnoreCase("Transfer-Encoding") && value.equalsIgnoreCase("chunked")) chunked = true;
            if (name.equalsIgnoreCase("Content-Encoding") && value.equalsIgnoreCase("gzip")) gzip = true;
            JsonArray pair = new JsonArray();
            pair.add(name);
            pair.add(value);
            hs.add(pair);
        }
        resp.add("headers", hs);
        byte[] body = java.util.Arrays.copyOfRange(raw, split + 4, raw.length);
        if (!head) {
            if (chunked) body = dechunk(body);
            if (gzip) {
                resp.addProperty("gzipped", true);
                body = new GZIPInputStream(new java.io.ByteArrayInputStream(body)).readAllBytes();
            }
        }
        try {
            String text = StandardCharsets.UTF_8.newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(body)).toString();
            resp.addProperty("body", text);
        } catch (CharacterCodingException e) {
            resp.addProperty("bodyBase64", b64(body));
        }
        return resp;
    }

    static byte[] dechunk(byte[] body) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        int pos = 0;
        while (pos < body.length) {
            int eol = indexOf(java.util.Arrays.copyOfRange(body, pos, body.length), "\r\n".getBytes(StandardCharsets.US_ASCII));
            if (eol < 0) break;
            String sizeLine = new String(body, pos, eol, StandardCharsets.US_ASCII).split(";")[0].trim();
            int size = Integer.parseInt(sizeLine, 16);
            pos += eol + 2;
            if (size == 0) break;
            out.write(body, pos, size);
            pos += size + 2;
        }
        return out.toByteArray();
    }

    static int indexOf(byte[] haystack, byte[] needle) {
        outer:
        for (int i = 0; i + needle.length <= haystack.length; i++) {
            for (int j = 0; j < needle.length; j++) {
                if (haystack[i + j] != needle[j]) continue outer;
            }
            return i;
        }
        return -1;
    }

    // ===================================================================== logging

    static void installCapture() {
        LoggerContext ctx = (LoggerContext) LogManager.getContext(false);
        Configuration config = ctx.getConfiguration();
        AbstractAppender appender = new AbstractAppender("capture", null, null, true, new Property[0]) {
            @Override
            public void append(LogEvent event) {
                StringBuilder line = new StringBuilder();
                line.append(event.getLevel()).append(' ').append(event.getLoggerName()).append(": ")
                        .append(event.getMessage().getFormattedMessage());
                Throwable t = event.getThrown();
                if (t != null) {
                    line.append("\n  ").append(t.getClass().getName()).append(": ").append(t.getMessage());
                }
                logs.add(line.toString());
            }
        };
        appender.start();
        config.addAppender(appender);
        config.getRootLogger().addAppender(appender, null, null);
        ctx.updateLoggers();
    }

    // ===================================================================== Phira stub

    static String chartJson(int id) {
        JsonObject chart;
        if (indexCharts.has(String.valueOf(id))) {
            chart = indexCharts.getAsJsonObject(String.valueOf(id)).deepCopy();
            chart.remove("duration_seconds");
        } else {
            chart = JsonParser.parseString("{\"id\":" + id + ",\"name\":\"C" + id + "\",\"level\":\"AT Lv.17\","
                    + "\"difficulty\":17.5,\"charter\":\"ch" + id + "\",\"ranked\":false,\"reviewed\":true,\"stable\":true,"
                    + "\"stableRequest\":false,\"uploader\":0,\"tags\":[\"regular\"],\"rating\":0.9,\"ratingCount\":10,"
                    + "\"created\":\"2025-01-02T03:04:05.123456+08:00\",\"updated\":\"2025-03-04T05:06:07Z\","
                    + "\"chartUpdated\":\"2025-03-04T05:06:00Z\"}").getAsJsonObject();
        }
        chart.addProperty("file", "http://127.0.0.1:" + stubPort + "/files/" + id);
        return chart.toString();
    }

    static final Map<Integer, byte[]> ARCHIVES = new ConcurrentHashMap<>();

    /** A deflated ZIP holding song.mp3: one MPEG-1 Layer III 128 kbps frame header, then zeros. */
    static byte[] archive(int id) {
        return ARCHIVES.computeIfAbsent(id, k -> {
            try {
                int seconds = 300 + id % 100;
                byte[] mp3 = new byte[seconds * 16000];
                mp3[0] = (byte) 0xFF;
                mp3[1] = (byte) 0xFB;
                mp3[2] = (byte) 0x90;
                ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                try (ZipOutputStream zip = new ZipOutputStream(bytes)) {
                    zip.putNextEntry(new ZipEntry("song.mp3"));
                    zip.write(mp3);
                    zip.closeEntry();
                }
                return bytes.toByteArray();
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        });
    }

    static void reply(HttpExchange ex, int code, byte[] body) throws java.io.IOException {
        ex.sendResponseHeaders(code, body.length == 0 ? -1 : body.length);
        try (OutputStream out = ex.getResponseBody()) {
            out.write(body);
        }
    }

    static void startStub() throws Exception {
        HttpServer s = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        stubPort = s.getAddress().getPort();
        s.createContext("/", ex -> {
            String path = ex.getRequestURI().getPath();
            try {
                if (path.equals("/login")) {
                    JsonObject body = JsonParser.parseString(new String(ex.getRequestBody().readAllBytes(), StandardCharsets.UTF_8)).getAsJsonObject();
                    String email = body.get("email").getAsString();
                    String password = body.get("password").getAsString();
                    if (!password.equals("right")) {
                        reply(ex, 401, "{\"detail\":\"wrong password\"}".getBytes(StandardCharsets.UTF_8));
                    } else if (email.equals("empty@x")) {
                        reply(ex, 200, new byte[0]);
                    } else if (email.equals("html@x")) {
                        reply(ex, 200, "<html>".getBytes(StandardCharsets.UTF_8));
                    } else if (email.equals("notoken@x")) {
                        reply(ex, 200, "{\"id\":9}".getBytes(StandardCharsets.UTF_8));
                    } else {
                        int id = Integer.parseInt(email.replaceAll("\\D", ""));
                        reply(ex, 200, ("{\"id\":" + id + ",\"token\":\"phira-token-" + id
                                + "\",\"refresh_token\":\"rt\",\"expire_at\":\"2026-01-01T00:00:00Z\"}").getBytes(StandardCharsets.UTF_8));
                    }
                } else if (path.startsWith("/user/")) {
                    int id = Integer.parseInt(path.substring(6));
                    reply(ex, 200, ("{\"id\":" + id + ",\"name\":\"player" + id + "\"}").getBytes(StandardCharsets.UTF_8));
                } else if (path.equals("/chart")) {
                    reply(ex, 200, "{\"count\":0,\"results\":[]}".getBytes(StandardCharsets.UTF_8));
                } else if (path.startsWith("/chart/")) {
                    int id = Integer.parseInt(path.substring(7));
                    if (id >= 900000) {
                        reply(ex, 404, "{\"detail\":\"Not found\"}".getBytes(StandardCharsets.UTF_8));
                    } else {
                        reply(ex, 200, chartJson(id).getBytes(StandardCharsets.UTF_8));
                    }
                } else if (path.startsWith("/files/")) {
                    byte[] body = archive(Integer.parseInt(path.substring(7)));
                    String range = ex.getRequestHeaders().getFirst("Range");
                    int start = 0;
                    int end = body.length - 1;
                    if (range != null) {
                        String spec = range.substring(6);
                        if (spec.startsWith("-")) {
                            start = Math.max(0, body.length - Integer.parseInt(spec.substring(1)));
                        } else {
                            String[] p = spec.split("-");
                            start = Integer.parseInt(p[0]);
                            if (p.length > 1) end = Math.min(end, Integer.parseInt(p[1]));
                        }
                    }
                    reply(ex, 206, java.util.Arrays.copyOfRange(body, start, end + 1));
                } else {
                    reply(ex, 404, new byte[0]);
                }
            } catch (Exception e) {
                e.printStackTrace();
                reply(ex, 500, new byte[0]);
            }
        });
        s.start();
    }

    static void copyDir(Path from, Path to) throws Exception {
        try (var walk = Files.walk(from)) {
            for (Path p : walk.toList()) {
                Path target = to.resolve(from.relativize(p).toString());
                if (Files.isDirectory(p)) {
                    Files.createDirectories(target);
                } else {
                    Files.createDirectories(target.getParent());
                    Files.copy(p, target, StandardCopyOption.REPLACE_EXISTING);
                }
            }
        }
    }
}
