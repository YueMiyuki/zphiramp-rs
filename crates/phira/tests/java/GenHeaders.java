package top.rymc.phira.main.util;

import com.sun.net.httpserver.HttpServer;

import java.io.PrintWriter;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * The request headers the real Java client sends (N-32), recorded by running the real
 * {@code PhiraFetcher} and {@code ChartDuration.probe} against a local server that dumps what
 * arrives. Writes {@code request_headers.tsv}: label, method, target, then the headers as
 * {@code name: value} lines (names lower-cased, sorted), escaped by {@code Esc}.
 *
 * <p>Over plain http the JDK client also sends its cleartext HTTP/2 upgrade trio ({@code connection},
 * {@code upgrade}, {@code http2-settings}), {@code host} and, for a body, {@code content-length}; over
 * https it negotiates h2 through ALPN and sends none of the trio. The Rust test compares everything
 * else.
 */
public final class GenHeaders {

    static final List<String[]> rows = new ArrayList<>();

    public static void main(String[] args) throws Exception {
        Thread watchdog = new Thread(() -> {
            try {
                Thread.sleep(120_000);
            } catch (InterruptedException e) {
                return;
            }
            System.err.println("GenHeaders: watchdog fired");
            Runtime.getRuntime().halt(3);
        }, "watchdog");
        watchdog.setDaemon(true);
        watchdog.start();

        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        int port = server.getAddress().getPort();
        String base = "http://127.0.0.1:" + port + "/";
        server.createContext("/", exchange -> {
            Map<String, String> headers = new TreeMap<>();
            exchange.getRequestHeaders().forEach((k, v) -> headers.put(k.toLowerCase(), String.join(", ", v)));
            StringBuilder b = new StringBuilder();
            headers.forEach((k, v) -> b.append(k).append(": ").append(v).append('\n'));
            rows.add(new String[]{exchange.getRequestMethod(), exchange.getRequestURI().toString(), b.toString()});
            String path = exchange.getRequestURI().getPath();
            byte[] body;
            int status = 200;
            if (path.startsWith("/files/")) {
                body = new byte[64];
            } else if (path.equals("/record")) {
                body = "[]".getBytes(StandardCharsets.UTF_8);
            } else if (path.equals("/chart")) {
                body = "{\"count\":0,\"results\":[]}".getBytes(StandardCharsets.UTF_8);
            } else {
                body = "{\"id\":1,\"token\":\"t\"}".getBytes(StandardCharsets.UTF_8);
            }
            if (exchange.getRequestHeaders().getFirst("Range") != null) {
                status = 206;
            }
            exchange.sendResponseHeaders(status, body.length);
            exchange.getResponseBody().write(body);
            exchange.close();
        });
        server.start();
        PhiraFetcher.setHost(base);

        try {
            run(base);
        } finally {
            Path out = Path.of(args[0]).resolve("request_headers.tsv");
            try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out, StandardCharsets.UTF_8))) {
                for (String[] r : rows) {
                    w.println(Esc.esc(r[0]) + "\t" + Esc.esc(r[1]) + "\t" + Esc.esc(r[2]));
                }
            }
            server.stop(0);
            System.exit(0);
        }
    }

    static void run(String base) throws Exception {
        PhiraFetcher.GET_USER_INFO.apply("secret-token");
        PhiraFetcher.GET_USER_INFO_BY_ID.apply(77);
        PhiraFetcher.GET_CHART_INFO.apply(42);
        PhiraFetcher.GET_RECORD_INFO.apply(9);
        PhiraFetcher.GET_RECENT_RECORDS.apply(7);
        PhiraFetcher.POST_LOGIN.apply("player@example.com", "pw");
        PhiraFetcher.fetchChartPage(3, null);
        PhiraFetcher.fetchChartPage(4, "plain");
        ChartDuration.probe(base + "files/archive");
    }
}
