import com.google.gson.FieldNamingPolicy;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonDeserializer;
import com.google.gson.JsonElement;
import com.google.gson.JsonParser;
import com.google.gson.JsonPrimitive;
import com.google.gson.JsonSerializer;
import com.google.gson.Strictness;
import com.google.gson.annotations.SerializedName;
import com.google.gson.reflect.TypeToken;

import java.io.IOException;
import java.io.PrintWriter;
import java.lang.reflect.Type;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.OffsetDateTime;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * Golden vectors for zphira_common::gson, recorded with Gson 2.12.1 configured exactly like
 * GsonUtil (pretty and compact instances).
 *
 * <p>Run: {@code java -cp <gson-2.12.1.jar> crates/common/tests/java/GenGson.java crates/common/tests/fixtures}
 */
public class GenGson {

    static final Gson PRETTY = new GsonBuilder()
            .registerTypeAdapter(OffsetDateTime.class,
                    (JsonSerializer<OffsetDateTime>) (src, type, context) -> new JsonPrimitive(src.toString()))
            .registerTypeAdapter(OffsetDateTime.class,
                    (JsonDeserializer<OffsetDateTime>) (json, type, context) -> OffsetDateTime.parse(json.getAsString()))
            .setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES)
            .setStrictness(Strictness.LENIENT)
            .setPrettyPrinting()
            .create();

    static final Gson COMPACT = new GsonBuilder()
            .registerTypeAdapter(OffsetDateTime.class,
                    (JsonSerializer<OffsetDateTime>) (src, type, context) -> new JsonPrimitive(src.toString()))
            .registerTypeAdapter(OffsetDateTime.class,
                    (JsonDeserializer<OffsetDateTime>) (json, type, context) -> OffsetDateTime.parse(json.getAsString()))
            .setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES)
            .setStrictness(Strictness.LENIENT)
            .serializeNulls()
            .create();

    // ------------------------------------------------------------------ models (mirrored in tests/gson.rs)

    enum Kind { REGULAR, FINAL }

    static class Inner {
        int id;
        String label;
        Inner(int id, String label) { this.id = id; this.label = label; }
    }

    static class Everything {
        int intValue;
        long longValue;
        float floatValue;
        double doubleValue;
        boolean flag;
        char letter;
        String text;
        String nullText;
        Integer boxedNull;
        Integer boxed;
        List<Integer> numbers;
        List<Integer> emptyList;
        List<String> withNulls;
        Map<String, Integer> map;
        Map<String, Integer> emptyMap;
        Map<Integer, String> intKeys;
        Inner inner;
        Inner nullInner;
        List<Inner> inners;
        Kind kind;
        Kind nullKind;
        OffsetDateTime at;
        @SerializedName("ratingCount") int ratingCount;
        int userID;
        int chart2Id;
        List<List<Integer>> nested;
        Map<String, List<Integer>> mapOfLists;
    }

    static Everything everything() {
        Everything e = new Everything();
        e.intValue = -42;
        e.longValue = 9007199254740993L;
        e.floatValue = 0.9876f * 100;
        e.doubleValue = 1234.0 / 1000.0;
        e.flag = true;
        e.letter = '<';
        e.text = "a<b>&c='d' \"q\" \\ \n\t\b\f\r \u0001\u001f\u007f \u2028\u2029 中文 😀 /";
        e.nullText = null;
        e.boxedNull = null;
        e.boxed = 7;
        e.numbers = List.of(1, 2, 3);
        e.emptyList = List.of();
        e.withNulls = Arrays.asList("x", null, "y");
        e.map = new LinkedHashMap<>();
        e.map.put("one", 1);
        e.map.put("gone", null);
        e.map.put("<three>", 3);
        e.emptyMap = new LinkedHashMap<>();
        e.intKeys = new LinkedHashMap<>();
        e.intKeys.put(10, "ten");
        e.intKeys.put(-1, null);
        e.inner = new Inner(1, "first");
        e.nullInner = null;
        e.inners = List.of(new Inner(2, null), new Inner(3, "third"));
        e.kind = Kind.FINAL;
        e.nullKind = null;
        e.at = OffsetDateTime.parse("2026-11-14T15:26:00.120+08:00");
        e.ratingCount = 5;
        e.userID = 6;
        e.chart2Id = 8;
        e.nested = List.of(List.of(), List.of(1), List.of(2, 3));
        e.mapOfLists = new LinkedHashMap<>();
        e.mapOfLists.put("a", List.of());
        e.mapOfLists.put("b", List.of(4, 5));
        return e;
    }

    static class Floats {
        float a;
        float b;
        float c;
        float d;
        double e;
        double f;
        double g;
        double h;
        Floats(float a, float b, float c, float d, double e, double f, double g, double h) {
            this.a = a; this.b = b; this.c = c; this.d = d; this.e = e; this.f = f; this.g = g; this.h = h;
        }
    }

    static class AllNull {
        String a;
        Integer b;
        Inner c;
        List<Integer> d;
    }

    static class NamesHolder {
        int userID;
        int chart2Id;
        int URL;
        int aB;
        int _x;
        int x_Y;
        int someLongFieldName;
        int already_snake;
        int ÀccentÉ;
        int a1B2;
    }

    // Coercion targets; initial values let "kept default" be told apart from zero.
    static class IntBox { int v = 42; }
    static class LongBox { long v = 42; }
    static class ShortBox { short v = 42; }
    static class ByteBox { byte v = 42; }
    static class FloatBox { float v = 42; }
    static class DoubleBox { double v = 42; }
    static class BoolBox { boolean v = true; }
    static class CharBox { char v = 'z'; }
    static class StringBox { String v = "init"; }
    static class IntegerBox { Integer v = 42; }
    static class KindBox { Kind v = Kind.REGULAR; }
    static class TimeBox { OffsetDateTime v; }
    static class ListBox { List<Integer> v = new ArrayList<>(List.of(9)); }
    static class InnerBox { Inner v = new Inner(9, "init"); }

    record Rec(int a, String b, double c) {}

    static class Pair {
        int a;
        String b;
        Inner c;
    }

    public static void main(String[] args) throws IOException {
        Path out = Path.of(args[0]);
        Files.createDirectories(out);
        serialize(out.resolve("gson_serialize.tsv"));
        coerce(out.resolve("gson_coerce.tsv"));
        parse(out.resolve("gson_parse.tsv"));
        documents(out.resolve("gson_document.tsv"));
        maps(out.resolve("gson_map.tsv"));
        names(out.resolve("gson_names.tsv"));
    }

    static PrintWriter open(Path p) throws IOException {
        return new PrintWriter(Files.newBufferedWriter(p, StandardCharsets.UTF_8));
    }

    static String err(Throwable e) {
        return "ERR " + e.getClass().getName() + ": " + e.getMessage();
    }

    // ------------------------------------------------------------------ serialization

    static void serialize(Path p) throws IOException {
        Map<String, Object> cases = new LinkedHashMap<>();
        cases.put("everything", everything());
        cases.put("floats", new Floats(0.1f, -0.0f, 1.0E-4f, 1.2345678E7f, 1.0E-5, 1.0E7, -0.0, Double.MIN_VALUE));
        cases.put("all_null", new AllNull());
        cases.put("top_null", null);
        cases.put("top_string", "<html> & 'x'");
        cases.put("top_int", 5);
        cases.put("top_empty_list", List.of());
        cases.put("top_list_of_nulls", Arrays.asList(null, null));
        Map<String, Object> nullMap = new LinkedHashMap<>();
        nullMap.put("a", null);
        nullMap.put("b", null);
        cases.put("map_all_null", nullMap);
        Map<String, Object> nestedMaps = new LinkedHashMap<>();
        nestedMaps.put("empty", Map.of());
        nestedMaps.put("list", List.of(Map.of(), List.of()));
        cases.put("nested_empty", nestedMaps);
        TreeMap<Double, String> doubleKeys = new TreeMap<>();
        doubleKeys.put(1.0, "one");
        doubleKeys.put(0.001, "milli");
        doubleKeys.put(1e7, "big");
        cases.put("double_keys", doubleKeys);
        Map<Boolean, Integer> boolKeys = new LinkedHashMap<>();
        boolKeys.put(true, 1);
        boolKeys.put(false, 0);
        cases.put("bool_keys", boolKeys);
        cases.put("record", new Rec(1, null, 2.5));
        cases.put("element", JsonParser.parseString(
                "{\"n\":1.50,\"e\":1e5,\"neg\":-0,\"big\":123456789012345678901234567890,\"s\":\"<\",\"nul\":null,"
                        + "\"arr\":[null,1,{\"x\":null}],\"o\":{}}"));
        try (PrintWriter w = open(p)) {
            for (Map.Entry<String, Object> c : cases.entrySet()) {
                String pretty;
                String compact;
                try {
                    pretty = PRETTY.toJson(c.getValue());
                } catch (Exception e) {
                    pretty = err(e);
                }
                try {
                    compact = COMPACT.toJson(c.getValue());
                } catch (Exception e) {
                    compact = err(e);
                }
                w.println(c.getKey() + "\t" + Esc.esc(pretty) + "\t" + Esc.esc(compact));
            }
            // errors
            for (Object bad : new Object[]{new Floats(Float.NaN, 0, 0, 0, 0, 0, 0, 0),
                    new Floats(0, 0, 0, 0, Double.NEGATIVE_INFINITY, 0, 0, 0)}) {
                String r;
                try {
                    r = PRETTY.toJson(bad);
                } catch (Exception e) {
                    r = err(e);
                }
                w.println("error_" + (((Floats) bad).a != ((Floats) bad).a ? "nan" : "neg_inf") + "\t" + Esc.esc(r) + "\t" + Esc.esc(r));
            }
        }
    }

    // ------------------------------------------------------------------ coercion matrix

    static final String[] VALUES = {
            "1", "-1", "0", "1.0", "1.5", "-1.5", "1e2", "1E2", "1e-2", "-0", "-0.0", "1.00", "2147483647",
            "2147483648", "-2147483648", "-2147483649", "9223372036854775807", "9223372036854775808", "1e19",
            "123456789012345678901234567890", "255", "256", "-128", "-129", "65535", "65536", "-32769", "128",
            "\"5\"", "\"5.0\"", "\"5.5\"", "\" 5\"", "\"+5\"", "\"-5\"", "\"abc\"", "\"\"", "\"0x10\"", "\"1e2\"",
            "\"NaN\"", "NaN", "Infinity", "-Infinity", "\"Infinity\"", "\"1.5f\"", "\"٥\"", "\"2147483648\"",
            "\"9223372036854775808\"", "\"1e19\"", "01", "-01", "'5'", "abc", "true", "false", "\"true\"",
            "\"TRUE\"", "\"yes\"", "\"false\"", "TRUE", "null", "[]", "[1]", "{}", "\"a\"", "\"ab\"", "\"😀\"",
            "\"é\"", "\"REGULAR\"", "\"FINAL\"", "\"BOGUS\"", "\"regular\"", "\"2026-11-14T15:26Z\"",
            "\"2026-11-14T15:26:46.1234+08:00\"", "\"not a date\"", "1.7976931348623157E309", "4.9E-325",
            "0.1", "16777217", "\"16777217\"", "1e400", "{\"id\":3}", "[[]]"
    };

    static final Map<String, Class<?>> TARGETS = new LinkedHashMap<>();
    static {
        TARGETS.put("int", IntBox.class);
        TARGETS.put("long", LongBox.class);
        TARGETS.put("short", ShortBox.class);
        TARGETS.put("byte", ByteBox.class);
        TARGETS.put("float", FloatBox.class);
        TARGETS.put("double", DoubleBox.class);
        TARGETS.put("boolean", BoolBox.class);
        TARGETS.put("char", CharBox.class);
        TARGETS.put("String", StringBox.class);
        TARGETS.put("Integer", IntegerBox.class);
        TARGETS.put("enum", KindBox.class);
        TARGETS.put("OffsetDateTime", TimeBox.class);
        TARGETS.put("List", ListBox.class);
        TARGETS.put("object", InnerBox.class);
    }

    static String show(Object box) throws ReflectiveOperationException {
        Object v = box.getClass().getDeclaredField("v").get(box);
        if (v == null) return "null";
        if (v instanceof Float f) return Float.toString(f);
        if (v instanceof Double d) return Double.toString(d);
        if (v instanceof Inner || v instanceof List) return COMPACT.toJson(v);
        return String.valueOf(v);
    }

    static void coerce(Path p) throws IOException {
        try (PrintWriter w = open(p)) {
            for (Map.Entry<String, Class<?>> t : TARGETS.entrySet()) {
                for (String value : VALUES) {
                    String json = "{\"v\":" + value + "}";
                    String result;
                    try {
                        Object box = PRETTY.fromJson(json, t.getValue());
                        result = "OK " + show(box);
                    } catch (Throwable e) {
                        result = err(e);
                    }
                    w.println(t.getKey() + "\t" + Esc.esc(value) + "\t" + Esc.esc(result));
                }
            }
            // records: null into a primitive component fails (classes keep the default)
            for (String json : new String[]{"{\"a\":null}", "{}", "{\"a\":1,\"b\":\"x\",\"c\":\"2.5\"}", "{\"a\":1.5}"}) {
                String result;
                try {
                    result = "OK " + COMPACT.toJson(PRETTY.fromJson(json, Rec.class));
                } catch (Throwable e) {
                    result = err(e);
                }
                w.println("record\t" + Esc.esc(json) + "\t" + Esc.esc(result));
            }
        }
    }

    // ------------------------------------------------------------------ syntax: JsonParser

    static final String[] DOCS = {
            "", " ", "\n\t ", "null", "null garbage", "nul", "{}", "[]", "1", "-0", "1.50", "1e5", "01", "-", "\"x\"",
            "'x'", "x", "not-json", "true", "TRUE", "tRuE", "True", "NULL", "nullx", "{} {}", "{} x", "{} }",
            "[1,]", "[,]", "[1,,2]", "[1;2]", "{a:1}", "{'a':1}", "{\"a\"=1}", "{\"a\"=>1}", "{\"a\":1;\"b\":2}",
            "{\"a\":1,}", "{\"a\":1,\"a\":2,\"b\":3}", "// c\n{\"a\":1}", "# c\n[1]", "/* c */ [1]", "/* c [1]",
            "[1] // trailing", "[1] /* x", ")]}'\n[1]", "\uFEFF[1]", "{\"a\":\"\\u0041\\n\\/\\'\"}", "\"\\x\"",
            "\"\\u12\"", "\"\\u12G4\"", "\"unterminated", "[1, 2", "{\"a\":", "{\"a\" 1}", "{,}", "{\"a\":1 \"b\":2}",
            "[\"a\" \"b\"]", "{1:2}", "{\"a\":{\"b\":[1,{\"c\":nul}]}}", "\"\\ud83d\\ude00\"", "\"\\ud83d\"",
            "[1e400]", "[-1.5e-3]", "[123abc]", "[1.5.3]", "[NaN]", "[-Infinity]", "[0x10]", "[+1]", "[.5]",
            "[5.]", "{\"\":1}", "{\"a\":[1,2],\"b\":{\"c\":null}}", "[\"\\u2028\"]", "\"a\nb\"", "{\"a\":1}\n\n x",
            "[\n1,\n2\n,\n]", "{\"a\":\n\n1\n,\n}", "[[[[[[[[[[1]]]]]]]]]]", "{a b:1}", "{\"a\":b c}", "[a=b]",
            "[\"<&>='\"]"
    };

    static void parse(Path p) throws IOException {
        try (PrintWriter w = open(p)) {
            for (String doc : DOCS) {
                String toString;
                String pretty = "";
                try {
                    JsonElement e = JsonParser.parseString(doc);
                    toString = "OK " + e.toString();
                    pretty = PRETTY.toJson(e);
                } catch (Throwable e) {
                    toString = err(e);
                }
                w.println(Esc.esc(doc) + "\t" + Esc.esc(toString) + "\t" + Esc.esc(pretty));
            }
            // nesting limit
            String deep = "[".repeat(300) + "]".repeat(300);
            String ok = "[".repeat(255) + "]".repeat(255);
            for (String doc : new String[]{deep, ok}) {
                String r;
                try {
                    r = "OK " + JsonParser.parseString(doc).toString().length();
                } catch (Throwable e) {
                    r = err(e);
                }
                w.println(Esc.esc(doc) + "\t" + Esc.esc(r) + "\t");
            }
        }
    }

    // ------------------------------------------------------------------ whole documents into a class

    static void documents(Path p) throws IOException {
        String[] docs = {
                "", "   ", "// only a comment", "null", "null garbage", "{}", "{} garbage", "{} {}", "{} }",
                "{\"a\":1}", "{\"a\":1,\"a\":2}", "{\"a\":1,\"a\":null}", "{\"a\":null,\"a\":3}", "{\"c\":null}",
                "{\"c\":{\"id\":1,\"id\":2}}", "{\"unknown\":[1,{\"x\":nul}],\"a\":3}", "{\"a\":1,\"b\":\"x\",\"c\":{\"id\":5,\"label\":\"l\",\"zzz\":1}}",
                "[]", "\"str\"", "{\"a\":\"1\"}", "{\"a\":true}", "{\"b\":5}", "{\"b\":{}}", "{\"c\":[]}", "{\"c\":\"x\"}",
                "{a:1,b:'two',c:{id:3}}", "{\"a\":1", "{\"a\":1}}", ")]}'\n{\"a\":1}", "\uFEFF{\"a\":1}", "{\"a\":01}"
        };
        try (PrintWriter w = open(p)) {
            for (String doc : docs) {
                String r;
                try {
                    Pair pair = PRETTY.fromJson(doc, Pair.class);
                    r = pair == null ? "NULL" : "OK " + COMPACT.toJson(pair);
                } catch (Throwable e) {
                    r = err(e);
                }
                w.println(Esc.esc(doc) + "\t" + Esc.esc(r));
            }
        }
    }

    // ------------------------------------------------------------------ maps

    static void maps(Path p) throws IOException {
        Type stringInt = new TypeToken<LinkedHashMap<String, Integer>>() {}.getType();
        Type intString = new TypeToken<LinkedHashMap<Integer, String>>() {}.getType();
        Type intInner = new TypeToken<LinkedHashMap<Integer, Inner>>() {}.getType();
        Object[][] cases = {
                {"si", stringInt, "{\"a\":1,\"b\":2}"},
                {"si", stringInt, "{\"a\":1,\"a\":2}"},
                {"si", stringInt, "{\"a\":null,\"a\":2}"},
                {"si", stringInt, "{\"a\":\"3\",\"b\":4.0}"},
                {"si", stringInt, "[[\"a\",1],[\"b\",2]]"},
                {"si", stringInt, "[[\"a\",1,2]]"},
                {"si", stringInt, "[[\"a\"]]"},
                {"si", stringInt, "[\"a\"]"},
                {"si", stringInt, "[[1,2]]"},
                {"si", stringInt, "{}"},
                {"si", stringInt, "[]"},
                {"si", stringInt, "\"x\""},
                {"is", intString, "{\"1\":\"a\",\"2\":\"b\"}"},
                {"is", intString, "{\"1.0\":\"a\"}"},
                {"is", intString, "{\"x\":\"a\"}"},
                {"is", intString, "{\"1\":\"a\",\"01\":\"b\"}"},
                {"is", intString, "{\"1\":1}"},
                {"is", intString, "{\"1\":true}"},
                {"is", intString, "[[1,\"a\"],[\"2\",\"b\"]]"},
                {"ii", intInner, "{\"5\":{\"id\":5,\"label\":\"x\"},\"6\":{}}"},
                {"ii", intInner, "{\"5\":null}"},
        };
        try (PrintWriter w = open(p)) {
            for (Object[] c : cases) {
                String r;
                try {
                    Object m = PRETTY.fromJson((String) c[2], (Type) c[1]);
                    r = "OK " + COMPACT.toJson(m);
                } catch (Throwable e) {
                    r = err(e);
                }
                w.println(c[0] + "\t" + Esc.esc((String) c[2]) + "\t" + Esc.esc(r));
            }
        }
    }

    // ------------------------------------------------------------------ field names

    static void names(Path p) throws IOException {
        try (PrintWriter w = open(p)) {
            for (var f : NamesHolder.class.getDeclaredFields()) {
                w.println(f.getName() + "\t" + FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES.translateName(f));
            }
        }
    }
}
