package top.rymc.phira.main.util;

import com.sun.net.httpserver.HttpServer;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.net.InetSocketAddress;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Random;
import java.util.concurrent.ConcurrentHashMap;
import java.util.zip.CRC32;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

/**
 * Golden vectors for zphira_phira::duration, recorded by running the real compiled
 * {@code ChartDuration} classes of the Java server (same package, so the package-private
 * parsers and {@code probe} are reachable).
 *
 * <p>Writes into the output directory:
 * <ul>
 *   <li>{@code duration_parse.tsv}: one row per crafted input of the pure parsers:
 *       category, kind, declared size, input as hex, Java result, note.</li>
 *   <li>{@code zip_*.zip}, {@code bare_*.bin}, {@code duration_probe.tsv}: archives and bare files
 *       and what {@code probe(url)} returned for them against a local range server (127.0.0.1,
 *       with a 303 hop to a second path like the real {@code /files} endpoint). One row per
 *       file: name, category, payload ({@code file:<name>} is written next to the table,
 *       {@code recipe:<id>} is rebuilt by the Rust test), Java result.</li>
 *   <li>{@code real_audio.tsv}: Java's result for real LAME/ffmpeg encodes ({@code real/manifest.tsv},
 *       made by {@code make_real_audio.sh}), next to ffprobe's duration.</li>
 * </ul>
 *
 * <p>Categories: {@code same} = Java is correct, Rust must return the same value;
 * {@code exc} = Java threw (Rust must merely not panic); {@code A-xx} = a finding that Rust
 * fixes, the recorded value is the flawed Java result and the Rust test asserts the
 * corrected one.
 *
 * <p>Run (see README.md): compile together with {@code Esc.java} against the server's compiled
 * classes, then {@code java -Duser.timezone=UTC -cp ... top.rymc.phira.main.util.GenDuration ../fixtures}.
 * A daemon watchdog halts the JVM after 120 s, so a blocked socket cannot hang the run.
 */
public final class GenDuration {

    static final Random R = new Random(20261115L);
    static PrintWriter parse;

    // ------------------------------------------------------------------ byte helpers

    static void putInt(byte[] d, int off, int v) {
        ByteBuffer.wrap(d, off, 4).order(ByteOrder.LITTLE_ENDIAN).putInt(v);
    }

    static void putIntBE(byte[] d, int off, int v) {
        ByteBuffer.wrap(d, off, 4).order(ByteOrder.BIG_ENDIAN).putInt(v);
    }

    static void putAscii(byte[] d, int off, String s) {
        System.arraycopy(s.getBytes(StandardCharsets.ISO_8859_1), 0, d, off, s.length());
    }

    static String hex(byte[] d) {
        StringBuilder b = new StringBuilder(d.length * 2);
        for (byte x : d) {
            b.append(Character.forDigit((x >> 4) & 0xF, 16)).append(Character.forDigit(x & 0xF, 16));
        }
        return b.toString();
    }

    static byte[] randomBytes(int n) {
        byte[] d = new byte[n];
        R.nextBytes(d);
        return d;
    }

    /** Random bytes that cannot form an MPEG sync (no 0xFF) nor the ASCII tags we look for. */
    static byte[] noise(int n) {
        byte[] d = new byte[n];
        for (int i = 0; i < n; i++) {
            d[i] = (byte) (R.nextInt(0xFF)); // 0..254, never 0xFF
        }
        return d;
    }

    static byte[] concat(byte[]... parts) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        for (byte[] p : parts) {
            out.writeBytes(p);
        }
        return out.toByteArray();
    }

    /** MPEG audio frame header: version bits (3 = MPEG-1, 2 = MPEG-2, 0 = MPEG-2.5), layer bits (1 = III). */
    static byte[] header(int version, int layer, int bitrateIdx, int rateIdx, int channelMode) {
        byte b1 = (byte) (0xE0 | (version << 3) | (layer << 1) | 1);
        byte b2 = (byte) ((bitrateIdx << 4) | (rateIdx << 2));
        byte b3 = (byte) (channelMode << 6);
        return new byte[]{(byte) 0xFF, b1, b2, b3};
    }

    /** Like {@link #header} with the protection bit clear: a 16-bit CRC follows the header. */
    static byte[] headerCrc(int version, int layer, int bitrateIdx, int rateIdx, int channelMode) {
        byte[] h = header(version, layer, bitrateIdx, rateIdx, channelMode);
        h[1] &= (byte) 0xFE;
        return h;
    }

    // ------------------------------------------------------------------ parser rows

    /**
     * Whether Java's WAV walk stops at a zero-size chunk before reaching {@code data} (N-29): the
     * same loop as {@code parseWavDuration}, over long arithmetic, up to that point.
     */
    static boolean zeroChunkStopsWalk(byte[] d) {
        if (d.length < 44) {
            return false;
        }
        long position = 12;
        while (position + 8 <= d.length) {
            String id = new String(d, (int) position, 4, StandardCharsets.ISO_8859_1);
            int size = ByteBuffer.wrap(d, (int) position + 4, 4).order(ByteOrder.LITTLE_ENDIAN).getInt();
            if (id.equals("data")) {
                return false;
            }
            if (size == 0) {
                return true;
            }
            if (size < 0) {
                return false;
            }
            position += 8L + size + (size & 1);
        }
        return false;
    }

    static void row(String category, String kind, long declared, byte[] data, String note) {
        if (category.equals("same") && kind.equals("wav") && zeroChunkStopsWalk(data)) {
            category = "N-29";
        }
        String result;
        try {
            Integer v = switch (kind) {
                case "wav" -> ChartDuration.parseWavDuration(data);
                case "ogg" -> ChartDuration.parseOggDuration(data, declared);
                case "mp3" -> ChartDuration.parseMp3Duration(data, declared);
                default -> throw new IllegalArgumentException(kind);
            };
            result = v == null ? "null" : v.toString();
        } catch (Throwable t) {
            result = "EXC " + t.getClass().getSimpleName();
            if (category.equals("same")) {
                category = "exc";
            }
        }
        parse.println(category + "\t" + kind + "\t" + declared + "\t" + hex(data) + "\t" + result + "\t" + Esc.esc(note));
    }

    /** {@code extraChunkSize} value that leaves the LIST chunk out altogether. */
    static final int NO_LIST = Integer.MIN_VALUE;

    static byte[] wav(int byteRate, int dataSize, int extraChunkSize, boolean listFirst) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] riff = new byte[12];
        putAscii(riff, 0, "RIFF");
        putInt(riff, 4, 36);
        putAscii(riff, 8, "WAVE");
        out.writeBytes(riff);
        byte[] fmt = new byte[24];
        putAscii(fmt, 0, "fmt ");
        putInt(fmt, 4, 16);
        putInt(fmt, 8, 1 | (2 << 16));
        putInt(fmt, 12, 44100);
        putInt(fmt, 16, byteRate);
        putInt(fmt, 20, 4 | (16 << 16));
        int physical = Math.max(0, Math.min(extraChunkSize, 6000));
        byte[] list = new byte[8 + physical + (physical & 1)];
        putAscii(list, 0, "LIST");
        putInt(list, 4, extraChunkSize);
        if (extraChunkSize == NO_LIST) {
            out.writeBytes(fmt);
        } else if (listFirst) {
            out.writeBytes(list);
            out.writeBytes(fmt);
        } else {
            out.writeBytes(fmt);
            out.writeBytes(list);
        }
        byte[] data = new byte[8 + 16];
        putAscii(data, 0, "data");
        putInt(data, 4, dataSize);
        out.writeBytes(data);
        return out.toByteArray();
    }

    static byte[] ogg(int segments, int nominal, int totalLength) {
        byte[] d = new byte[totalLength];
        putAscii(d, 0, "OggS");
        if (d.length > 26) {
            d[26] = (byte) segments;
        }
        for (int i = 0; i < segments && 27 + i < d.length; i++) {
            d[27 + i] = (byte) R.nextInt(256);
        }
        int body = 27 + segments;
        if (body + 28 <= d.length) {
            d[body] = 1;
            putAscii(d, body + 1, "vorbis");
            putInt(d, body + 12, 44100);
            putInt(d, body + 16, nominal + 1000);
            putInt(d, body + 20, nominal);
            putInt(d, body + 24, nominal - 1000);
        }
        return d;
    }

    static void parserRows() {
        // ---- the nine cases of ChartDurationParseTest that stay valid, in Java's own encoding
        byte[] w = new byte[44];
        putAscii(w, 0, "RIFF");
        putInt(w, 4, 36);
        putAscii(w, 8, "WAVE");
        putAscii(w, 12, "fmt ");
        putInt(w, 16, 16);
        putInt(w, 20, 1 | (2 << 16));
        putInt(w, 24, 44100);
        putInt(w, 28, 176400);
        putAscii(w, 36, "data");
        putInt(w, 40, 529200);
        row("same", "wav", 0, w, "java test: wavIsReadFromItsChunks");
        row("same", "wav", 0, new byte[8], "java test: truncatedWavIsRejected");

        byte[] o = new byte[64];
        putAscii(o, 0, "OggS");
        o[26] = 1;
        o[27] = 30;
        putAscii(o, 28, "\u0001vorbis");
        putInt(o, 28 + 12, 44100);
        putInt(o, 28 + 16, 0);
        putInt(o, 28 + 20, 128000);
        row("same", "ogg", 256000, o, "java test: oggUsesTheNominalBitrate");
        byte[] o2 = new byte[64];
        putAscii(o2, 0, "OggS");
        row("same", "ogg", 256000, o2, "java test: oggNeedsItsIdentificationHeader");

        byte[] frame = new byte[64];
        frame[0] = (byte) 0xFF;
        frame[1] = (byte) 0xFB;
        frame[2] = (byte) 0x90;
        row("same", "mp3", 128000, frame, "java test: constantBitrateFallsBackToTheFileSize");
        row("same", "mp3", 0, new byte[0], "java test: garbage (empty)");
        row("same", "mp3", 0, "not audio at all".getBytes(StandardCharsets.ISO_8859_1), "java test: garbage (text)");

        // ---- WAV: structured and mutated
        for (int i = 0; i < 300; i++) {
            int byteRate = switch (R.nextInt(6)) {
                case 0 -> 0;
                case 1 -> -R.nextInt(1000);
                default -> 1 + R.nextInt(2_000_000);
            };
            int dataSize = switch (R.nextInt(6)) {
                case 0 -> 0;
                case 1 -> -R.nextInt(1000);
                case 2 -> Integer.MAX_VALUE - R.nextInt(100);
                default -> R.nextInt(400_000_000);
            };
            int extra = switch (R.nextInt(5)) {
                case 0 -> NO_LIST;
                case 1 -> 1 + R.nextInt(30);
                case 2 -> -R.nextInt(50);
                case 3 -> Integer.MAX_VALUE - R.nextInt(100);
                default -> R.nextInt(5000);
            };
            byte[] d = wav(byteRate, dataSize, extra, R.nextBoolean());
            if (R.nextInt(4) == 0) {
                d = java.util.Arrays.copyOf(d, R.nextInt(d.length + 1));
            }
            if (R.nextInt(3) == 0 && d.length > 0) {
                for (int k = 0; k < 1 + R.nextInt(3); k++) {
                    d[R.nextInt(d.length)] ^= (byte) (1 << R.nextInt(8));
                }
            }
            row("same", "wav", 0, d, "random wav " + i);
        }

        row("same", "wav", 0, wav(176400, 1_764_000, NO_LIST, false), "fmt then data: 10 s");
        row("N-29", "wav", 0, wav(176400, 1_764_000, 0, false),
                "empty LIST chunk before data: Java stops at the zero-size chunk");
        row("N-29", "wav", 0, wav(176400, 1_764_000, 0, true),
                "empty LIST chunk before fmt: Java stops at the zero-size chunk");
        row("same", "wav", 0, wav(176400, 1_764_000, 3, false), "LIST chunk of odd size 3 (padding byte)");

        // ---- Ogg: structured and mutated
        // sizes whose estimate fits an int for every nominal bitrate (the huge ones are explicit A-28 rows)
        long[] sizes = {0, 1, 100, 256000, 3_000_000};
        for (int i = 0; i < 300; i++) {
            int nominal = switch (R.nextInt(6)) {
                case 0 -> 0;
                case 1 -> -R.nextInt(1000);
                case 2 -> 1 + R.nextInt(100);
                default -> 32000 + R.nextInt(500_000);
            };
            int segments = R.nextInt(5) == 0 ? R.nextInt(256) : R.nextInt(4);
            int total = 20 + R.nextInt(160);
            byte[] d = ogg(segments, nominal, total);
            if (R.nextInt(3) == 0) {
                d[R.nextInt(d.length)] ^= (byte) (1 << R.nextInt(8));
            }
            row("same", "ogg", sizes[R.nextInt(sizes.length)], d, "random ogg " + i);
        }

        // ---- MP3: constant bitrate, MPEG-1 Layer III, no ID3, no Xing/Info
        long[] declared = {1, 100, 128000, 6_400_000, 4_294_967_295L, 2_147_483_649L};
        for (int i = 0; i < 300; i++) {
            int lead = R.nextInt(3) == 0 ? R.nextInt(20) : 0;
            byte[] pre = noise(lead);
            // sometimes a decoy: a sync pattern whose header is invalid (bitrate index 0 or 15)
            byte[] decoy = null;
            if (R.nextInt(6) == 0) {
                decoy = new byte[]{(byte) 0xFF, (byte) 0xFB, (byte) (R.nextBoolean() ? 0x00 : 0xF0), 0};
            }
            int bitrateIdx = R.nextInt(16);
            int rateIdx = R.nextInt(4);
            int mode = R.nextInt(4);
            byte[] h = header(3, 1, bitrateIdx, rateIdx, mode);
            byte[] tailNoise = noise(60 + R.nextInt(100));
            long size = declared[R.nextInt(declared.length)];
            if (decoy == null) {
                row("same", "mp3", size, concat(pre, h, tailNoise), "random cbr mp3 " + i);
            } else {
                // N-30: Java gives up at the decoy; the expectation is what the real Java parser
                // says about the same bytes without it
                Integer clean = ChartDuration.parseMp3Duration(concat(pre, h, tailNoise), size);
                row("N-30", "mp3", size, concat(pre, decoy, h, tailNoise),
                        "decoy sync ahead of the frame; expect=" + (clean == null ? "null" : clean));
            }
        }

        // ---- N-30: a stray sync ahead of the first frame, and CRC protected frames
        byte[] frameAfterJunk = new byte[200];
        frameAfterJunk[0] = (byte) 0xFF;
        frameAfterJunk[1] = (byte) 0xFF;
        System.arraycopy(header(3, 1, 9, 0, 0), 0, frameAfterJunk, 8, 4);
        row("N-30", "mp3", 128000, frameAfterJunk, "FF FF junk ahead of the frame; expect=8");
        byte[] tagWithJunk = new byte[300];
        putAscii(tagWithJunk, 0, "ID3");
        tagWithJunk[3] = 3; // the size field says 0, but the tag body follows: junk with a Layer II looking sync
        tagWithJunk[14] = (byte) 0xFF;
        tagWithJunk[15] = (byte) 0xF5;
        System.arraycopy(header(3, 1, 9, 0, 0), 0, tagWithJunk, 100, 4);
        row("N-30", "mp3", 128000 + 10, tagWithJunk, "tag body with a wrong ID3 size and an FF F5 sync; expect=8");
        byte[] crcXing = new byte[200];
        System.arraycopy(headerCrc(3, 1, 9, 0, 0), 0, crcXing, 0, 4);
        putAscii(crcXing, 4 + 2 + 32, "Xing");
        putIntBE(crcXing, 4 + 2 + 32 + 4, 0xF);
        putIntBE(crcXing, 4 + 2 + 32 + 8, 9000);
        row("N-30", "mp3", 235L * 245000 / 8, crcXing, "CRC protected MPEG-1 stereo, Xing after the CRC bytes, 9000 frames; expect=235");
        byte[] crcXingMono = new byte[200];
        System.arraycopy(headerCrc(2, 1, 9, 0, 3), 0, crcXingMono, 0, 4);
        putAscii(crcXingMono, 4 + 2 + 9, "Info");
        putIntBE(crcXingMono, 4 + 2 + 9 + 4, 1);
        putIntBE(crcXingMono, 4 + 2 + 9 + 8, 20000);
        row("N-30", "mp3", 522L * 10000, crcXingMono, "CRC protected MPEG-2 mono, Info after the CRC bytes, 20000 frames; expect=522");
        byte[] crcPlain = new byte[200];
        System.arraycopy(headerCrc(3, 1, 9, 0, 0), 0, crcPlain, 0, 4);
        row("same", "mp3", 128000, crcPlain, "CRC protected frame without a tag: the size estimate");

        // ---- A-02: Xing/Info fields are big-endian
        byte[] x = new byte[200];
        System.arraycopy(header(3, 1, 9, 0, 0), 0, x, 0, 4);
        putAscii(x, 4 + 32, "Xing");
        putIntBE(x, 4 + 32 + 4, 0x0000000F);
        putIntBE(x, 4 + 32 + 8, 9000);
        row("A-02", "mp3", 235L * 245000 / 8, x, "MPEG-1 stereo Xing, BE flags 0xF and 9000 frames: true 235 s");
        byte[] xm = x.clone();
        System.arraycopy(header(3, 1, 9, 0, 3), 0, xm, 0, 4);
        java.util.Arrays.fill(xm, 4, xm.length, (byte) 0);
        putAscii(xm, 4 + 17, "Info");
        putIntBE(xm, 4 + 17 + 4, 1);
        putIntBE(xm, 4 + 17 + 8, 44100);
        row("A-02", "mp3", 1_000_000, xm, "MPEG-1 mono Info, BE 44100 frames: true 1152 s");
        byte[] xle = x.clone();
        putInt(xle, 4 + 32 + 4, 0x1);
        putInt(xle, 4 + 32 + 8, 1000);
        row("A-02", "mp3", 128000, xle, "the old test fixture: little-endian fields (1000 frames)");
        byte[] xnoframes = x.clone();
        putIntBE(xnoframes, 4 + 32 + 4, 0x2);
        row("A-02", "mp3", 128000, xnoframes, "Xing without the frames flag falls back to the bitrate");

        // ---- A-17: the ID3v2 tag is not audio
        int id3 = 16_000;
        byte[] tagged = new byte[10 + id3 + 64];
        putAscii(tagged, 0, "ID3");
        tagged[3] = 3;
        tagged[6] = (byte) ((id3 >> 21) & 0x7F);
        tagged[7] = (byte) ((id3 >> 14) & 0x7F);
        tagged[8] = (byte) ((id3 >> 7) & 0x7F);
        tagged[9] = (byte) (id3 & 0x7F);
        System.arraycopy(header(3, 1, 9, 0, 0), 0, tagged, 10 + id3, 4);
        row("A-17", "mp3", 100L * 16000 + 10 + id3, tagged, "100 s of audio plus a 16000 byte tag: true 100 s");
        byte[] tiny = new byte[128];
        putAscii(tiny, 0, "ID3");
        tiny[3] = 3;
        System.arraycopy(header(3, 1, 9, 0, 0), 0, tiny, 10, 4);
        row("A-17", "mp3", 128000, tiny, "java test: id3TagIsSkipped (declared 128000 including the 10 byte tag)");

        // ---- A-27: MPEG-2 and MPEG-2.5 Layer III, and non Layer III
        byte[] m2 = new byte[200];
        System.arraycopy(header(2, 1, 9, 0, 0), 0, m2, 0, 4);
        row("A-27", "mp3", 300L * 10000, m2, "MPEG-2 22.05 kHz 80 kbps CBR, 300 s");
        byte[] m2x = new byte[200];
        System.arraycopy(header(2, 1, 9, 0, 0), 0, m2x, 0, 4);
        putAscii(m2x, 4 + 17, "Xing");
        putIntBE(m2x, 4 + 17 + 4, 1);
        putIntBE(m2x, 4 + 17 + 8, 20000);
        row("A-27", "mp3", 522L * 10000, m2x, "MPEG-2 22.05 kHz stereo Xing, 20000 frames: true 522 s");
        byte[] m25 = new byte[200];
        System.arraycopy(header(0, 1, 9, 0, 0), 0, m25, 0, 4);
        row("A-27", "mp3", 100L * 10000, m25, "MPEG-2.5 11.025 kHz 80 kbps CBR, 100 s");
        byte[] l2 = new byte[200];
        System.arraycopy(header(3, 2, 9, 0, 0), 0, l2, 0, 4);
        row("A-27", "mp3", 128000, l2, "MPEG-1 Layer II header");
        byte[] reserved = new byte[200];
        System.arraycopy(header(1, 1, 9, 0, 0), 0, reserved, 0, 4);
        row("A-27", "mp3", 128000, reserved, "reserved MPEG version bits (Java reads it as MPEG-2 tables)");

        // ---- A-28: arithmetic and bounds
        row("A-28", "wav", 0, wav(176400, 1000, Integer.MAX_VALUE - 15, true), "LIST chunk of 0x7FFFFFF0 bytes");
        // LIST chunk of 8 bytes at 12, so "fmt " starts at 28 and its byte rate (offset 44) is
        // cut off by the end of the 44 byte buffer: Java passes its bound check and reads past it.
        byte[] w2 = new byte[44];
        putAscii(w2, 0, "RIFF");
        putAscii(w2, 8, "WAVE");
        putAscii(w2, 12, "LIST");
        putInt(w2, 16, 8);
        putAscii(w2, 28, "fmt ");
        putInt(w2, 32, 16);
        row("A-28", "wav", 0, w2, "fmt chunk whose byte rate is cut off by the end of the data");
        row("A-28", "ogg", 1L << 40, ogg(1, 1, 64), "nominal bitrate 1 on a huge file: result does not fit an int");
        row("A-28", "ogg", 4_294_967_295L, ogg(1, 1, 64), "nominal bitrate 1 on a 4 GiB file: result does not fit an int");
    }

    // ------------------------------------------------------------------ archives and probe

    static byte[] cbr(int headerVersion, int bitrateIdx, int rateIdx, int bytes) {
        byte[] d = randomBytes(bytes);
        // keep the random payload free of anything that looks like a Xing/Info/VBRI tag
        byte[] h = header(headerVersion, 1, bitrateIdx, rateIdx, 0);
        System.arraycopy(h, 0, d, 0, 4);
        return d;
    }

    static byte[] zip(boolean stored, Map<String, byte[]> entries) throws IOException {
        return zip(stored, java.util.Set.of(), entries);
    }

    /** {@code alwaysStored}: names written with method 0 whatever their type. */
    static byte[] zip(boolean stored, java.util.Set<String> alwaysStored, Map<String, byte[]> entries) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (ZipOutputStream zip = new ZipOutputStream(bytes)) {
            for (Map.Entry<String, byte[]> e : entries.entrySet()) {
                ZipEntry entry = new ZipEntry(e.getKey());
                entry.setTime(1_700_000_000_000L);
                boolean store = alwaysStored.contains(e.getKey())
                        || stored && (e.getKey().toLowerCase().endsWith(".mp3") || e.getKey().toLowerCase().endsWith(".ogg"));
                if (store) {
                    entry.setMethod(ZipEntry.STORED);
                    entry.setSize(e.getValue().length);
                    entry.setCompressedSize(e.getValue().length);
                    CRC32 crc = new CRC32();
                    crc.update(e.getValue());
                    entry.setCrc(crc.getValue());
                }
                zip.putNextEntry(entry);
                zip.write(e.getValue());
                zip.closeEntry();
            }
        }
        return bytes.toByteArray();
    }

    static Map<String, byte[]> entries(Object... kv) {
        Map<String, byte[]> m = new LinkedHashMap<>();
        for (int i = 0; i < kv.length; i += 2) {
            m.put((String) kv[i], (byte[]) kv[i + 1]);
        }
        return m;
    }

    static final Map<String, byte[]> FILES = new ConcurrentHashMap<>();
    static int redirects;

    static HttpServer server() throws IOException {
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        int port = server.getAddress().getPort();
        server.createContext("/files/", exchange -> {
            // the real service answers 303 to a signed CDN url
            String key = exchange.getRequestURI().getPath().substring("/files/".length());
            exchange.getResponseHeaders().add("Location", "http://127.0.0.1:" + port + "/cdn/" + key + "?sign=abc");
            exchange.sendResponseHeaders(303, -1);
            exchange.close();
        });
        server.createContext("/cdn/", exchange -> {
            String key = exchange.getRequestURI().getPath().substring("/cdn/".length());
            byte[] body = FILES.get(key);
            if (body == null) {
                exchange.sendResponseHeaders(404, -1);
                exchange.close();
                return;
            }
            String range = exchange.getRequestHeaders().getFirst("Range");
            int start = 0;
            int end = body.length - 1;
            if (range != null && range.startsWith("bytes=")) {
                String spec = range.substring(6);
                if (spec.startsWith("-")) {
                    start = Math.max(0, body.length - Integer.parseInt(spec.substring(1)));
                } else {
                    String[] p = spec.split("-");
                    start = Integer.parseInt(p[0]);
                    if (p.length > 1 && !p[1].isEmpty()) {
                        end = Math.min(end, Integer.parseInt(p[1]));
                    }
                }
            }
            int len = Math.max(0, end - start + 1);
            exchange.getResponseHeaders().add("Content-Range", "bytes " + start + "-" + end + "/" + body.length);
            exchange.sendResponseHeaders(206, len == 0 ? -1 : len);
            try (OutputStream out = exchange.getResponseBody()) {
                out.write(body, start, len);
            }
        });
        server.start();
        return server;
    }

    static PrintWriter probeOut;

    /**
     * Probes one crafted file through the real {@code ChartDuration.probe} and records
     * {@code name, category, payload recipe, Java result}. A {@code null} payload is a key the
     * range server does not know (it answers 404).
     */
    static void probe(HttpServer server, String name, String category, String payloadDescription, byte[] payload,
                      Path dir) throws IOException {
        if (payload != null) {
            FILES.put(name, payload);
        }
        String url = "http://127.0.0.1:" + server.getAddress().getPort() + "/files/" + name;
        String result;
        try {
            Integer v = ChartDuration.probe(url);
            result = v == null ? "null" : v.toString();
        } catch (IOException e) {
            result = "IOException " + e.getMessage();
        } catch (Throwable t) {
            result = "EXC " + t.getClass().getSimpleName();
            if (category.equals("same")) {
                category = "exc";
            }
        }
        if (payloadDescription.startsWith("file:")) {
            Files.write(dir.resolve(payloadDescription.substring(5)), payload);
        }
        probeOut.println(name + "\t" + category + "\t" + payloadDescription + "\t" + Esc.esc(result));
    }

    static byte[] patchMethod(byte[] zip, int method) {
        byte[] d = zip.clone();
        for (int i = 0; i + 46 < d.length; i++) {
            if (d[i] == 'P' && d[i + 1] == 'K' && d[i + 2] == 1 && d[i + 3] == 2) {
                d[i + 10] = (byte) method;
                d[i + 11] = 0;
            }
            if (d[i] == 'P' && d[i + 1] == 'K' && d[i + 2] == 3 && d[i + 3] == 4) {
                d[i + 8] = (byte) method;
                d[i + 9] = 0;
            }
        }
        return d;
    }

    static void probeRows(Path dir) throws Exception {
        HttpServer server = server();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(dir.resolve("duration_probe.tsv"), StandardCharsets.UTF_8))) {
            probeOut = w;
            byte[] info = "name: x\nmusic: song.mp3\n".getBytes(StandardCharsets.UTF_8);
            byte[] illustration = randomBytes(3000);

            // ---- archives where Java is right: the deflate walk of the real service
            byte[] mp3 = cbr(3, 9, 0, 80_000);                 // 128 kbps, 44.1 kHz, 5 s
            probe(server, "zip_deflated_cbr", "same", "file:zip_deflated_cbr.zip",
                    zip(false, entries("info.yml", info, "illustration.png", illustration, "song.mp3", mp3)), dir);
            byte[] mp3b = cbr(3, 11, 1, 72_000);               // 192 kbps, 48 kHz, 3 s
            probe(server, "zip_deflated_cbr_192", "same", "file:zip_deflated_cbr_192.zip",
                    zip(false, entries("info.yml", info, "song.mp3", mp3b)), dir);
            byte[] oggBytes = ogg(1, 96000, 60_000);
            probe(server, "zip_deflated_ogg", "same", "file:zip_deflated_ogg.zip",
                    zip(false, entries("info.yml", info, "song.ogg", oggBytes)), dir);
            probe(server, "zip_upper_audio_last", "same", "file:zip_upper_audio_last.zip",
                    zip(false, entries("Chart.JSON", "{}".getBytes(StandardCharsets.UTF_8), "ILLUSTRATION.PNG", illustration,
                            "Music.MP3", mp3)), dir);
            probe(server, "zip_first_audio_wins", "same", "file:zip_first_audio_wins.zip",
                    zip(false, entries("a.ogg", oggBytes, "b.mp3", mp3)), dir);
            probe(server, "zip_no_audio", "same", "file:zip_no_audio.zip",
                    zip(false, entries("info.yml", info, "illustration.png", illustration)), dir);

            // ---- A-14: stored entries; anything but stored/deflate
            probe(server, "zip_stored_cbr", "A-14", "file:zip_stored_cbr.zip",
                    zip(true, entries("info.yml", info, "song.mp3", mp3)), dir);
            byte[] storedZip = zip(true, entries("info.yml", info, "song.mp3", mp3));
            // Java inflates the bytes as deflate, fails, and returns null; Rust fails explicitly: the same value
            probe(server, "zip_method_other", "same", "file:zip_method_other.zip", patchMethod(storedZip, 12), dir);

            // ---- A-02: BE Xing frame count, 9000 frames = 235 s, in a file that is physically tiny
            byte[] xing = new byte[2000];
            System.arraycopy(header(3, 1, 9, 0, 0), 0, xing, 0, 4);
            putAscii(xing, 4 + 32, "Xing");
            putIntBE(xing, 4 + 32 + 4, 0xF);
            putIntBE(xing, 4 + 32 + 8, 9000);
            probe(server, "zip_deflated_xing_be", "A-02", "file:zip_deflated_xing_be.zip",
                    zip(false, entries("info.yml", info, "song.mp3", xing)), dir);

            // ---- A-17: 40000 byte tag (over the first window) + 4 s of 128 kbps audio
            byte[] tagged = new byte[10 + 40_000 + 64_000];
            byte[] body = randomBytes(tagged.length);
            System.arraycopy(body, 0, tagged, 0, tagged.length);
            putAscii(tagged, 0, "ID3");
            tagged[3] = 3;
            tagged[4] = 0;
            tagged[5] = 0;
            tagged[6] = (byte) ((40_000 >> 21) & 0x7F);
            tagged[7] = (byte) ((40_000 >> 14) & 0x7F);
            tagged[8] = (byte) ((40_000 >> 7) & 0x7F);
            tagged[9] = (byte) (40_000 & 0x7F);
            System.arraycopy(header(3, 1, 9, 0, 0), 0, tagged, 10 + 40_000, 4);
            // the random "tag" body must not contain a sync before the real frame
            for (int i = 10; i < 10 + 40_000; i++) {
                if ((tagged[i] & 0xFF) == 0xFF) {
                    tagged[i] = 0x55;
                }
            }
            probe(server, "zip_deflated_id3_cover", "A-17", "file:zip_deflated_id3_cover.zip",
                    zip(false, entries("info.yml", info, "song.mp3", tagged)), dir);

            // ---- A-27: MPEG-2 22.05 kHz 80 kbps CBR, 4 s = 40000 bytes
            byte[] mpeg2 = cbr(2, 9, 0, 40_000);
            probe(server, "zip_deflated_mpeg2", "A-27", "file:zip_deflated_mpeg2.zip",
                    zip(false, entries("info.yml", info, "song.mp3", mpeg2)), dir);

            // ---- a real LAME encode behind a real 50 KB ID3v2 cover (made by make_real_audio.sh)
            Path cover = dir.resolve("real/lame_cbr128_cover_44k.head");
            if (Files.exists(cover)) {
                byte[] realMp3 = Files.readAllBytes(cover);
                probe(server, "zip_real_cover", "A-17", "file:zip_real_cover.zip",
                        zip(false, entries("info.yml", info, "song.mp3", realMp3)), dir);
            }

            // ---- N-31: macOS resource-fork stubs next to the audio
            byte[] appleDouble = new byte[4096];
            appleDouble[0] = 0x00;
            appleDouble[1] = 0x05;
            appleDouble[2] = 0x16;
            appleDouble[3] = 0x07;
            probe(server, "zip_macosx_stub_first", "N-31", "file:zip_macosx_stub_first.zip",
                    zip(false, entries("__MACOSX/._song.mp3", appleDouble, "song.mp3", mp3, "info.yml", info)), dir);
            probe(server, "zip_macosx_stub_after", "same", "file:zip_macosx_stub_after.zip",
                    zip(false, entries("song.mp3", mp3, "__MACOSX/._song.mp3", appleDouble)), dir);
            probe(server, "zip_dot_underscore_ogg", "N-31", "file:zip_dot_underscore_ogg.zip",
                    zip(false, entries("._song.ogg", appleDouble, "song.ogg", oggBytes)), dir);

            // ---- bare files (no archive)
            byte[] wavFile = new byte[40_000];
            byte[] wavHeader = wav(176400, 1_764_000, NO_LIST, false);
            System.arraycopy(wavHeader, 0, wavFile, 0, wavHeader.length);
            probe(server, "bare_wav", "same", "file:bare_wav.bin", wavFile, dir);
            byte[] bare = new byte[400 * 16_000];
            System.arraycopy(header(3, 1, 9, 0, 0), 0, bare, 0, 4);
            probe(server, "bare_cbr_400s", "A-18", "recipe:cbr128:400", bare, dir);
            byte[] bareOgg = ogg(1, 128000, 100_000);
            probe(server, "bare_ogg", "A-18", "file:bare_ogg.bin", bareOgg, dir);
            byte[] bareXing = new byte[1_000_000];
            System.arraycopy(xing, 0, bareXing, 0, xing.length);
            probe(server, "bare_xing_be", "A-02", "recipe:xing_be_9000:1000000", bareXing, dir);
            byte[] tiny = new byte[200];
            System.arraycopy(header(3, 1, 9, 0, 0), 0, tiny, 0, 4);
            probe(server, "bare_tiny_cbr", "same", "recipe:cbr128:tiny200", tiny, dir);
            probe(server, "bare_garbage", "same", "recipe:garbage",
                    "this is not audio".getBytes(StandardCharsets.UTF_8), dir);
            probe(server, "bare_three_bytes", "A-28", "recipe:three_bytes", new byte[]{1, 2, 3}, dir);

            // ---- the CDN does not know the key
            probe(server, "missing_never_stored", "same", "recipe:404", null, dir);

            // ---- O-07: the archive's info file names the song. Java takes the first .mp3/.ogg entry,
            //      here a 2 s preview ahead of the 5 s song. Written last so the files above keep
            //      their bytes.
            byte[] preview = cbr(3, 9, 0, 32_000);             // 128 kbps, 2 s
            byte[] songOgg = ogg(1, 96000, 60_000);             // 5 s
            byte[] wavSong = randomBytes(3000);
            byte[] garbage = randomBytes(300);
            for (int i = 0; i < garbage.length; i++) {
                if (garbage[i] == ':') {
                    garbage[i] = '.';
                }
            }
            byte[] ymlSong = "name: x\nmusic: song.mp3\nchart: c.json\n".getBytes(StandardCharsets.UTF_8);
            byte[] txtSong = "#\nName: x\nSong: song.mp3\n".getBytes(StandardCharsets.UTF_8);
            byte[] empty = new byte[0];

            probe(server, "zip_song_second_yml", "O-07", "file:zip_song_second_yml.zip",
                    zip(false, entries("info.yml", ymlSong, "preview.mp3", preview, "song.mp3", mp3)), dir);
            probe(server, "zip_song_second_txt", "O-07", "file:zip_song_second_txt.zip",
                    zip(false, entries("info.txt", txtSong, "preview.mp3", preview, "song.mp3", mp3)), dir);
            probe(server, "zip_song_second_both", "O-07", "file:zip_song_second_both.zip",
                    zip(false, entries("info.txt", "#\nMusic: preview.mp3\n".getBytes(StandardCharsets.UTF_8),
                            "info.yml", ymlSong, "preview.mp3", preview, "song.mp3", mp3)), dir);
            probe(server, "zip_song_folder", "O-07", "file:zip_song_folder.zip",
                    zip(false, entries("chart/", empty, "chart/info.yml", ymlSong, "chart/preview.mp3", preview,
                            "chart/song.mp3", mp3, "__MACOSX/chart/._info.yml", appleDouble)), dir);
            probe(server, "zip_song_case_insensitive", "O-07", "file:zip_song_case_insensitive.zip",
                    zip(false, entries("info.yml", "music: SONG.mp3\n".getBytes(StandardCharsets.UTF_8),
                            "preview.mp3", preview, "Song.Mp3", mp3)), dir);
            probe(server, "zip_song_quoted_bom_crlf", "O-07", "file:zip_song_quoted_bom_crlf.zip",
                    zip(false, entries("info.yml",
                            "\uFEFFname: x\r\nmusic: \"song.mp3\"   # the song\r\nchart: c.json\r\n".getBytes(StandardCharsets.UTF_8),
                            "preview.mp3", preview, "song.mp3", mp3)), dir);
            probe(server, "zip_song_info_stored", "O-07", "file:zip_song_info_stored.zip",
                    zip(false, java.util.Set.of("info.yml"),
                            entries("info.yml", ymlSong, "preview.mp3", preview, "song.mp3", mp3)), dir);
            probe(server, "zip_song_ogg", "O-07", "file:zip_song_ogg.zip",
                    zip(false, entries("info.yml", "music: song.ogg\n".getBytes(StandardCharsets.UTF_8),
                            "preview.mp3", preview, "song.ogg", songOgg)), dir);
            // naming a file that is there but is not mp3/ogg: Java measures the preview, Rust fails
            probe(server, "zip_info_names_wav", "O-07", "file:zip_info_names_wav.zip",
                    zip(false, entries("info.yml", "music: song.wav\n".getBytes(StandardCharsets.UTF_8),
                            "preview.mp3", preview, "song.wav", wavSong)), dir);
            // what the info file cannot tell: the first audio entry, as before (Java is right)
            probe(server, "zip_info_names_missing_file", "same", "file:zip_info_names_missing_file.zip",
                    zip(false, entries("info.yml", "music: gone.mp3\n".getBytes(StandardCharsets.UTF_8),
                            "preview.mp3", preview, "song.mp3", mp3)), dir);
            probe(server, "zip_info_no_music", "same", "file:zip_info_no_music.zip",
                    zip(false, entries("info.yml", "name: x\ndifficulty: 3\n".getBytes(StandardCharsets.UTF_8),
                            "preview.mp3", preview, "song.mp3", mp3)), dir);
            probe(server, "zip_info_garbage", "same", "file:zip_info_garbage.zip",
                    zip(false, entries("info.yml", garbage, "preview.mp3", preview, "song.mp3", mp3)), dir);
            probe(server, "zip_no_info_preview_first", "same", "file:zip_no_info_preview_first.zip",
                    zip(false, entries("preview.mp3", preview, "song.mp3", mp3)), dir);
            // an info file over the 64 KiB cap is not read
            StringBuilder big = new StringBuilder();
            while (big.length() < 70_000) {
                big.append("# padding padding padding padding padding\n");
            }
            big.append("music: song.mp3\n");
            probe(server, "zip_info_too_big", "same", "file:zip_info_too_big.zip",
                    zip(false, entries("info.yml", big.toString().getBytes(StandardCharsets.UTF_8),
                            "preview.mp3", preview, "song.mp3", mp3)), dir);
        } finally {
            probeOut = null;
            server.stop(0);
        }
    }

    /**
     * What the probe does with the audio of an entry, over the head bytes of a real encoder's file:
     * the first window, then the large one (the probe fetches them with Range requests, here they
     * are cut from the recorded head). {@code total} is the size of the whole file, which a ZIP
     * entry reports as its uncompressed size.
     */
    static Integer javaReal(String kind, byte[] head, long total) {
        if (kind.equals("wav")) {
            return ChartDuration.parseWavDuration(head);
        }
        for (int window : new int[]{32 * 1024, 512 * 1024}) {
            byte[] audio = java.util.Arrays.copyOf(head, Math.min(head.length, window));
            Integer v = kind.equals("ogg") ? ChartDuration.parseOggDuration(audio, total)
                    : ChartDuration.parseMp3Duration(audio, total);
            if (v != null) {
                return v;
            }
        }
        return null;
    }

    /**
     * {@code real_audio.tsv}: the Java result for the files of {@code real/manifest.tsv} (written by
     * make_real_audio.sh from real LAME and ffmpeg encodes): name, kind, total size, ffprobe duration
     * in ms, Java result. Skipped when the manifest is absent.
     */
    static void realRows(Path dir) throws IOException {
        Path real = dir.resolve("real");
        Path manifest = real.resolve("manifest.tsv");
        if (!Files.exists(manifest)) {
            return;
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(dir.resolve("real_audio.tsv"), StandardCharsets.UTF_8))) {
            for (String line : Files.readAllLines(manifest, StandardCharsets.UTF_8)) {
                String[] f = line.split("\t");
                byte[] head = Files.readAllBytes(real.resolve(f[0] + ".head"));
                String result;
                try {
                    Integer v = javaReal(f[1], head, Long.parseLong(f[2]));
                    result = v == null ? "null" : v.toString();
                } catch (Throwable t) {
                    result = "EXC " + t.getClass().getSimpleName();
                }
                w.println(f[0] + "\t" + f[1] + "\t" + f[2] + "\t" + f[3] + "\t" + result);
            }
        }
    }

    public static void main(String[] args) throws Exception {
        // Never hang: a daemon watchdog ends the JVM if anything blocks (a socket, the HTTP client).
        Thread watchdog = new Thread(() -> {
            try {
                Thread.sleep(120_000);
            } catch (InterruptedException e) {
                return;
            }
            System.err.println("GenDuration: watchdog fired, a step blocked for 120 s");
            Runtime.getRuntime().halt(3);
        }, "watchdog");
        watchdog.setDaemon(true);
        watchdog.start();

        Path out = Path.of(args[0]);
        Files.createDirectories(out);
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("duration_parse.tsv"), StandardCharsets.UTF_8))) {
            parse = w;
            parserRows();
        }
        probeRows(out);
        realRows(out);
        System.exit(0);
    }
}
