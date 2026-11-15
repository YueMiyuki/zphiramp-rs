import joptsimple.OptionParser;
import joptsimple.OptionSet;
import joptsimple.OptionSpec;

import java.io.IOException;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

/**
 * Golden vectors for zphira_common::args, recorded with jopt-simple 5.0.4.
 *
 * <p>The parser is built exactly like {@code ServerArgs} (same option order, descriptions,
 * types and defaults); the field evaluation order and clamping are copied from its
 * constructor. Instead of logging and exiting, each argument vector records what happened.
 *
 * <p>Two options are not in the Java server: {@code --max-pending-submissions} and
 * {@code --login-rate-limit} (owner decisions O-01 and O-02). They are declared here, after
 * {@code empty-room-ttl}, so the real jopt-simple decides parsing, prefixes and help layout;
 * their validation ({@code validLimit}) is the owner's specification, not Java behaviour.
 *
 * <p>Run: {@code java -cp <jopt-simple-5.0.4.jar> crates/common/tests/java/GenArgs.java crates/common/tests/fixtures}
 *
 * <p>Record: {@code n, arg1..argn, outcome...} where outcome is one of
 * {@code PARSE_ERROR message}, {@code HELP}, {@code FATAL warnings exception-chain} or
 * {@code OK port host http_host http_port plugins proxy language read_timeout max_connections
 * max_rooms empty_room_ttl max_pending_submissions login_rate_limit warnings} ({@code none}
 * for no limit); lists are joined with {@code " | "}.
 */
public class GenArgs {

    static OptionParser parser;
    static OptionSpec<Integer> portSpec, httpPortSpec, readTimeoutSpec, maxConnectionsSpec, maxRoomsSpec, emptyRoomTtlSpec,
            maxPendingSpec, loginRateSpec;
    static OptionSpec<String> hostSpec, pluginsSpec, languageSpec, httpHostSpec;
    static OptionSpec<Boolean> proxyProtocol;

    static void build() {
        parser = new OptionParser();
        portSpec = parser.accepts("port", "Server listening port").withRequiredArg().ofType(Integer.class).defaultsTo(12346);
        hostSpec = parser.accepts("host", "Bind address (IP to listen on)").withRequiredArg().ofType(String.class).defaultsTo("0.0.0.0");
        pluginsSpec = parser.accepts("plugins", "Plugins directory path").withRequiredArg().ofType(String.class).defaultsTo("plugins");
        proxyProtocol = parser.accepts("proxy-protocol", "Enable proxy protocol support").withOptionalArg().ofType(Boolean.class).defaultsTo(false);
        languageSpec = parser.accepts("language", "Default server language (e.g., zh-CN, en-US)").withRequiredArg().ofType(String.class).defaultsTo("zh-CN");
        httpHostSpec = parser.accepts("http-host", "HTTP API bind address").withRequiredArg().ofType(String.class).defaultsTo("0.0.0.0");
        httpPortSpec = parser.accepts("http-port", "HTTP API listening port").withRequiredArg().ofType(Integer.class).defaultsTo(8080);
        readTimeoutSpec = parser.accepts("read-timeout", "Seconds before an idle client is dropped").withRequiredArg().ofType(Integer.class).defaultsTo(3600);
        maxConnectionsSpec = parser.accepts("max-connections", "Maximum simultaneous connections").withRequiredArg().ofType(Integer.class).defaultsTo(1000);
        maxRoomsSpec = parser.accepts("max-rooms", "Maximum rooms alive at once").withRequiredArg().ofType(Integer.class).defaultsTo(200);
        emptyRoomTtlSpec = parser.accepts("empty-room-ttl",
                        "Minutes a room may stay empty before being reclaimed (0 disables reclaiming)")
                .withRequiredArg().ofType(Integer.class).defaultsTo(60);
        maxPendingSpec = parser.accepts("max-pending-submissions",
                        "Pending chart submissions allowed per account (-1 for no limit)")
                .withRequiredArg().ofType(Integer.class).defaultsTo(50);
        loginRateSpec = parser.accepts("login-rate-limit",
                        "HTTP login attempts allowed per minute per client address (-1 for no limit)")
                .withRequiredArg().ofType(Integer.class).defaultsTo(30);
        parser.accepts("help", "Show this help message").forHelp();
    }

    static final List<String> warnings = new ArrayList<>();

    static int clampPositive(int value, int minimum) {
        if (value < minimum) {
            warnings.add("Value " + value + " is below the minimum " + minimum + ", using " + minimum);
            return minimum;
        }
        return value;
    }

    /** O-01/O-02: -1 = no limit (null), >= 1 as given, anything else the default with a warning. */
    static Integer validLimit(int value, String flag, int defaultValue) {
        if (value == -1) {
            return null;
        }
        if (value >= 1) {
            return value;
        }
        warnings.add("Value " + value + " is not valid for " + flag + ", using " + defaultValue);
        return defaultValue;
    }

    static int clampPort(int value) {
        return Math.max(1, Math.min(value, 65535));
    }

    static String run(String[] args) {
        build();
        warnings.clear();
        OptionSet options;
        try {
            options = parser.parse(args);
        } catch (Exception e) {
            return "PARSE_ERROR\t" + Esc.esc(e.getMessage());
        }
        if (options.has("help")) {
            return "HELP";
        }
        try {
            int port = clampPort(options.valueOf(portSpec));
            String host = options.valueOf(hostSpec);
            String httpHost = options.valueOf(httpHostSpec);
            int httpPort = clampPort(options.valueOf(httpPortSpec));
            Path plugins = Paths.get(options.valueOf(pluginsSpec));
            boolean proxy = options.valueOf(proxyProtocol);
            String language = options.valueOf(languageSpec);
            int readTimeout = clampPositive(options.valueOf(readTimeoutSpec), 5);
            int maxConnections = clampPositive(options.valueOf(maxConnectionsSpec), 1);
            int maxRooms = clampPositive(options.valueOf(maxRoomsSpec), 1);
            int emptyRoomTtl = Math.max(0, options.valueOf(emptyRoomTtlSpec));
            Integer maxPending = validLimit(options.valueOf(maxPendingSpec), "--max-pending-submissions", 50);
            Integer loginRate = validLimit(options.valueOf(loginRateSpec), "--login-rate-limit", 30);
            return String.join("\t", "OK", Integer.toString(port), Esc.esc(host), Esc.esc(httpHost),
                    Integer.toString(httpPort), Esc.esc(plugins.toString()), Boolean.toString(proxy), Esc.esc(language),
                    Integer.toString(readTimeout), Integer.toString(maxConnections), Integer.toString(maxRooms),
                    Integer.toString(emptyRoomTtl), maxPending == null ? "none" : maxPending.toString(),
                    loginRate == null ? "none" : loginRate.toString(), Esc.esc(String.join(" | ", warnings)));
        } catch (RuntimeException e) {
            List<String> chain = new ArrayList<>();
            for (Throwable t = e; t != null; t = t.getCause()) {
                chain.add(t.toString());
            }
            return "FATAL\t" + Esc.esc(String.join(" | ", warnings)) + "\t" + Esc.esc(String.join(" | ", chain));
        }
    }

    static final String[][] CASES = {
            {},
            {"--help"}, {"-help"}, {"--he"}, {"--help=foo"}, {"--help", "--port", "abc"}, {"--help", "--bogus"},
            {"-h"}, {"-?"},
            {"--port", "1234"}, {"--port=1234"}, {"-port", "1234"}, {"-port=1234"}, {"--po", "1234"}, {"--p", "1"},
            {"--port"}, {"--port="}, {"--port", "abc"}, {"--port", ""}, {"--port", " 5"}, {"--port", "+5"},
            {"--port", "-5"}, {"--port", "0"}, {"--port", "70000"}, {"--port", "99999999999"}, {"--port", "٥٥"},
            {"--port", "1", "--port", "2"}, {"--port", "1", "--port", "abc"}, {"--port", "abc", "--port", "1"},
            {"--port", "--host"}, {"--host", "--port"}, {"--port", "5", "extra"}, {"extra", "--port", "5"},
            {"--port==5"}, {"--=5"}, {"--"}, {"--", "--port", "5"}, {"-"}, {"-", "--port", "5"},
            {"--bogus"}, {"--bogus=1"}, {"-x"}, {"-=x"}, {"--PORT", "1"},
            {"--proxy-protocol"}, {"--proxy-protocol", "true"}, {"--proxy-protocol", "TRUE"}, {"--proxy-protocol=true"},
            {"--proxy-protocol=True"}, {"--proxy-protocol", "false"}, {"--proxy-protocol", "yes"}, {"--proxy-protocol", "1"},
            {"--proxy-protocol="}, {"--proxy-protocol", "--port", "5"}, {"--proxy-protocol", "-"}, {"--proxy-protocol", "--"},
            {"--proxy-protocol", "-x"}, {"--proxy-protocol", "--proxy-protocol"}, {"--proxy-protocol", "true", "--proxy-protocol"},
            {"--proxy-protocol", "--proxy-protocol", "true"}, {"--proxy-protocol", "true", "--proxy-protocol", "false"},
            {"--pr"}, {"--pr", "true"}, {"-pr"}, {"--proxy"}, {"--proxy-protocol", "extra"},
            {"--host", "127.0.0.1"}, {"--ho", "::1"}, {"--h", "x"}, {"--http-host", "1.2.3.4"}, {"--http-", "x"},
            {"--http-port", "9090"}, {"--http-port", "0"}, {"--http-port", "x"}, {"--ht", "1"},
            {"--plugins", "p2"}, {"--plugins", ""}, {"--pl", "dir/sub"}, {"--plugins", "a", "--plugins", "b"},
            {"--language", "en-US"}, {"--language="}, {"-l", "ja-JP"}, {"-lja-JP"}, {"--l", "x"},
            {"--read-timeout", "4"}, {"--read-timeout", "5"}, {"--read-timeout", "0"}, {"--read-timeout", "-1"},
            {"-r", "4"}, {"-r10"}, {"--read-timeout", "4", "--max-rooms", "abc"},
            {"--max-connections", "0"}, {"--max-connections", "1"}, {"--max-rooms", "0"}, {"--max-rooms", "-5"},
            {"--max-r", "3"}, {"--max-c", "3"}, {"--max", "3"}, {"--m", "3"}, {"--max-rooms"},
            {"--empty-room-ttl", "0"}, {"--empty-room-ttl", "-5"}, {"--empty-room-ttl", "15"}, {"-e5"}, {"-e", "5"},
            {"-ee"}, {"--empty-room-ttl", "1.5"},
            {"--read-timeout", "1", "--max-connections", "0", "--max-rooms", "0", "--empty-room-ttl", "-1"},
            {"--read-timeout", "1", "--max-connections", "x"},
            {"--[arguments]"}, {"--["}, {"-["}, {"-[x"}, {"--[arguments]=x"},
            {"--max-pending-submissions", "-1"}, {"--max-pending-submissions", "1"}, {"--max-pending-submissions", "50"},
            {"--max-pending-submissions", "1000"}, {"--max-pending-submissions", "0"}, {"--max-pending-submissions", "-2"},
            {"--max-pending-submissions", "-2147483648"}, {"--max-pending-submissions", "abc"},
            {"--max-pending-submissions"}, {"--max-pending-submissions=-1"}, {"--max-pending-submissions="},
            {"--max-p", "7"}, {"--max-pe", "7"}, {"-max-p", "7"}, {"--max-pending-submissions", "1", "--max-pending-submissions", "2"},
            {"--login-rate-limit", "-1"}, {"--login-rate-limit", "1"}, {"--login-rate-limit", "30"},
            {"--login-rate-limit", "120"}, {"--login-rate-limit", "0"}, {"--login-rate-limit", "-5"},
            {"--login-rate-limit", "x"}, {"--login-rate-limit"}, {"--login-rate-limit=-1"}, {"--lo", "9"},
            {"--log", "9"}, {"--la", "en-US"}, {"--lang", "en-US"}, {"--l", "x"}, {"-l", "x"}, {"-lo", "9"},
            {"--max-c", "3"}, {"--max-r", "3"}, {"--ma", "3"},
            {"--max-pending-submissions", "0", "--login-rate-limit", "0"},
            {"--read-timeout", "1", "--max-pending-submissions", "0", "--login-rate-limit", "abc"},
            {"--max-pending-submissions", "abc", "--read-timeout", "1"},
            {"--port", "1", "--http-port", "2", "--host", "h", "--http-host", "hh", "--plugins", "pp", "--language", "fr",
                    "--read-timeout", "60", "--max-connections", "10", "--max-rooms", "3", "--empty-room-ttl", "7",
                    "--proxy-protocol", "true", "--max-pending-submissions", "10", "--login-rate-limit", "60"},
    };

    public static void main(String[] a) throws IOException {
        Path out = Path.of(a[0]);
        Files.createDirectories(out);
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("args.tsv"), StandardCharsets.UTF_8))) {
            for (String[] args : CASES) {
                StringBuilder line = new StringBuilder().append(args.length);
                for (String arg : args) {
                    line.append('\t').append(Esc.esc(arg));
                }
                line.append('\t').append(run(args));
                w.println(line);
            }
        }
        build();
        StringWriter help = new StringWriter();
        parser.printHelpOn(help);
        Files.writeString(out.resolve("args_help.txt"), help.toString(), StandardCharsets.UTF_8);
    }
}
