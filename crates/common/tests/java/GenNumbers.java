import java.io.IOException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Random;

/**
 * Golden vectors for zphira_common::java_fmt, recorded from the running JDK.
 *
 * <p>Run: {@code java crates/common/tests/java/GenNumbers.java crates/common/tests/fixtures}
 * (see crates/common/tests/java/README for all generators).
 *
 * <p>Fields are tab separated and escaped with {@link Esc#esc}.
 */
public class GenNumbers {

    public static void main(String[] args) throws IOException {
        Locale.setDefault(Locale.US);
        Path out = Path.of(args[0]);
        Files.createDirectories(out);
        floats(out.resolve("float_to_string.tsv"));
        doubles(out.resolve("double_to_string.tsv"));
        fixed(out.resolve("format_fixed.tsv"));
        parseDouble(out.resolve("parse_double.tsv"));
        parseInts(out.resolve("parse_int.tsv"));
    }

    static PrintWriter open(Path p) throws IOException {
        return new PrintWriter(Files.newBufferedWriter(p, StandardCharsets.UTF_8));
    }

    // ------------------------------------------------------------------ Float.toString

    static void floats(Path p) throws IOException {
        List<Float> values = new ArrayList<>();
        float[] specials = {0f, -0f, Float.MIN_VALUE, -Float.MIN_VALUE, Float.MIN_NORMAL, Float.MAX_VALUE,
                -Float.MAX_VALUE, Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY, 1f, -1f, 0.1f,
                0.2f, 0.3f, 1.1f, 98.76f, 100f, 0.9876f * 100, 0.12345f * 100, 0.01f * 1000, 0.0123f * 1000,
                1.0E-4f, 1.2345678E7f, 9999999f, 1.0E7f, 0.001f, 0.00099999f, 3.4028235E38f, 1.17549435E-38f,
                16777216f, 16777217f, 33554432f, 8.5f, 2.5f, 0.5f, 0.25f, 0.125f, 1.0E-45f, 2.0E-45f, 3.0E-45f,
                5.0E-45f, 7.0E-45f, 1.1E-44f, 2.8E-45f};
        for (float f : specials) values.add(f);
        for (int e = -46; e <= 39; e++) {
            float f = Float.parseFloat("1e" + e);
            values.add(f);
            values.add(Math.nextUp(f));
            values.add(Math.nextDown(f));
            values.add(-f);
            for (int m = 2; m <= 9; m++) values.add(Float.parseFloat(m + "e" + e));
        }
        for (int i = 1; i < 64; i++) values.add(Float.intBitsToFloat(i));
        for (int i = 0; i <= 1000; i++) {
            values.add(i / 1000f * 100);           // accuracy * 100 style
            values.add((float) (i / 1000.0) * 100);
            values.add(i * 0.001f * 1000);          // std * 1000 style
        }
        for (int i = 0; i < 24; i++) values.add((float) (1 << i));
        Random r = new Random(20261114L);
        for (int i = 0; i < 4000; i++) values.add(Float.intBitsToFloat(r.nextInt()));
        for (int i = 0; i < 1000; i++) values.add(r.nextFloat());
        for (int i = 0; i < 1000; i++) values.add(r.nextFloat() * 1000);
        try (PrintWriter w = open(p)) {
            for (float f : values) {
                w.println(String.format("%08x", Float.floatToRawIntBits(f)) + "\t" + Float.toString(f));
            }
        }
    }

    // ------------------------------------------------------------------ Double.toString

    static void doubles(Path p) throws IOException {
        List<Double> values = new ArrayList<>();
        double[] specials = {0d, -0d, Double.MIN_VALUE, -Double.MIN_VALUE, Double.MIN_NORMAL, Double.MAX_VALUE,
                Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, 1d, 0.1, 0.2, 0.3, 0.1 + 0.2,
                1.0E23, 2.0E23, 9.007199254740991E15, 9.007199254740992E15, 1.0E22, 5.0E-324, 1.0E-323,
                2.2250738585072014E-308, 1.7976931348623157E308, 1.0E7, 9999999.999999998, 0.001,
                9.999999999999998E-4, 1.0E-4, 123456.789, 1.0 / 3, 2.0 / 3, Math.PI, Math.E,
                (double) 0.9876f * 100, (double) 0.1f, 4.35, 0.15, 1.005, 2.675};
        for (double d : specials) values.add(d);
        for (int e = -325; e <= 309; e++) {
            double d = Double.parseDouble("1e" + e);
            values.add(d);
            values.add(Math.nextUp(d));
            values.add(Math.nextDown(d));
            values.add(Double.parseDouble("5e" + e));
            values.add(Double.parseDouble("9.87654321e" + e));
        }
        for (long i = 1; i < 64; i++) values.add(Double.longBitsToDouble(i));
        for (int ms = 0; ms <= 3000; ms += 7) values.add(ms / 1000.0);
        for (int i = 0; i < 64; i++) values.add((double) (1L << i));
        Random r = new Random(1114L);
        for (int i = 0; i < 4000; i++) values.add(Double.longBitsToDouble(r.nextLong()));
        for (int i = 0; i < 1000; i++) values.add(r.nextDouble());
        for (int i = 0; i < 1000; i++) values.add(r.nextDouble() * 1e6);
        try (PrintWriter w = open(p)) {
            for (double d : values) {
                w.println(String.format("%016x", Double.doubleToRawLongBits(d)) + "\t" + Double.toString(d));
            }
        }
    }

    // ------------------------------------------------------------------ String.format("%.Nf")

    static void fixed(Path p) throws IOException {
        List<double[]> cases = new ArrayList<>();
        double[] values = {0, -0.0, 0.5, 1.5, 2.5, -2.5, 0.125, 0.375, 1.005, 2.675, 0.15, 0.25, 0.35, 0.45,
                1e-7, -1e-7, 0.0005, 0.0015, 0.9995, 9.9995, 99.9995, 999.9995, 123456789.123456789, 1e15, 1e16,
                1e17, 1e20, 1e22, 1e23, 1e300, Double.MAX_VALUE, Double.MIN_VALUE, 1e-300, 0.1, 0.2, 0.3,
                (double) 0.1f, (double) 0.9876f * 100, 4.35, 4.345, 4.3449999, Double.NaN,
                Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, 5e-5, 4.9999e-5, 0.95, 0.05, 0.005, 99.5,
                999999.5, 1234.5678, -1234.5678, 1.0 / 3, 2.0 / 3, Math.PI, 0.045, 0.055, 1.45, 1.55};
        for (double v : values) {
            for (int prec = 0; prec <= 8; prec++) cases.add(new double[]{v, prec});
            cases.add(new double[]{v, 12});
            cases.add(new double[]{v, 20});
        }
        for (int ms = 0; ms <= 5000; ms += 3) cases.add(new double[]{ms / 1000.0, 3});
        for (int i = 0; i < 2000; i += 5) {
            cases.add(new double[]{i / 1000.0 + 0.0005, 3}); // ties after shortest repr
            cases.add(new double[]{i / 100.0 + 0.005, 2});
            cases.add(new double[]{i / 10.0 + 0.05, 1});
        }
        Random r = new Random(77L);
        for (int i = 0; i < 1500; i++) cases.add(new double[]{(r.nextDouble() - 0.5) * Math.pow(10, r.nextInt(12) - 4), r.nextInt(7)});
        for (int i = 0; i < 300; i++) cases.add(new double[]{Double.longBitsToDouble(r.nextLong()), r.nextInt(5)});
        try (PrintWriter w = open(p)) {
            for (double[] c : cases) {
                int prec = (int) c[1];
                w.println(String.format("%016x", Double.doubleToRawLongBits(c[0])) + "\t" + prec + "\t"
                        + String.format("%." + prec + "f", c[0]));
            }
        }
    }

    // ------------------------------------------------------------------ Double.parseDouble

    static void parseDouble(Path p) throws IOException {
        String[] inputs = {"", " ", "0", "-0", "+0", "1", "-1", "+1", "1.5", ".5", "5.", ".", "-.", "1e5", "1E5",
                "1e+5", "1e-5", "1e", "1e+", "e5", "1.5f", "1.5F", "1.5d", "1.5D", "1.5x", "1.5ff", " 1.5 ",
                "\t1.5\n", "\u00001.5\u0000", "1 .5", "1..5", "1.5.", "NaN", "-NaN", "+NaN", "nan", "NaNx",
                "Infinity", "-Infinity", "+Infinity", "infinity", "Inf", " Infinity ", "Infinityf", "0x1p0",
                "0x1.8p1", "0X1P-2", "0x.8p1", "0x1", "0x1p", "0x1pf", "0xp1", "0x1.p1", "-0x1p-1074",
                "0x1p-1075", "0x1.0000000000001p0", "0x1.00000000000008p0", "0x1.00000000000018p0",
                "0x1p1024", "0x1.fffffffffffffp1023", "0x1.fffffffffffff8p1023", "0x0.0p0", "0x123456789abcdefp0",
                "1e400", "-1e400", "1e-400", "4.9e-324", "2.4703282292062327e-324", "2.4703282292062328e-324",
                "1.7976931348623157e308", "1.7976931348623158e308", "1.7976931348623159e308",
                "9007199254740993", "9007199254740995", "0.1", "0.30000000000000004", "1e10000000000",
                "1e-10000000000", "1e99999999999999999999", "123456789012345678901234567890", "00000.000001e6",
                "0e999999", "１", "٥", "1_000", "1,5", "--1", "+-1", "1e5.5", "0x1.8p1d", "12345e-5f",
                "2.5E-3", "100", "5.0", "-5.0", "3.4028235E38", "1.0E-45", "true", "abc", "1e-5d ", " 1"};
        try (PrintWriter w = open(p)) {
            for (String s : inputs) {
                String result;
                try {
                    result = String.format("%016x", Double.doubleToRawLongBits(Double.parseDouble(s)));
                } catch (NumberFormatException e) {
                    result = "ERR " + e.getMessage();
                }
                w.println(Esc.esc(s) + "\t" + Esc.esc(result));
            }
        }
    }

    // ------------------------------------------------------------------ Integer/Long.parseInt

    static void parseInts(Path p) throws IOException {
        String[] inputs = {"", "0", "-0", "+0", "5", "+5", "-5", "05", "2147483647", "2147483648", "-2147483648",
                "-2147483649", "9223372036854775807", "9223372036854775808", "-9223372036854775808",
                "-9223372036854775809", "1.0", "1e3", " 5", "5 ", "+", "-", "--5", "+-5", "٥", "１２", "໐",
                "꩙", "𝟓", "0x10", "abc", "12a", "99999999999999999999999", "true", "TRUE",
                "tRuE", " true", "true ", "false", "yes", "1"};
        try (PrintWriter w = open(p)) {
            for (String s : inputs) {
                String ri;
                String rl;
                try {
                    ri = Integer.toString(Integer.parseInt(s));
                } catch (NumberFormatException e) {
                    ri = "ERR " + e.getMessage();
                }
                try {
                    rl = Long.toString(Long.parseLong(s));
                } catch (NumberFormatException e) {
                    rl = "ERR " + e.getMessage();
                }
                w.println(Esc.esc(s) + "\t" + Esc.esc(ri) + "\t" + Esc.esc(rl) + "\t" + Boolean.parseBoolean(s));
            }
        }
    }
}
