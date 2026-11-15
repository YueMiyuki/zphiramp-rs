import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import io.netty.channel.embedded.EmbeddedChannel;
import top.rymc.phira.protocol.PacketRegistry;
import top.rymc.phira.protocol.codec.Encodeable;
import top.rymc.phira.protocol.codec.encoder.ClientPacketEncoder;
import top.rymc.phira.protocol.codec.encoder.FrameEncoder;
import top.rymc.phira.protocol.codec.encoder.ServerPacketEncoder;
import top.rymc.phira.protocol.data.RoomInfo;
import top.rymc.phira.protocol.data.monitor.touch.TouchFrame;
import top.rymc.phira.protocol.data.state.GameState;
import top.rymc.phira.protocol.data.message.Message;
import top.rymc.phira.protocol.data.monitor.judge.Judgement;
import top.rymc.phira.protocol.packet.ClientBoundPacket;
import top.rymc.phira.protocol.packet.ServerBoundPacket;
import top.rymc.phira.protocol.util.Float16Util;
import top.rymc.phira.protocol.util.NettyPacketUtil;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * Records what the real jphira-mp-protocol 2.2.1 (running on the Netty 4.2.3 the server ships with)
 * does, as fixture files for the Rust tests. See {@code regen.sh}.
 */
public final class GenFixtures {
    private GenFixtures() {}

    // ------------------------------------------------------------ codec shorthands

    static byte[] encSB(ServerBoundPacket p) {
        return Util.bytes(PacketRegistry.ServerBound.encode(p, Unpooled::buffer));
    }

    static byte[] encCB(ClientBoundPacket p) {
        return Util.bytes(PacketRegistry.ClientBound.encode(p, Unpooled::buffer));
    }

    static byte[] rtSB(byte[] in) {
        return encSB(PacketRegistry.ServerBound.decode(Util.wrap(in)));
    }

    static byte[] rtCB(byte[] in) {
        return encCB(PacketRegistry.ClientBound.decode(Util.wrap(in)));
    }

    static byte[] encode(Encodeable e) {
        ByteBuf b = Unpooled.buffer();
        e.encode(b);
        return Util.bytes(b);
    }

    interface Fn {
        byte[] apply(byte[] in) throws Throwable;
    }

    /** {@code K:<crc of the re-encoded value>} or a one-letter failure code. */
    static String outcomeCode(Fn f, byte[] in) {
        try {
            return "K:" + Util.crc(f.apply(in));
        } catch (Throwable t) {
            return Util.code(t);
        }
    }

    /** {@code ok<TAB>hex} or {@code err<TAB>class<TAB>message}. */
    static String outcomeFull(Fn f, byte[] in) {
        try {
            return "ok\t" + Util.hex(f.apply(in));
        } catch (Throwable t) {
            return "err\t" + Util.describe(t);
        }
    }

    /** Hex of the re-encoded value, or {@code ERR:class:message} when the real decoder refuses the input. */
    static String rtOrErr(Fn f, byte[] in) {
        try {
            return Util.hex(f.apply(in));
        } catch (Throwable t) {
            return "ERR:" + Util.describe(t).replace('\t', ':');
        }
    }

    /** The bytes the real {@code FrameEncoder} + packet encoder put on the wire. */
    static byte[] framed(boolean serverBound, Object packet) {
        EmbeddedChannel ch = serverBound
                ? new EmbeddedChannel(new FrameEncoder(), new ClientPacketEncoder())
                : new EmbeddedChannel(new FrameEncoder(), new ServerPacketEncoder());
        ch.writeOutbound(packet);
        List<byte[]> parts = new ArrayList<>();
        ByteBuf b;
        while ((b = ch.readOutbound()) != null) {
            parts.add(Util.bytes(b));
            b.release();
        }
        ch.finish();
        return Util.concat(parts.toArray(new byte[0][]));
    }

    static byte[] framedRaw(byte[] payload) {
        EmbeddedChannel ch = new EmbeddedChannel(new FrameEncoder());
        ch.writeOutbound(Unpooled.wrappedBuffer(payload));
        List<byte[]> parts = new ArrayList<>();
        ByteBuf b;
        while ((b = ch.readOutbound()) != null) {
            parts.add(Util.bytes(b));
            b.release();
        }
        ch.finish();
        return Util.concat(parts.toArray(new byte[0][]));
    }

    static byte[] varint(int v) {
        ByteBuf b = Unpooled.buffer();
        NettyPacketUtil.encodeVarInt(b, v);
        return Util.bytes(b);
    }

    /** Hex of a framed payload given as hex: length prefix first. */
    static String f(String payloadHex) {
        byte[] p = Util.unhex(payloadHex.replace(" ", ""));
        return Util.hex(Util.concat(varint(p.length), p));
    }

    static String le(int v) {
        return String.format("%02x%02x%02x%02x", v & 0xff, (v >>> 8) & 0xff, (v >>> 16) & 0xff, (v >>> 24) & 0xff);
    }

    static String str(String s) {
        ByteBuf b = Unpooled.buffer();
        NettyPacketUtil.encodeString(b, s);
        return Util.hex(Util.bytes(b));
    }

    static String h(String hex) {
        return hex.replace(" ", "");
    }

    public static void main(String[] args) throws Exception {
        Path dir = Path.of(args[0]);
        Files.createDirectories(dir);
        registry(dir);
        packets(dir);
        types(dir);
        prefixes(dir);
        decodeCases(dir);
        fuzz(dir);
        varints(dir);
        strings(dir);
        utf8Lossy(dir);
        float16(dir);
        streams(dir);
        System.err.println("done");
    }

    // ------------------------------------------------------------ registry

    static void registry(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "registry.tsv")) {
            for (PacketRegistry.ServerBound e : PacketRegistry.ServerBound.values()) {
                o.line("sb", e.name(), String.format("%02x", e.getId()), e.getClazz().getSimpleName());
            }
            for (PacketRegistry.ClientBound e : PacketRegistry.ClientBound.values()) {
                o.line("cb", e.name(), String.format("%02x", e.getId()), e.getClazz().getSimpleName());
            }
        }
    }

    // ------------------------------------------------------------ packets

    static void packets(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "packets.tsv")) {
            o.raw("# name\tpacket hex (id + payload)\tdecode-then-encode hex (or ERR:class:message when Java refuses to decode it)\tframed hex (length prefix + packet)");
            for (Vectors.Named<ServerBoundPacket> v : Vectors.SB) {
                byte[] e = encSB(v.value());
                o.line("sb." + v.name(), Util.hex(e), rtOrErr(GenFixtures::rtSB, e), Util.hex(framed(true, v.value())));
            }
            for (Vectors.Named<ClientBoundPacket> v : Vectors.CB) {
                byte[] e = encCB(v.value());
                o.line("cb." + v.name(), Util.hex(e), rtOrErr(GenFixtures::rtCB, e), Util.hex(framed(false, v.value())));
            }
        }
    }

    static void types(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "types.tsv")) {
            o.raw("# name\tencoded hex\tdecode-then-encode hex");
            for (Vectors.TypeVec v : Vectors.TYPES) {
                byte[] e = encode(v.value());
                String rt;
                try {
                    rt = Util.hex(encode(v.decode().apply(Util.wrap(e))));
                } catch (Throwable t) {
                    rt = "ERR " + Util.describe(t).replace('\t', ' ');
                }
                o.line(v.name(), Util.hex(e), rt);
            }
        }
    }

    // ------------------------------------------------------------ truncation

    static void prefixes(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "prefixes.tsv")) {
            o.raw("# name\tone token per strict prefix (length 0..n-1) of the packet hex: K:<crc> decoded fine, else failure code (see Util.code)");
            for (Vectors.Named<ServerBoundPacket> v : Vectors.SB) {
                byte[] e = encSB(v.value());
                List<String> toks = new ArrayList<>();
                for (int n = 0; n < e.length; n++) {
                    toks.add(outcomeCode(GenFixtures::rtSB, java.util.Arrays.copyOf(e, n)));
                }
                o.line("sb." + v.name(), String.join(" ", toks));
            }
            for (Vectors.Named<ClientBoundPacket> v : Vectors.CB) {
                byte[] e = encCB(v.value());
                List<String> toks = new ArrayList<>();
                for (int n = 0; n < e.length; n++) {
                    toks.add(outcomeCode(GenFixtures::rtCB, java.util.Arrays.copyOf(e, n)));
                }
                o.line("cb." + v.name(), String.join(" ", toks));
            }
        }
    }

    // ------------------------------------------------------------ hand-made malformed / non-canonical inputs

    static void decodeCases(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "decode.tsv")) {
            o.raw("# kind\tname\tinput hex\tok<TAB>re-encoded hex | err<TAB>java class<TAB>message");
            Fn sb = GenFixtures::rtSB;
            Fn cb = GenFixtures::rtCB;
            Fn state = in -> encode(GameState.decode(Util.wrap(in)));
            Fn msg = in -> encode(Message.decode(Util.wrap(in)));
            Fn judgement = in -> encode(Judgement.decode(Util.wrap(in)));
            Fn room = in -> encode(RoomInfo.decode(Util.wrap(in)));
            Fn frame = in -> encode(TouchFrame.decode(Util.wrap(in)));

            // --- serverbound packets
            for (String id : new String[]{"10", "11", "7f", "80", "ff", "20"}) {
                o.line("sb", "unknown_id_" + id, id, outcomeFull(sb, Util.unhex(id)));
            }
            o.line("sb", "empty", "", outcomeFull(sb, new byte[0]));
            o.line("sb", "ping_trailing", "00 aa bb".replace(" ", ""), outcomeFull(sb, Util.unhex("00aabb")));
            o.line("sb", "leave_trailing", "07 ffff", outcomeFull(sb, Util.unhex("07ffff")));
            sbCase(o, "auth_len32", "01 20" + "61".repeat(32));
            sbCase(o, "auth_len33", "01 21" + "61".repeat(33));
            sbCase(o, "auth_len_neg", "01 ffffffff0f");
            sbCase(o, "auth_len_neg_min", "01 8080808008");
            sbCase(o, "auth_len_bad_varint", "01 ffffffffff");
            sbCase(o, "auth_len_missing", "01");
            sbCase(o, "auth_len_truncated_varint", "01 ff");
            sbCase(o, "auth_short_body", "01 05 616263");
            sbCase(o, "auth_zero_len", "01 00");
            sbCase(o, "auth_invalid_utf8", "01 02 c328");
            sbCase(o, "auth_invalid_utf8_lone_cont", "01 03 80 41 bf");
            sbCase(o, "auth_overlong_slash", "01 02 c0af");
            sbCase(o, "auth_surrogate_utf8", "01 03 eda080");
            sbCase(o, "auth_truncated_seq", "01 02 e282");
            sbCase(o, "auth_utf8_bom", "01 03 efbbbf");
            sbCase(o, "auth_noncanonical_len", "01 8100 41");
            sbCase(o, "chat_len200", "02 c801" + "61".repeat(200));
            sbCase(o, "chat_len201", "02 c901" + "61".repeat(201));
            sbCase(o, "chat_len_5byte_canonical", "02 8180808000 61");
            sbCase(o, "create_len20", "05 14" + "61".repeat(20));
            sbCase(o, "create_len21", "05 15" + "61".repeat(21));
            sbCase(o, "join_bool2", "06 01 61 02");
            sbCase(o, "join_boolff", "06 01 61 ff");
            sbCase(o, "join_missing_bool", "06 01 61");
            sbCase(o, "lock_bool2", "08 02");
            sbCase(o, "lock_boolff", "08 ff");
            sbCase(o, "lock_missing", "08");
            sbCase(o, "cycle_bool80", "09 80");
            sbCase(o, "select_short", "0a 010203");
            sbCase(o, "select_long", "0a 0102030405");
            sbCase(o, "played_short", "0e");
            sbCase(o, "touches_neg_count", "03 ffffffff0f");
            sbCase(o, "touches_neg_count_min", "03 8080808008");
            sbCase(o, "touches_huge_count", "03 ffffff7f");
            sbCase(o, "touches_max_count", "03 ffffffff07");
            sbCase(o, "touches_count_missing", "03");
            sbCase(o, "touches_count_bad_varint", "03 ffffffffff");
            sbCase(o, "touches_count1_missing_frame", "03 01");
            sbCase(o, "touches_count2_one_frame", "03 02 00000000 00");
            sbCase(o, "touches_frame_short_time", "03 01 000000");
            sbCase(o, "touches_point_short", "03 01 00000000 01 05 0000");
            sbCase(o, "touches_nan_half", "03 01 00000000 01 05 017e 017c");
            sbCase(o, "touches_snan_half_payload", "03 01 00000000 01 05 ff7f ff7d");
            sbCase(o, "touches_subnormal_half", "03 01 00000000 01 05 0100 ff03");
            sbCase(o, "touches_noncanonical_count", "03 8100 00000000 00");
            sbCase(o, "judges_neg_count", "04 ffffffff0f");
            sbCase(o, "judges_count1_short", "04 01 0000000000000000");
            sbCase(o, "judges_unknown_judgement_6", "04 01 00000000 01000000 02000000 06");
            sbCase(o, "judges_unknown_judgement_ff", "04 01 00000000 01000000 02000000 ff");
            sbCase(o, "judges_unknown_judgement_80", "04 01 00000000 01000000 02000000 80");
            sbCase(o, "judges_ok_hold_good", "04 01 0000803f 01000000 02000000 05");
            sbCase(o, "judges_first_ok_second_bad", "04 02 00000000 01000000 02000000 00 00000000 01000000 02000000 09");
            sbCase(o, "judges_snan_time", "04 01 010080 7f 01000000 02000000 00".replace(" ", ""));

            // --- clientbound packets
            for (String id : new String[]{"14", "15", "7f", "80", "ff"}) {
                o.line("cb", "unknown_id_" + id, id, outcomeFull(cb, Util.unhex(id)));
            }
            o.line("cb", "empty", "", outcomeFull(cb, new byte[0]));
            cbCase(o, "pong_trailing", "00 01");
            cbCase(o, "auth_failed", "01 00 03 616263");
            cbCase(o, "auth_failed_trunc", "01 00 03 6162");
            cbCase(o, "auth_failed_missing_msg", "01 00");
            cbCase(o, "auth_success_bool2", "01 02 01000000 01 61 00 00");
            String authHead = "01 01 01000000 0161 00 01"; // id, success, user 1 "a" (not monitor), has room info
            String roomHead = "0172 00 00 00 00 00 00 00";  // room "r", SelectChart(none), five false flags
            cbCase(o, "auth_success_room_flag2", "01 01 01000000 0161 00 02 " + roomHead + " 00");
            cbCase(o, "auth_success_no_room", "01 01 01000000 0161 00 00");
            cbCase(o, "auth_success_missing_room_flag", "01 01 01000000 0161 00");
            cbCase(o, "auth_success_monitor_bool5", "01 01 01000000 0161 05 00");
            cbCase(o, "auth_room_minimal", authHead + roomHead + "00");
            cbCase(o, "auth_room_id_len20", authHead + "14" + "61".repeat(20) + "00 00 00 00 00 00 00 00");
            cbCase(o, "auth_room_id_len21", authHead + "15" + "61".repeat(21) + "00 00 00 00 00 00 00 00");
            cbCase(o, "auth_room_users_neg", authHead + roomHead + "ffffffff0f");
            cbCase(o, "auth_room_users_neg_then_junk", authHead + roomHead + "8080808008 aabbccdd");
            cbCase(o, "auth_room_users_key_mismatch", authHead + roomHead + "01 09000000 02000000 0162 01");
            cbCase(o, "auth_room_users_key_match", authHead + roomHead + "01 02000000 02000000 0162 01");
            cbCase(o, "auth_room_users_key_missing", authHead + roomHead + "01");
            cbCase(o, "auth_room_users_count_gt_data", authHead + roomHead + "03 01000000 01000000 0161 00");
            cbCase(o, "auth_room_users_huge_count", authHead + roomHead + "ffffff7f");
            cbCase(o, "auth_room_users_count_missing", authHead + roomHead);
            cbCase(o, "auth_room_state_unknown", authHead + "0172 05 00 00 00 00 00 00");
            cbCase(o, "auth_room_select_chart_flag2", authHead + "0172 00 02 2a000000 00 00 00 00 00 00");
            cbCase(o, "auth_room_bools_ff", authHead + "0172 02 ff ff 7f 80 01 00");
            cbCase(o, "auth_room_trailing", authHead + roomHead + "00 aabbcc");
            cbCase(o, "chat_success", "02 01");
            cbCase(o, "chat_success_bool2", "02 02 aabb");
            cbCase(o, "chat_failed", "02 00 02 6162");
            cbCase(o, "chat_failed_len_bad", "02 00 ffffffff0f");
            cbCase(o, "chat_failed_invalid_utf8", "02 00 02 c328");
            cbCase(o, "chat_missing", "02");
            cbCase(o, "touches_short_id", "03 010203");
            cbCase(o, "touches_ok", "03 07000000 01 00000000 01 05 0034 00b8");
            cbCase(o, "touches_neg_count", "03 07000000 ffffffff0f");
            cbCase(o, "judges_unknown_judgement", "04 07000000 01 00000000 01000000 02000000 06");
            cbCase(o, "message_unknown_10", "05 10");
            cbCase(o, "message_unknown_ff", "05 ff");
            cbCase(o, "message_unknown_80", "05 80");
            cbCase(o, "message_missing", "05");
            cbCase(o, "message_chat_short", "05 00 01000000");
            cbCase(o, "message_played_bool2", "05 0b 01000000 02000000 0000803f 02");
            cbCase(o, "message_lock_bool3", "05 0e 03");
            cbCase(o, "message_startplaying_trailing", "05 0a 11 22");
            cbCase(o, "message_gameend_trailing", "05 0c 11 22");
            cbCase(o, "state_unknown_3", "06 03");
            cbCase(o, "state_unknown_ff", "06 ff");
            cbCase(o, "state_unknown_80", "06 80");
            cbCase(o, "state_select_none", "06 00 00");
            cbCase(o, "state_select_flag2", "06 00 02 2a000000");
            cbCase(o, "state_select_flag_missing", "06 00");
            cbCase(o, "state_select_some_short", "06 00 01 2a00");
            cbCase(o, "state_select_none_trailing", "06 00 00 2a000000");
            cbCase(o, "state_wait_trailing", "06 01 aa");
            cbCase(o, "host_bool2", "07 02");
            cbCase(o, "join_failed", "09 00 01 61");
            cbCase(o, "join_success_bool2", "09 02 01 00 01 02");
            cbCase(o, "join_success_users_neg", "09 01 01 ffffffff0f 00");
            cbCase(o, "join_success_users_huge", "09 01 01 ffffff7f");
            cbCase(o, "join_success_state_unknown", "09 01 07 00 00");
            cbCase(o, "join_success_missing_live", "09 01 01 00");
            cbCase(o, "onjoin_ok", "0a 01000000 01 61 02");
            cbCase(o, "onjoin_short", "0a 01000000 01 61");
            cbCase(o, "leave_ok", "0b 01 ff");
            cbCase(o, "abort_failed", "13 00 03 616263");

            // --- sub-structures
            for (String s : new String[]{"03", "04", "ff", "80", "7f"}) {
                o.line("state", "unknown_" + s, s, outcomeFull(state, Util.unhex(s)));
            }
            o.line("state", "empty", "", outcomeFull(state, new byte[0]));
            for (String s : new String[]{"0f", "10", "11", "ff", "80", "7f"}) {
                o.line("message", "id_" + s, s, outcomeFull(msg, Util.unhex(s)));
            }
            o.line("message", "empty", "", outcomeFull(msg, new byte[0]));
            for (String s : new String[]{"06", "07", "ff", "80", "7f", "fa"}) {
                o.line("judgement", "id_" + s, s, outcomeFull(judgement, Util.unhex(s)));
            }
            o.line("judgement", "empty", "", outcomeFull(judgement, new byte[0]));
            o.line("roominfo", "min", h("01 72 01 00 00 00 00 00 00"), outcomeFull(room, Util.unhex(h("01 72 01 00 00 00 00 00 00"))));
            o.line("roominfo", "id_len20", h("14" + "61".repeat(20) + "01 00 00 00 00 00 00"), outcomeFull(room, Util.unhex(h("14" + "61".repeat(20) + "01 00 00 00 00 00 00"))));
            o.line("roominfo", "id_len21", h("15" + "61".repeat(21) + "01 00 00 00 00 00 00"), outcomeFull(room, Util.unhex(h("15" + "61".repeat(21) + "01 00 00 00 00 00 00"))));
            o.line("roominfo", "trailing_ignored", h("01 72 01 00 00 00 00 00 00 aa"), outcomeFull(room, Util.unhex(h("01 72 01 00 00 00 00 00 00 aa"))));
            o.line("frame", "no_points", h("0000803f 00"), outcomeFull(frame, Util.unhex(h("0000803f 00"))));
            o.line("frame", "truncated_time", h("0000"), outcomeFull(frame, Util.unhex(h("0000"))));
        }
    }

    static void sbCase(Util.Out o, String name, String hex) {
        o.line("sb", name, h(hex), outcomeFull(GenFixtures::rtSB, Util.unhex(h(hex))));
    }

    static void cbCase(Util.Out o, String name, String hex) {
        o.line("cb", name, h(hex), outcomeFull(GenFixtures::rtCB, Util.unhex(h(hex))));
    }

    // ------------------------------------------------------------ differential fuzz

    static final int[] INTERESTING = {0x00, 0x01, 0x7f, 0x80, 0xff, 0xfe, 0x0f};
    static final int FUZZ_PACKETS = 6000;
    static final int FUZZ_STREAMS = 1500;
    static final long SEED_SB = 0x5B01L;
    static final long SEED_CB = 0xCB01L;
    static final long SEED_STREAM_SB = 0x57AEA01L;
    static final long SEED_STREAM_CB = 0x57AEA02L;

    /**
     * The mutation scheme is part of the contract with the Rust test: both sides draw from
     * splitmix64 in exactly this order.
     */
    static byte[] mutate(Util.Rng r, byte[] base, int ops) {
        byte[] b = base.clone();
        int len = b.length;
        for (int k = 0; k < ops; k++) {
            int op = r.below(5);
            switch (op) {
                case 0 -> {
                    if (len > 0) {
                        int i = r.below(len);
                        b[i] = (byte) r.next();
                    }
                }
                case 1 -> {
                    if (len > 0) {
                        len = r.below(len + 1);
                    }
                }
                case 2 -> {
                    int i = r.below(len + 1);
                    byte v = (byte) r.next();
                    byte[] nb = new byte[len + 1];
                    System.arraycopy(b, 0, nb, 0, i);
                    nb[i] = v;
                    System.arraycopy(b, i, nb, i + 1, len - i);
                    b = nb;
                    len++;
                }
                case 3 -> {
                    if (len > 0) {
                        int i = r.below(len);
                        System.arraycopy(b, i + 1, b, i, len - i - 1);
                        len--;
                    }
                }
                default -> {
                    if (len > 0) {
                        int i = r.below(len);
                        b[i] = (byte) INTERESTING[r.below(INTERESTING.length)];
                    }
                }
            }
        }
        return java.util.Arrays.copyOf(b, len);
    }

    static void fuzz(Path dir) throws IOException {
        List<byte[]> sbBases = new ArrayList<>();
        for (Vectors.Named<ServerBoundPacket> v : Vectors.SB) sbBases.add(encSB(v.value()));
        List<byte[]> cbBases = new ArrayList<>();
        for (Vectors.Named<ClientBoundPacket> v : Vectors.CB) cbBases.add(encCB(v.value()));

        try (Util.Out o = new Util.Out(dir, "fuzz_sb.txt")) {
            Util.Rng r = new Util.Rng(SEED_SB);
            for (int i = 0; i < FUZZ_PACKETS; i++) {
                byte[] base = sbBases.get(r.below(sbBases.size()));
                byte[] in = mutate(r, base, 1 + r.below(3));
                o.raw(outcomeCode(GenFixtures::rtSB, in));
            }
        }
        try (Util.Out o = new Util.Out(dir, "fuzz_cb.txt")) {
            Util.Rng r = new Util.Rng(SEED_CB);
            for (int i = 0; i < FUZZ_PACKETS; i++) {
                byte[] base = cbBases.get(r.below(cbBases.size()));
                byte[] in = mutate(r, base, 1 + r.below(3));
                o.raw(outcomeCode(GenFixtures::rtCB, in));
            }
        }

        List<byte[]> sbFrames = new ArrayList<>();
        for (Vectors.Named<ServerBoundPacket> v : Vectors.SB) sbFrames.add(framed(true, v.value()));
        List<byte[]> cbFrames = new ArrayList<>();
        for (Vectors.Named<ClientBoundPacket> v : Vectors.CB) cbFrames.add(framed(false, v.value()));
        streamFuzz(dir, "stream_fuzz_sb.txt", SEED_STREAM_SB, sbFrames, true);
        streamFuzz(dir, "stream_fuzz_cb.txt", SEED_STREAM_CB, cbFrames, false);
    }

    /** Builds the byte stream of stream-fuzz case; also used by the Rust test through the same recipe. */
    static void streamFuzz(Path dir, String file, long seed, List<byte[]> frames, boolean server) throws IOException {
        try (Util.Out o = new Util.Out(dir, file)) {
            Util.Rng r = new Util.Rng(seed);
            for (int i = 0; i < FUZZ_STREAMS; i++) {
                int k = 1 + r.below(4);
                byte[] stream = new byte[0];
                for (int j = 0; j < k; j++) {
                    stream = Util.concat(stream, frames.get(r.below(frames.size())));
                }
                stream = mutate(r, stream, r.below(4));
                List<byte[]> chunks = new ArrayList<>();
                int pos = 0;
                while (pos < stream.length) {
                    int remaining = stream.length - pos;
                    int n = 1 + r.below(Math.min(remaining, 1 + r.below(64)));
                    chunks.add(java.util.Arrays.copyOfRange(stream, pos, pos + n));
                    pos += n;
                }
                o.raw(Streams.run(server, false, chunks).summary());
            }
        }
    }

    // ------------------------------------------------------------ scalars

    static void varints(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "varint.tsv")) {
            o.raw("# enc\tvalue (signed int)\thex  |  dec\thex\tvalue:bytes consumed | err<TAB>class<TAB>message  |  frame\tpayload length\tprefix hex");
            int[] values = {0, 1, 2, 63, 64, 127, 128, 129, 255, 256, 300, 16383, 16384, 16385, 100000, 2097151, 2097152,
                    268435455, 268435456, 1234567890, Integer.MAX_VALUE, -1, -2, -128, -129, -300, Integer.MIN_VALUE + 1,
                    Integer.MIN_VALUE, 0x12345678, -0x12345678};
            for (int v : values) {
                o.line("enc", Integer.toString(v), Util.hex(varint(v)));
            }
            String[] decodes = {
                    "00", "01", "7f", "8001", "ff7f", "808001", "80808001", "8080808001", "ffffffff07", "ffffffff0f",
                    "ffffffff7f", "8080808008", "8080808010", "8080808070", "ac02", "8000", "808000", "8180808000",
                    "00ff", "7fffff", "ac02ff", "", "80", "ff", "ffff", "ffffff", "ffffffff", "ffffffff80", "ffffffffff",
                    "ffffffffff01", "808080808080", "e58e26", "9601", "ffffffff08", "ffffffff0e",
            };
            for (String hx : decodes) {
                byte[] in = Util.unhex(hx);
                ByteBuf b = Util.wrap(in);
                String res;
                try {
                    int v = NettyPacketUtil.decodeVarInt(b);
                    res = v + ":" + b.readerIndex();
                } catch (Throwable t) {
                    res = "err\t" + Util.describe(t);
                }
                o.line("dec", hx, res);
            }
            for (int len : new int[]{0, 1, 127, 128, 16383, 16384, 2097151, 2097152}) {
                byte[] framed = framedRaw(new byte[len]);
                int plen = varint(len).length;
                o.line("frame", Integer.toString(len), Util.hex(java.util.Arrays.copyOf(framed, plen)));
            }
        }
    }

    static String utf8hex(String s) {
        return Util.hex(s.getBytes(StandardCharsets.UTF_8));
    }

    static void strings(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "strings.tsv")) {
            o.raw("# enc\tutf8 hex of the string\tencoded hex  |  dec\tmax\tinput hex\tok<TAB>utf8 hex:consumed | err<TAB>class<TAB>message  |  len\tchars\tprefix hex");
            String[] samples = {"", "a", "hello", Vectors.ZH, Vectors.EMOJI, Vectors.MIX, "a\u0000b", "﻿", "�",
                    new String(Character.toChars(0x10FFFF)), "\u0080", "߿", "ࠀ", "￿", "😀",
                    "é", "a".repeat(127), "a".repeat(128), "é".repeat(100), "\n\r\t", "\\\"'", "ｆｕｌｌ", "한국어", "🏳️‍🌈"};
            for (String s : samples) {
                o.line("enc", utf8hex(s), str(s));
            }
            String[][] decodes = {
                    {"131072", "00"}, {"131072", "01 41"}, {"131072", "02 4142 ff"}, {"131072", "05 4142"}, {"131072", ""},
                    {"131072", "ffffffff0f"}, {"131072", "8080808008"}, {"131072", "ffffffffff"}, {"131072", "ff"},
                    {"131072", "808040 " + "41".repeat(3)}, {"131072", "8100 41"}, {"131072", "e5a4a7"}, {"131072", "03 e5a4a7"},
                    {"131072", "03 e5a4"}, {"131072", "04 e5a4a7 41"}, {"131072", "0a 00 41 42 43 44 45 46 47 48 49"},
                    {"3", "03 414243"}, {"3", "04 41424344"}, {"0", "00"}, {"0", "01 41"}, {"20", "14" + "41".repeat(20)},
                    {"20", "15" + "41".repeat(21)}, {"32", "20" + "41".repeat(32)}, {"32", "21" + "41".repeat(33)},
                    {"200", "c801" + "41".repeat(200)}, {"200", "c901" + "41".repeat(201)}, {"32767", "ffff01 41"},
                    {"32767", "ffff01"}, {"131072", "808008" + "41".repeat(10)}, {"131072", "808008"},
                    {"131072", "818008"}, {"131072", "ffff07"}, {"2147483647", "ffffffff07"}, {"2147483647", "01 41"},
                    {"-1", "00"}, {"-1", "01 41"},
            };
            for (String[] d : decodes) {
                byte[] in = Util.unhex(h(d[1]));
                ByteBuf b = Util.wrap(in);
                String res;
                try {
                    String s = NettyPacketUtil.decodeString(b, Integer.parseInt(d[0]));
                    res = "ok\t" + utf8hex(s) + ":" + b.readerIndex();
                } catch (Throwable t) {
                    res = "err\t" + Util.describe(t);
                }
                o.line("dec", d[0], h(d[1]), res);
            }
            for (int n : new int[]{16383, 16384, 65535, 65536}) {
                o.line("len", Integer.toString(n), Util.hex(java.util.Arrays.copyOf(Util.unhex(str("a".repeat(n))), 4)));
            }
            // limits that need big inputs: Rust rebuilds them by name
            o.line("big", "default_max_ok", bigString(131072));
            o.line("big", "default_max_plus_one", bigString(131073));
            o.line("big", "short_max_ok", bigString(32767, 32767));
            o.line("big", "short_max_plus_one", bigString(32768, 32767));
        }
    }

    static String bigString(int n) {
        return bigString(n, 131072);
    }

    /** A string field of {@code n} 'a' bytes decoded with the given maximum: outcome only. */
    static String bigString(int n, int max) {
        byte[] body = new byte[n];
        java.util.Arrays.fill(body, (byte) 'a');
        byte[] in = Util.concat(varint(n), body);
        try {
            String s = NettyPacketUtil.decodeString(Util.wrap(in), max);
            return "ok:" + s.length();
        } catch (Throwable t) {
            return "err:" + Util.code(t);
        }
    }

    static void utf8Lossy(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "utf8_lossy.tsv")) {
            o.raw("# input bytes (hex)\tUTF-8 hex of the string NettyPacketUtil.decodeString returns");
            int[] alphabet = {0x00, 0x41, 0x80, 0x8f, 0x90, 0x9f, 0xa0, 0xbf, 0xc0, 0xc2, 0xdf, 0xe0, 0xe1, 0xed, 0xef, 0xf0, 0xf1, 0xf4, 0xf5, 0xff};
            List<byte[]> inputs = new ArrayList<>();
            inputs.add(new byte[0]);
            for (int a : alphabet) inputs.add(new byte[]{(byte) a});
            for (int a : alphabet) for (int b : alphabet) inputs.add(new byte[]{(byte) a, (byte) b});
            for (int a : alphabet) for (int b : alphabet) for (int c : alphabet) inputs.add(new byte[]{(byte) a, (byte) b, (byte) c});
            Util.Rng r = new Util.Rng(0x07F8L);
            for (int i = 0; i < 4000; i++) {
                int n = 4 + r.below(9);
                byte[] x = new byte[n];
                for (int j = 0; j < n; j++) x[j] = (byte) alphabet[r.below(alphabet.length)];
                inputs.add(x);
            }
            // valid sequences cut and glued
            String[] valid = {"é", "字", "🎮", "￿", "ࠀ", "߿", new String(Character.toChars(0x10FFFF))};
            for (String s : valid) {
                byte[] full = s.getBytes(StandardCharsets.UTF_8);
                for (int n = 1; n <= full.length; n++) {
                    byte[] cut = java.util.Arrays.copyOf(full, n);
                    inputs.add(cut);
                    inputs.add(Util.concat(cut, new byte[]{0x41}));
                    inputs.add(Util.concat(cut, full));
                }
            }
            for (byte[] in : inputs) {
                byte[] body = Util.concat(varint(in.length), in);
                String s = NettyPacketUtil.decodeString(Util.wrap(body), 131072);
                // The server reads from pooled direct buffers; make sure the heap and direct decoders agree.
                ByteBuf direct = Unpooled.directBuffer(body.length).writeBytes(body);
                String d = NettyPacketUtil.decodeString(direct, 131072);
                direct.release();
                if (!s.equals(d)) {
                    throw new IllegalStateException("heap and direct UTF-8 decoding differ for " + Util.hex(in));
                }
                o.line(Util.hex(in), utf8hex(s));
            }
        }
    }

    // ------------------------------------------------------------ float16

    static final int[] MANTISSAS = {0, 1, 0xfff, 0x1000, 0x1001, 0x1fff, 0x2000, 0x2001, 0x3000, 0x3fff, 0x400000, 0x400001, 0x7fffff,
            0x7ff000, 0x7fefff, 0x7ffe00, 0x200, 0x201, 0x1ff, 0x555555, 0x2aaaaa, 0x123456, 0x7f0000, 0x00ffff, 0x010000,
            0x008000, 0x004000, 0x000800, 0x000400, 0x000002, 0x001000, 0x7fffff - 0x1000};

    static void float16(Path dir) throws IOException {
        // every half-precision bit pattern -> float bits
        try (Util.Out o = new Util.Out(dir, "float16_decode.txt")) {
            o.raw("# 65536 words, one 8-digit hex float bit pattern per half value 0x0000..0xffff, 256 per line");
            StringBuilder sb = new StringBuilder();
            for (int i = 0; i < 65536; i++) {
                float f = Float16Util.halfToFloat((short) i);
                sb.append(String.format("%08x", Float.floatToRawIntBits(f)));
                if (i % 256 == 255) {
                    o.raw(sb.toString());
                    sb.setLength(0);
                }
            }
        }
        // float bits -> half. Inputs: structured (sign x exponent x mantissa patterns), then splitmix64.
        try (Util.Out o = new Util.Out(dir, "float16_encode.txt")) {
            o.raw("# 4-digit hex half per input float bit pattern; inputs: sign (0,1) x exponent 0..255 x 32 mantissa patterns, then 32768 splitmix64(seed 0xF16) low words; 256 per line");
            List<Integer> in = new ArrayList<>();
            for (int s = 0; s < 2; s++) {
                for (int e = 0; e < 256; e++) {
                    for (int m : MANTISSAS) in.add((s << 31) | (e << 23) | m);
                }
            }
            Util.Rng r = new Util.Rng(0xF16L);
            for (int i = 0; i < 32768; i++) in.add((int) r.next());
            StringBuilder sb = new StringBuilder();
            int n = 0;
            for (int bits : in) {
                short h = Float16Util.floatToHalf(Float.intBitsToFloat(bits));
                sb.append(String.format("%04x", h & 0xffff));
                if (++n % 256 == 0) {
                    o.raw(sb.toString());
                    sb.setLength(0);
                }
            }
            if (sb.length() > 0) o.raw(sb.toString());
        }
    }

    // ------------------------------------------------------------ streams

    static String cat(String... hex) {
        return String.join("", hex);
    }

    /** Length prefix 300000, then a ping id and 299999 zero bytes of trailing garbage. */
    static byte[] bigFrame() {
        return Util.concat(varint(300000), new byte[300000]);
    }

    static void stream(Util.Out o, String name, String side, String handshake, List<String> chunkHex) {
        List<byte[]> chunks = new ArrayList<>();
        for (String c : chunkHex) {
            if (c.equals("gen:big_frame")) {
                chunks.add(bigFrame());
            } else if (c.equals("gen:big_frame_chunked")) {
                byte[] all = bigFrame();
                for (int p = 0; p < all.length; p += 4096) chunks.add(java.util.Arrays.copyOfRange(all, p, Math.min(all.length, p + 4096)));
            } else {
                chunks.add(Util.unhex(c));
            }
        }
        boolean server = side.equals("server");
        Streams.Result r = Streams.run(server, handshake.equals("1"), chunks);
        o.line(name, side, handshake, String.join("|", chunkHex), r.render());
    }

    static void streams(Path dir) throws IOException {
        try (Util.Out o = new Util.Out(dir, "streams.tsv")) {
            o.raw("# name\tside (server: decodes ServerBound, client: decodes ClientBound)\thandshake (1: starts with HandshakeDecoder)\tinbound chunks (hex, '|' separated, 'gen:' = built by the test)\tevents: P:<packet hex> E:<exception reaching PlayerConnection while open> V:<version> H:<handshake failure>, then CLOSED or OPEN (whether the pipeline had closed the channel by the end of the input). Events the pipeline produced afterwards, when the peer hung up, come last, prefixed EOF:");
            String ping = f("00");
            String chat = f(Util.hex(encSB(top.rymc.phira.protocol.packet.serverbound.ServerBoundChatPacket.create("hi"))));
            String join = f(Util.hex(encSB(top.rymc.phira.protocol.packet.serverbound.ServerBoundJoinRoomPacket.create("r1", true))));
            String touches = f(Util.hex(encSB(top.rymc.phira.protocol.packet.serverbound.ServerBoundTouchesPacket.create(Vectors.frames()))));
            String longChat = f(Util.hex(encSB(top.rymc.phira.protocol.packet.serverbound.ServerBoundChatPacket.create("x".repeat(200)))));

            List<String> none = List.of();
            // ---- framing (server side, no handshake)
            stream(o, "empty", "server", "0", none);
            stream(o, "empty_chunk", "server", "0", List.of(""));
            stream(o, "nul_only", "server", "0", List.of("000000"));
            stream(o, "nul_then_ping", "server", "0", List.of(cat("0000", ping)));
            stream(o, "ping", "server", "0", List.of(ping));
            stream(o, "ping_ping_one_chunk", "server", "0", List.of(cat(ping, ping)));
            stream(o, "nul_between_frames", "server", "0", List.of(cat(ping, "00", "00", ping, "00")));
            stream(o, "nul_then_len_wait", "server", "0", List.of(cat("0000", "20", "0000")));
            stream(o, "mixed_one_chunk", "server", "0", List.of(cat(chat, join, ping, touches, longChat)));
            List<String> bytewise = new ArrayList<>();
            String all = cat(chat, join, ping, touches, longChat);
            for (int i = 0; i < all.length(); i += 2) bytewise.add(all.substring(i, i + 2));
            stream(o, "mixed_byte_by_byte", "server", "0", bytewise);
            stream(o, "len_varint_split", "server", "0", List.of(longChat.substring(0, 2), longChat.substring(2)));
            stream(o, "split_in_payload", "server", "0", List.of(chat.substring(0, 4), chat.substring(4)));
            stream(o, "frame_then_partial", "server", "0", List.of(cat(ping, chat.substring(0, 6))));
            stream(o, "partial_only", "server", "0", List.of(chat.substring(0, 6)));
            stream(o, "noncanonical_len", "server", "0", List.of("810000"));
            stream(o, "noncanonical_len_5byte", "server", "0", List.of("818080800000"));
            stream(o, "empty_frame_via_8000", "server", "0", List.of(cat("8000", ping)));
            stream(o, "empty_frame_via_808000", "server", "0", List.of(cat("808000", ping)));
            stream(o, "len_is_zero_with_8080808000", "server", "0", List.of(cat("8080808000", ping)));
            stream(o, "bad_varint", "server", "0", List.of("ffffffffff"));
            stream(o, "bad_varint_after_ping", "server", "0", List.of(cat(ping, "ffffffffff", ping)));
            stream(o, "bad_varint_split", "server", "0", List.of("ffff", "ffff", "ff", ping));
            stream(o, "neg_len_ffffffff0f", "server", "0", List.of("ffffffff0f"));
            stream(o, "neg_len_8080808008", "server", "0", List.of(cat(ping, "8080808008", ping)));
            stream(o, "neg_len_ffffffff7f", "server", "0", List.of("ffffffff7f"));
            stream(o, "huge_len_waits", "server", "0", List.of("ffffffff07" + "00"));
            stream(o, "huge_len_2pow28_waits", "server", "0", List.of("8080808001" + "00000000"));
            stream(o, "varint_4_continuations_wait", "server", "0", List.of("ffffffff"));
            stream(o, "len_trailing_bytes_in_frame", "server", "0", List.of(f("00aabb")));
            stream(o, "unknown_id_20", "server", "0", List.of(f("20")));
            stream(o, "unknown_id_10", "server", "0", List.of(f("10")));
            stream(o, "unknown_id_ff", "server", "0", List.of(cat(ping, f("ff"), ping)));
            stream(o, "unknown_id_between", "server", "0", List.of(cat(chat, f("33"), chat)));
            stream(o, "string_need_more", "server", "0", List.of(f("01 05 41")));
            stream(o, "string_len_missing", "server", "0", List.of(f("01")));
            stream(o, "string_too_long_auth", "server", "0", List.of(f("01 21" + "41".repeat(33))));
            stream(o, "string_bad_varint_in_frame", "server", "0", List.of(f("01 ffffffffff")));
            stream(o, "string_neg_len", "server", "0", List.of(f("01 ffffffff0f")));
            stream(o, "bool_missing_ioobe", "server", "0", List.of(f("06 01 41")));
            stream(o, "int_missing_ioobe", "server", "0", List.of(f("0a 010203")));
            stream(o, "empty_payload_then_id_only_ioobe", "server", "0", List.of(f("0e")));
            stream(o, "bool_ioobe_after_ok", "server", "0", List.of(cat(ping, chat, f("08"), ping)));
            stream(o, "neg_list_count", "server", "0", List.of(cat(ping, f("03 ffffffff0f"), ping)));
            stream(o, "huge_list_count", "server", "0", List.of(f("03 ffffff7f")));
            stream(o, "huge_list_count_judges", "server", "0", List.of(f("04 ffffff7f")));
            stream(o, "unknown_judgement", "server", "0", List.of(f("04 01 00000000 01000000 02000000 06")));
            stream(o, "frame_in_frame_boundary", "server", "0", List.of(ping.substring(0, 2), ping.substring(2) + ping));
            stream(o, "after_close_frames_dropped", "server", "0", List.of(cat(ping, f("20"), ping, ping)));
            stream(o, "after_close_later_chunk_dropped", "server", "0", List.of(f("20"), ping));
            stream(o, "after_error_close_later_chunk_dropped", "server", "0", List.of(f("06 01 41"), ping));
            stream(o, "big_frame", "server", "0", List.of("gen:big_frame"));
            stream(o, "big_frame_chunked", "server", "0", List.of("gen:big_frame_chunked"));

            // ---- handshake (server side)
            stream(o, "hs_version_only", "server", "1", List.of("01"));
            stream(o, "hs_version_ping_same_chunk", "server", "1", List.of(cat("01", ping)));
            stream(o, "hs_version_then_ping", "server", "1", List.of("01", ping));
            stream(o, "hs_version_two_frames", "server", "1", List.of(cat("01", chat, ping)));
            stream(o, "hs_version_partial_frame", "server", "1", List.of(cat("01", chat.substring(0, 4)), chat.substring(4)));
            stream(o, "hs_version_nul_ping", "server", "1", List.of(cat("01", "0000", ping)));
            stream(o, "hs_nothing", "server", "1", none);
            stream(o, "hs_bad_00", "server", "1", List.of("00"));
            stream(o, "hs_bad_02", "server", "1", List.of("02"));
            stream(o, "hs_bad_7f", "server", "1", List.of("7f"));
            stream(o, "hs_bad_80", "server", "1", List.of("80"));
            stream(o, "hs_bad_ff", "server", "1", List.of("ff"));
            stream(o, "hs_bad_then_data", "server", "1", List.of(cat("05", ping)));
            stream(o, "hs_version_then_bad_frame", "server", "1", List.of(cat("01", f("20"), ping)));
            stream(o, "hs_version_then_neg_len", "server", "1", List.of(cat("01", "ffffffff0f")));
            o.line("hs_timeout", "server", "timeout", "-", Streams.handshakeTimeout());
            o.line("hs_eof", "server", "eof", "-", Streams.handshakeEof());

            // ---- client side
            String pong = f("00");
            String chatOk = f("02 01");
            String chatFail = f(Util.hex(encCB(top.rymc.phira.protocol.packet.clientbound.ClientBoundChatPacket.failed("no"))));
            String msg = f(Util.hex(encCB(top.rymc.phira.protocol.packet.clientbound.ClientBoundMessagePacket.create(new top.rymc.phira.protocol.data.message.ChatMessage(-1, "欢迎")))));
            String state = f(Util.hex(encCB(top.rymc.phira.protocol.packet.clientbound.ClientBoundChangeStatePacket.create(new top.rymc.phira.protocol.data.state.SelectChart(5)))));
            stream(o, "cb_pong", "client", "0", List.of(pong));
            stream(o, "cb_mixed", "client", "0", List.of(cat(pong, chatOk, chatFail, msg, state)));
            String firstThree = cat(pong, chatOk, chatFail);
            stream(o, "cb_mixed_split", "client", "0", List.of(firstThree.substring(0, 12), cat(firstThree.substring(12), msg, state)));
            stream(o, "cb_unknown_id_14", "client", "0", List.of(cat(pong, f("14"), pong)));
            stream(o, "cb_unknown_id_ff", "client", "0", List.of(f("ff")));
            stream(o, "cb_id_13_abort", "client", "0", List.of(f("13 01")));
            stream(o, "cb_state_unknown", "client", "0", List.of(f("06 03")));
            stream(o, "cb_message_unknown", "client", "0", List.of(f("05 10")));
            stream(o, "cb_bool_missing", "client", "0", List.of(f("07")));
            stream(o, "cb_neg_list", "client", "0", List.of(f("03 07000000 ffffffff0f")));
            stream(o, "cb_nul_skipped", "client", "0", List.of(cat("00", "00", pong)));
            stream(o, "cb_bad_varint", "client", "0", List.of(cat(pong, "ffffffffff")));
            stream(o, "cb_neg_len", "client", "0", List.of("ffffffff0f"));
            stream(o, "cb_string_need_more", "client", "0", List.of(f("02 00 05 41")));
        }
    }
}
