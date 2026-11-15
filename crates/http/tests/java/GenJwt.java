import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import io.jsonwebtoken.Jwts;
import io.jsonwebtoken.security.Keys;
import top.rymc.phira.main.http.JwtService;

import javax.crypto.Mac;
import javax.crypto.SecretKey;
import javax.crypto.spec.SecretKeySpec;
import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Base64;
import java.util.Date;
import java.util.concurrent.TimeUnit;
import java.util.zip.Deflater;

/**
 * jjwt 0.13.0 vectors for zphira-http's JWT port, from the real library and the real
 * {@code JwtService} (whose key is read from data/jwt-secret in the working directory).
 *
 * Usage (cwd = a fresh, empty scratch directory): java -cp CP GenJwt.java OUT_DIR [RUST_TOKENS]
 * Writes OUT_DIR/jwt.json:
 *   issued:   tokens built exactly like JwtService.issueToken, with a fixed clock, per key
 *   parsed:   JwtService.parseUserId(token) for crafted tokens (null = rejected)
 *   rust:     if RUST_TOKENS (one "name<TAB>token" per line, written by the Rust test
 *             jwt_write_rust_tokens) is given, what JwtService.parseUserId says about each
 */
public final class GenJwt {

    static final long EXPIRATION_MILLIS = TimeUnit.HOURS.toMillis(24);
    static final byte[] K32 = seq(32);
    static final byte[] K48 = seq(48);
    static final byte[] K64 = seq(64);
    static final byte[] WRONG = fill(32, (byte) 0xAA);
    /** Year 2100: still valid whenever the fixtures are replayed. */
    static final long FUTURE = 4_102_444_800_000L;
    static final long PAST = 1_600_000_000_000L;

    public static void main(String[] a) throws Exception {
        Path out = Path.of(a[0]).toAbsolutePath();
        Files.createDirectories(Path.of("data"));
        Files.writeString(Path.of("data", "jwt-secret"), Base64.getEncoder().encodeToString(K32));

        JsonObject doc = new JsonObject();
        doc.addProperty("secret", Base64.getEncoder().encodeToString(K32));

        JsonArray issued = new JsonArray();
        long[] clocks = {1_763_200_000_000L, 1_763_200_000_999L, FUTURE};
        int[][] subjects = {{1, 1}, {2, 0}, {Integer.MAX_VALUE, 0}, {-5, 1}, {0, 0}};
        for (byte[] key : new byte[][]{K32, K48, K64}) {
            for (long now : clocks) {
                for (int[] s : subjects) {
                    JsonObject row = new JsonObject();
                    row.addProperty("keyBase64", Base64.getEncoder().encodeToString(key));
                    row.addProperty("nowMillis", now);
                    row.addProperty("userId", s[0]);
                    row.addProperty("isAdmin", s[1] == 1);
                    row.addProperty("token", issue(Keys.hmacShaKeyFor(key), s[0], s[1] == 1, now));
                    issued.add(row);
                }
            }
        }
        doc.add("issued", issued);

        // Weak keys: what Keys.hmacShaKeyFor does with a short secret.
        JsonArray keys = new JsonArray();
        for (int len : new int[]{0, 16, 31, 32, 33, 47, 48, 63, 64, 100}) {
            JsonObject row = new JsonObject();
            row.addProperty("length", len);
            try {
                SecretKey k = Keys.hmacShaKeyFor(seq(len));
                row.addProperty("algorithm", k.getAlgorithm());
            } catch (Exception e) {
                row.addProperty("error", e.getClass().getName() + ": " + e.getMessage());
            }
            keys.add(row);
        }
        doc.add("keys", keys);

        JsonArray parsed = new JsonArray();
        SecretKey k32 = Keys.hmacShaKeyFor(K32);
        parse(parsed, "valid", issue(k32, 7, false, FUTURE - EXPIRATION_MILLIS));
        parse(parsed, "valid admin claim", issue(k32, 8, true, FUTURE - EXPIRATION_MILLIS));
        parse(parsed, "expired", issue(k32, 7, false, PAST));
        parse(parsed, "issued now by JwtService", JwtService.issueToken(9, true));
        parse(parsed, "wrong secret", issue(Keys.hmacShaKeyFor(WRONG), 7, false, FUTURE - EXPIRATION_MILLIS));
        parse(parsed, "HS384 with a 48-byte key", issue(Keys.hmacShaKeyFor(K48), 7, false, FUTURE - EXPIRATION_MILLIS));
        parse(parsed, "HS512 with a 64-byte key", issue(Keys.hmacShaKeyFor(K64), 7, false, FUTURE - EXPIRATION_MILLIS));
        String payload = "{\"sub\":\"7\",\"isAdmin\":true,\"iat\":4102358400,\"exp\":4102444800}";
        parse(parsed, "manual HS256", manual("{\"alg\":\"HS256\"}", payload, "HmacSHA256", K32));
        parse(parsed, "manual HS256 typ JWT", manual("{\"alg\":\"HS256\",\"typ\":\"JWT\"}", payload, "HmacSHA256", K32));
        parse(parsed, "manual HS256 kid", manual("{\"kid\":\"x\",\"alg\":\"HS256\"}", payload, "HmacSHA256", K32));
        parse(parsed, "manual HS384 with the 32-byte secret", manual("{\"alg\":\"HS384\"}", payload, "HmacSHA384", K32));
        parse(parsed, "manual HS512 with the 32-byte secret", manual("{\"alg\":\"HS512\"}", payload, "HmacSHA512", K32));
        parse(parsed, "manual lower-case alg", manual("{\"alg\":\"hs256\"}", payload, "HmacSHA256", K32));
        parse(parsed, "manual RS256 header with HMAC", manual("{\"alg\":\"RS256\"}", payload, "HmacSHA256", K32));
        parse(parsed, "manual no alg", manual("{\"typ\":\"JWT\"}", payload, "HmacSHA256", K32));
        parse(parsed, "alg none, no signature", b64u("{\"alg\":\"none\"}") + "." + b64u(payload) + ".");
        parse(parsed, "alg none, two segments", b64u("{\"alg\":\"none\"}") + "." + b64u(payload));
        parse(parsed, "alg none with signature", b64u("{\"alg\":\"none\"}") + "." + b64u(payload) + "."
                + sigOf("{\"alg\":\"none\"}", payload, "HmacSHA256", K32));
        parse(parsed, "alg None", b64u("{\"alg\":\"None\"}") + "." + b64u(payload) + ".");
        String good = manual("{\"alg\":\"HS256\"}", payload, "HmacSHA256", K32);
        String[] parts = good.split("\\.");
        parse(parsed, "tampered payload", parts[0] + "." + b64u(payload.replace("\"7\"", "\"1\"")) + "." + parts[2]);
        parse(parsed, "tampered signature", parts[0] + "." + parts[1] + "." + flip(parts[2]));
        parse(parsed, "signature padded", good + "=");
        parse(parsed, "signature truncated", parts[0] + "." + parts[1] + "." + parts[2].substring(0, parts[2].length() - 1));
        parse(parsed, "signature empty", parts[0] + "." + parts[1] + ".");
        parse(parsed, "header padded", b64u("{\"alg\":\"HS256\"}") + "=." + parts[1] + "." + parts[2]);
        parse(parsed, "four segments", good + ".x");
        parse(parsed, "five segments", good + ".x.y");
        parse(parsed, "empty", "");
        parse(parsed, "dots", "..");
        parse(parsed, "whitespace inside", parts[0] + ". " + parts[1] + "." + parts[2]);
        parse(parsed, "leading space", " " + good);
        parse(parsed, "trailing space", good + " ");
        parse(parsed, "trailing newline", good + "\n");
        parse(parsed, "standard base64 alphabet", good.replace('-', '+').replace('_', '/'));
        for (String[] c : new String[][]{
                {"sub text", "{\"sub\":\"abc\",\"exp\":4102444800}"},
                {"sub leading zeros", "{\"sub\":\"007\",\"exp\":4102444800}"},
                {"sub plus", "{\"sub\":\"+5\",\"exp\":4102444800}"},
                {"sub space", "{\"sub\":\" 5\",\"exp\":4102444800}"},
                {"sub overflow", "{\"sub\":\"2147483648\",\"exp\":4102444800}"},
                {"sub number", "{\"sub\":5,\"exp\":4102444800}"},
                {"sub missing", "{\"exp\":4102444800}"},
                {"sub null", "{\"sub\":null,\"exp\":4102444800}"},
                {"sub unicode digits", "{\"sub\":\"१०\",\"exp\":4102444800}"},
                {"no exp", "{\"sub\":\"5\"}"},
                {"no iat", "{\"sub\":\"5\",\"exp\":4102444800}"},
                {"exp string", "{\"sub\":\"5\",\"exp\":\"4102444800\"}"},
                {"exp fraction", "{\"sub\":\"5\",\"exp\":4102444800.5}"},
                {"exp past fraction", "{\"sub\":\"5\",\"exp\":1600000000.5}"},
                {"exp huge", "{\"sub\":\"5\",\"exp\":1e20}"},
                {"exp negative", "{\"sub\":\"5\",\"exp\":-1}"},
                {"exp zero", "{\"sub\":\"5\",\"exp\":0}"},
                {"exp bool", "{\"sub\":\"5\",\"exp\":true}"},
                {"exp iso string", "{\"sub\":\"5\",\"exp\":\"2100-01-01T00:00:00Z\"}"},
                {"exp past", "{\"sub\":\"5\",\"exp\":1600000000}"},
                {"exp null", "{\"sub\":\"5\",\"exp\":null}"},
                {"nbf future", "{\"sub\":\"5\",\"nbf\":4102444800,\"exp\":4102444900}"},
                {"nbf past", "{\"sub\":\"5\",\"nbf\":1600000000,\"exp\":4102444800}"},
                {"nbf string", "{\"sub\":\"5\",\"nbf\":\"x\",\"exp\":4102444800}"},
                {"iat future", "{\"sub\":\"5\",\"iat\":4102444800,\"exp\":4102444900}"},
                {"iat string", "{\"sub\":\"5\",\"iat\":\"x\",\"exp\":4102444800}"},
                {"extra claims", "{\"sub\":\"5\",\"exp\":4102444800,\"role\":[1,{}],\"isAdmin\":\"yes\"}"},
                {"duplicate sub", "{\"sub\":\"5\",\"sub\":\"6\",\"exp\":4102444800}"},
                {"payload array", "[1]"},
                {"payload string", "\"x\""},
                {"payload not json", "not json"},
                {"payload empty object", "{}"},
                {"payload lenient json", "{sub:'5',exp:4102444800}"},
                {"aud claim", "{\"sub\":\"5\",\"aud\":\"x\",\"exp\":4102444800}"},
                {"iss claim", "{\"sub\":\"5\",\"iss\":\"x\",\"exp\":4102444800}"},
        }) {
            parse(parsed, c[0], manual("{\"alg\":\"HS256\"}", c[1], "HmacSHA256", K32));
        }
        parse(parsed, "header not json", manual("nope", payload, "HmacSHA256", K32));
        parse(parsed, "header array", manual("[]", payload, "HmacSHA256", K32));
        parse(parsed, "crit unknown", manual("{\"alg\":\"HS256\",\"crit\":[\"foo\"],\"foo\":1}", payload, "HmacSHA256", K32));
        parse(parsed, "b64 false", manual("{\"alg\":\"HS256\",\"b64\":false,\"crit\":[\"b64\"]}", payload, "HmacSHA256", K32));
        parse(parsed, "zip DEF", manualRaw(b64u("{\"alg\":\"HS256\",\"zip\":\"DEF\"}"), b64u(deflate(payload)), "HmacSHA256", K32));
        parse(parsed, "zip unknown", manual("{\"alg\":\"HS256\",\"zip\":\"XYZ\"}", payload, "HmacSHA256", K32));
        parse(parsed, "enc header", manual("{\"alg\":\"HS256\",\"enc\":\"A128GCM\"}", payload, "HmacSHA256", K32));
        parse(parsed, "cty header", manual("{\"alg\":\"HS256\",\"cty\":\"JWT\"}", payload, "HmacSHA256", K32));
        parse(parsed, "typ other", manual("{\"alg\":\"HS256\",\"typ\":\"x\"}", payload, "HmacSHA256", K32));
        doc.add("parsed", parsed);

        if (a.length > 1) {
            JsonArray rust = new JsonArray();
            for (String line : Files.readAllLines(Path.of(a[1]))) {
                if (line.isBlank()) continue;
                String[] kv = line.split("\t", 2);
                JsonObject row = new JsonObject();
                row.addProperty("name", kv[0]);
                row.addProperty("token", kv[1]);
                Integer id = JwtService.parseUserId(kv[1]);
                if (id == null) row.add("userId", com.google.gson.JsonNull.INSTANCE);
                else row.addProperty("userId", id);
                rust.add(row);
            }
            doc.add("rust", rust);
        }

        Gson gson = new GsonBuilder().setPrettyPrinting().disableHtmlEscaping().serializeNulls().create();
        Files.createDirectories(out);
        Files.writeString(out.resolve("jwt.json"), gson.toJson(doc) + "\n");
        System.exit(0);
    }

    /** JwtService.issueToken with an explicit clock and key. */
    static String issue(SecretKey key, int userId, boolean isAdmin, long now) {
        return Jwts.builder()
                .subject(String.valueOf(userId))
                .claim("isAdmin", isAdmin)
                .issuedAt(new Date(now))
                .expiration(new Date(now + EXPIRATION_MILLIS))
                .signWith(key)
                .compact();
    }

    static void parse(JsonArray out, String name, String token) {
        JsonObject row = new JsonObject();
        row.addProperty("name", name);
        row.addProperty("token", token);
        Integer id = JwtService.parseUserId(token);
        if (id == null) row.add("userId", com.google.gson.JsonNull.INSTANCE);
        else row.addProperty("userId", id);
        // the reason jjwt gave, for the record
        try {
            Jwts.parser().verifyWith(Keys.hmacShaKeyFor(K32)).build().parseSignedClaims(token);
            row.addProperty("jjwt", "ok");
        } catch (Exception e) {
            row.addProperty("jjwt", e.getClass().getSimpleName());
        }
        out.add(row);
    }

    static String manual(String header, String payload, String mac, byte[] key) throws Exception {
        return manualRaw(b64u(header), b64u(payload), mac, key);
    }

    static String manualRaw(String header, String payload, String mac, byte[] key) throws Exception {
        String signingInput = header + "." + payload;
        Mac m = Mac.getInstance(mac);
        m.init(new SecretKeySpec(key, mac));
        return signingInput + "." + Base64.getUrlEncoder().withoutPadding().encodeToString(m.doFinal(signingInput.getBytes(StandardCharsets.US_ASCII)));
    }

    static String sigOf(String header, String payload, String mac, byte[] key) throws Exception {
        return manual(header, payload, mac, key).split("\\.")[2];
    }

    static String b64u(String s) {
        return b64u(s.getBytes(StandardCharsets.UTF_8));
    }

    static String b64u(byte[] b) {
        return Base64.getUrlEncoder().withoutPadding().encodeToString(b);
    }

    static byte[] deflate(String s) {
        Deflater d = new Deflater();
        d.setInput(s.getBytes(StandardCharsets.UTF_8));
        d.finish();
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[256];
        while (!d.finished()) {
            int n = d.deflate(buf);
            out.write(buf, 0, n);
        }
        return out.toByteArray();
    }

    static String flip(String s) {
        char c = s.charAt(5);
        return s.substring(0, 5) + (c == 'A' ? 'B' : 'A') + s.substring(6);
    }

    static byte[] seq(int n) {
        byte[] b = new byte[n];
        for (int i = 0; i < n; i++) b[i] = (byte) i;
        return b;
    }

    static byte[] fill(int n, byte v) {
        byte[] b = new byte[n];
        java.util.Arrays.fill(b, v);
        return b;
    }

    // keep the parser import used
    static Object unused = JsonParser.class;
}
