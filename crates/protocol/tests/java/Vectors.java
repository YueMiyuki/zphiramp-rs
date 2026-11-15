import io.netty.buffer.ByteBuf;
import top.rymc.phira.protocol.codec.Encodeable;
import top.rymc.phira.protocol.data.EncodeableVoid;
import top.rymc.phira.protocol.data.FullUserProfile;
import top.rymc.phira.protocol.data.PacketResult;
import top.rymc.phira.protocol.data.RoomInfo;
import top.rymc.phira.protocol.data.UserProfile;
import top.rymc.phira.protocol.data.message.*;
import top.rymc.phira.protocol.data.monitor.judge.JudgeEvent;
import top.rymc.phira.protocol.data.monitor.judge.Judgement;
import top.rymc.phira.protocol.data.monitor.touch.CompactPos;
import top.rymc.phira.protocol.data.monitor.touch.TouchFrame;
import top.rymc.phira.protocol.data.monitor.touch.TouchPoint;
import top.rymc.phira.protocol.data.state.GameState;
import top.rymc.phira.protocol.data.state.Playing;
import top.rymc.phira.protocol.data.state.SelectChart;
import top.rymc.phira.protocol.data.state.WaitForReady;
import top.rymc.phira.protocol.packet.ClientBoundPacket;
import top.rymc.phira.protocol.packet.ServerBoundPacket;
import top.rymc.phira.protocol.packet.clientbound.*;
import top.rymc.phira.protocol.packet.serverbound.*;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Function;

/**
 * Every vector the Rust tests replay. The names are the contract with {@code tests/golden_packets.rs}:
 * the Rust side builds a value for each name and compares it with the bytes recorded here.
 * Names are stable: renaming one means updating the matching table in {@code tests/golden_packets.rs}.
 */
final class Vectors {
    private Vectors() {}

    record Named<T>(String name, T value) {}

    record TypeVec(String name, Encodeable value, Function<ByteBuf, ? extends Encodeable> decode) {}

    static final List<Named<ServerBoundPacket>> SB = new ArrayList<>();
    static final List<Named<ClientBoundPacket>> CB = new ArrayList<>();
    static final List<TypeVec> TYPES = new ArrayList<>();

    static final String ZH = "你好，世界";
    static final String EMOJI = "🎮🎵🏆";
    static final String MIX = "Zenith 音游战队 🎶 é";

    static String rep(String s, int n) {
        return s.repeat(n);
    }

    static final float NaN = Float.NaN;
    static final float INF = Float.POSITIVE_INFINITY;
    static final float NINF = Float.NEGATIVE_INFINITY;
    /** Quiet NaN with a payload, and a signalling NaN. */
    static final float NAN_PAYLOAD = Float.intBitsToFloat(0x7fc01234);
    static final float SNAN = Float.intBitsToFloat(0x7f800001);
    static final float NEG_NAN = Float.intBitsToFloat(0xffc00000);

    /** Floats that exercise every branch of the half-float conversion. */
    static final float[] F16 = {
            0f, -0f, 1f, -1f, 0.5f, -2f, 0.1f, 1.5f, 3.14159265f, 100.25f,
            65504f,            // largest half
            65519f, 65520f,    // just under / on the overflow rounding boundary
            1e9f, INF, NINF, NaN, NAN_PAYLOAD, NEG_NAN,
            6.1035156e-5f,     // smallest normal half (2^-14)
            6.097555e-5f,      // largest subnormal half
            5.9604645e-8f,     // smallest subnormal half (2^-24)
            2.9802322e-8f,     // 2^-25: tie rounding to even
            4.4703484e-8f,     // 1.5 * 2^-25
            1e-8f, -1e-8f,     // underflow to zero
            Float.MIN_VALUE, Float.MAX_VALUE, -Float.MAX_VALUE, Float.MIN_NORMAL,
            0.99951171875f,    // exactly representable, rounds trivially
            1.00048828125f,    // halfway between two halves
            2047.5f, 2048.5f,  // rounding at the top of the 11-bit mantissa
    };

    static final float[] TIMES = {0f, -0f, 1.5f, 123.456f, -7.25f, NaN, NAN_PAYLOAD, SNAN, INF, NINF,
            Float.MIN_VALUE, Float.MAX_VALUE, Float.MIN_NORMAL};

    static void sb(String name, ServerBoundPacket p) {
        SB.add(new Named<>(name, p));
    }

    static void cb(String name, ClientBoundPacket p) {
        CB.add(new Named<>(name, p));
    }

    static void type(String name, Encodeable v, Function<ByteBuf, ? extends Encodeable> dec) {
        TYPES.add(new TypeVec(name, v, dec));
    }

    static UserProfile up(int id, String n) {
        return new UserProfile(id, n);
    }

    static FullUserProfile fup(int id, String n, boolean m) {
        return new FullUserProfile(id, n, m);
    }

    static List<UserProfile> ups(UserProfile... u) {
        return List.of(u);
    }

    static TouchPoint tp(int id, float x, float y) {
        return new TouchPoint((byte) id, new CompactPos(x, y));
    }

    static TouchFrame tf(float t, TouchPoint... p) {
        return new TouchFrame(t, List.of(p));
    }

    static JudgeEvent je(float t, int line, int note, Judgement j) {
        return new JudgeEvent(t, line, note, j);
    }

    static List<TouchFrame> frames() {
        List<TouchFrame> f = new ArrayList<>();
        f.add(tf(0f));
        f.add(tf(1.5f, tp(0, 0.5f, -0.5f)));
        f.add(tf(2.25f, tp(1, 0f, 0f), tp(-1, 1f, 1f), tp(127, 65504f, -65504f), tp(-128, 0.099975586f, 3f)));
        return f;
    }

    static List<JudgeEvent> judges() {
        List<JudgeEvent> j = new ArrayList<>();
        j.add(je(0f, 0, 0, Judgement.Perfect));
        j.add(je(1.25f, 3, 17, Judgement.Good));
        j.add(je(-0.5f, -1, -2, Judgement.Bad));
        j.add(je(99.5f, Integer.MAX_VALUE, Integer.MIN_VALUE, Judgement.Miss));
        j.add(je(NaN, 0x01020304, 5, Judgement.HoldPerfect));
        j.add(je(INF, 6, 7, Judgement.HoldGood));
        return j;
    }

    static {
        buildServerBound();
        buildClientBound();
        buildTypes();
    }

    // ---------------------------------------------------------------- serverbound

    static void buildServerBound() {
        sb("Ping", ServerBoundPingPacket.INSTANCE);

        sb("Authenticate.empty", ServerBoundAuthenticatePacket.create(""));
        sb("Authenticate.ascii", ServerBoundAuthenticatePacket.create("abcDEF0123456789"));
        sb("Authenticate.max32", ServerBoundAuthenticatePacket.create(rep("t", 32)));
        sb("Authenticate.max32_cjk", ServerBoundAuthenticatePacket.create(rep("令", 10) + "ab")); // 32 bytes
        sb("Authenticate.zh", ServerBoundAuthenticatePacket.create("令牌"));
        sb("Authenticate.emoji", ServerBoundAuthenticatePacket.create("🎮🎮")); // 8 bytes
        sb("Authenticate.nul", ServerBoundAuthenticatePacket.create("a\u0000b"));
        sb("Authenticate.too_long33", ServerBoundAuthenticatePacket.create(rep("t", 33)));
        sb("Authenticate.too_long_cjk", ServerBoundAuthenticatePacket.create(rep("令", 11))); // 33 bytes

        sb("Chat.empty", ServerBoundChatPacket.create(""));
        sb("Chat.hello", ServerBoundChatPacket.create("hello"));
        sb("Chat.zh", ServerBoundChatPacket.create(ZH));
        sb("Chat.emoji", ServerBoundChatPacket.create(EMOJI));
        sb("Chat.mix", ServerBoundChatPacket.create(MIX));
        sb("Chat.max200", ServerBoundChatPacket.create(rep("x", 200)));
        sb("Chat.max200_cjk", ServerBoundChatPacket.create(rep("字", 66) + "yz")); // 200 bytes
        sb("Chat.too_long201", ServerBoundChatPacket.create(rep("x", 201)));
        sb("Chat.too_long_cjk", ServerBoundChatPacket.create(rep("字", 67))); // 201 bytes
        sb("Chat.long_varint2", ServerBoundChatPacket.create(rep("y", 128))); // length needs two varint bytes

        sb("Touches.empty", ServerBoundTouchesPacket.create(List.of()));
        sb("Touches.one_empty_frame", ServerBoundTouchesPacket.create(List.of(tf(0f))));
        sb("Touches.frames", ServerBoundTouchesPacket.create(frames()));
        for (int i = 0; i < F16.length; i++) {
            sb("Touches.f16_" + i, ServerBoundTouchesPacket.create(List.of(tf(TIMES[i % TIMES.length], tp(i, F16[i], F16[F16.length - 1 - i])))));
        }
        List<TouchFrame> many = new ArrayList<>();
        for (int i = 0; i < 130; i++) many.add(tf(i * 0.016f, tp(i, i * 0.5f, -i * 0.25f)));
        sb("Touches.many130", ServerBoundTouchesPacket.create(many)); // list length needs two varint bytes

        sb("Judges.empty", ServerBoundJudgesPacket.create(List.of()));
        sb("Judges.one", ServerBoundJudgesPacket.create(List.of(je(12.5f, 1, 2, Judgement.Perfect))));
        sb("Judges.all", ServerBoundJudgesPacket.create(judges()));
        for (int i = 0; i < TIMES.length; i++) {
            sb("Judges.time_" + i, ServerBoundJudgesPacket.create(List.of(je(TIMES[i], i, -i, Judgement.values()[i % 6]))));
        }

        sb("CreateRoom.empty", ServerBoundCreateRoomPacket.create(""));
        sb("CreateRoom.simple", ServerBoundCreateRoomPacket.create("room1"));
        sb("CreateRoom.max20", ServerBoundCreateRoomPacket.create("abcdefghij0123456789"));
        sb("CreateRoom.zh", ServerBoundCreateRoomPacket.create("房间"));
        sb("CreateRoom.emoji", ServerBoundCreateRoomPacket.create("🎮ab"));
        sb("CreateRoom.max20_cjk", ServerBoundCreateRoomPacket.create(rep("房", 6) + "ab")); // 20 bytes
        sb("CreateRoom.too_long21", ServerBoundCreateRoomPacket.create("abcdefghij01234567890"));
        sb("CreateRoom.too_long_cjk", ServerBoundCreateRoomPacket.create(rep("房", 7))); // 21 bytes

        sb("JoinRoom.player", ServerBoundJoinRoomPacket.create("room1", false));
        sb("JoinRoom.monitor", ServerBoundJoinRoomPacket.create("room1", true));
        sb("JoinRoom.empty", ServerBoundJoinRoomPacket.create("", false));
        sb("JoinRoom.zh", ServerBoundJoinRoomPacket.create("测试房", true));
        sb("JoinRoom.max20", ServerBoundJoinRoomPacket.create(rep("r", 20), false));
        sb("JoinRoom.too_long", ServerBoundJoinRoomPacket.create(rep("r", 21), true));

        sb("LeaveRoom", ServerBoundLeaveRoomPacket.INSTANCE);
        sb("LockRoom.true", ServerBoundLockRoomPacket.create(true));
        sb("LockRoom.false", ServerBoundLockRoomPacket.create(false));
        sb("CycleRoom.true", ServerBoundCycleRoomPacket.create(true));
        sb("CycleRoom.false", ServerBoundCycleRoomPacket.create(false));

        int[] ints = {0, 1, -1, 42, 255, 256, 65535, 0x01020304, Integer.MAX_VALUE, Integer.MIN_VALUE, -2};
        for (int i : ints) {
            sb("SelectChart." + i, ServerBoundSelectChartPacket.create(i));
            sb("Played." + i, ServerBoundPlayedPacket.create(i));
        }
        sb("RequestStart", ServerBoundRequestStartPacket.INSTANCE);
        sb("Ready", ServerBoundReadyPacket.INSTANCE);
        sb("CancelReady", ServerBoundCancelReadyPacket.INSTANCE);
        sb("Abort", ServerBoundAbortPacket.INSTANCE);
    }

    // ---------------------------------------------------------------- clientbound

    static List<RoomInfo> rooms() {
        List<RoomInfo> r = new ArrayList<>();
        r.add(new RoomInfo("room1", new SelectChart(), false, false, false, false, false, ups(), ups()));
        r.add(new RoomInfo("Room-中文", new WaitForReady(), true, true, true, true, true,
                ups(up(1, "alice"), up(2, "鲍勃")), ups(up(-5, "watcher🎥"))));
        r.add(new RoomInfo("p", new Playing(), false, true, false, true, false, ups(up(Integer.MAX_VALUE, "max")), ups()));
        r.add(new RoomInfo("", new SelectChart(42), true, false, true, false, true, ups(), ups(up(7, ""), up(8, "m"))));
        r.add(new RoomInfo("abcdefghij0123456789", new SelectChart(-1), false, false, false, true, true, ups(up(0, "zero"), up(Integer.MIN_VALUE, "min")), ups()));
        r.add(new RoomInfo("abcdefghij01234567890", new Playing(), false, false, false, false, false, ups(up(3, "x")), ups())); // 21 bytes: decode must fail
        List<UserProfile> many = new ArrayList<>();
        for (int i = 0; i < 40; i++) many.add(up(1000 + i, "user" + i));
        r.add(new RoomInfo("big", new WaitForReady(), true, false, true, false, false, many, ups(up(9999, "obs"))));
        return r;
    }

    static List<GameState> states() {
        return List.of(new SelectChart(), new SelectChart(0), new SelectChart(42), new SelectChart(-1),
                new SelectChart(Integer.MAX_VALUE), new SelectChart(Integer.MIN_VALUE), new WaitForReady(), new Playing());
    }

    static void buildClientBound() {
        cb("Pong", ClientBoundPongPacket.INSTANCE);

        cb("Authenticate.failed_empty", ClientBoundAuthenticatePacket.failed(""));
        cb("Authenticate.failed_msg", ClientBoundAuthenticatePacket.failed("bad token"));
        cb("Authenticate.failed_zh", ClientBoundAuthenticatePacket.failed("认证失败 🙅"));
        cb("Authenticate.failed_long", ClientBoundAuthenticatePacket.failed(rep("e", 300)));
        cb("Authenticate.success", ClientBoundAuthenticatePacket.success(fup(1, "alice", false)));
        cb("Authenticate.success_monitor", ClientBoundAuthenticatePacket.success(fup(-1, "观察者", true)));
        cb("Authenticate.success_empty_name", ClientBoundAuthenticatePacket.success(fup(0, "", false)));
        List<RoomInfo> rooms = rooms();
        for (int i = 0; i < rooms.size(); i++) {
            cb("Authenticate.success_room" + i, ClientBoundAuthenticatePacket.success(fup(100 + i, "player" + i, i % 2 == 0), rooms.get(i)));
        }

        voidPacket("Chat", ClientBoundChatPacket.success(), ClientBoundChatPacket::failed);
        voidPacket("CreateRoom", ClientBoundCreateRoomPacket.success(), ClientBoundCreateRoomPacket::failed);
        voidPacket("LeaveRoom", ClientBoundLeaveRoomPacket.success(), ClientBoundLeaveRoomPacket::failed);
        voidPacket("LockRoom", ClientBoundLockRoomPacket.success(), ClientBoundLockRoomPacket::failed);
        voidPacket("CycleRoom", ClientBoundCycleRoomPacket.success(), ClientBoundCycleRoomPacket::failed);
        voidPacket("SelectChart", ClientBoundSelectChartPacket.success(), ClientBoundSelectChartPacket::failed);
        voidPacket("RequestStart", ClientBoundRequestStartPacket.success(), ClientBoundRequestStartPacket::failed);
        voidPacket("Ready", ClientBoundReadyPacket.success(), ClientBoundReadyPacket::failed);
        voidPacket("CancelReady", ClientBoundCancelReadyPacket.success(), ClientBoundCancelReadyPacket::failed);
        voidPacket("Played", ClientBoundPlayedPacket.success(), ClientBoundPlayedPacket::failed);
        voidPacket("Abort", ClientBoundAbortPacket.success(), ClientBoundAbortPacket::failed);

        cb("Touches.empty", ClientBoundTouchesPacket.create(0, List.of()));
        cb("Touches.frames", ClientBoundTouchesPacket.create(7, frames()));
        cb("Touches.neg_id", ClientBoundTouchesPacket.create(-1, List.of(tf(1f, tp(3, 0.25f, 0.75f)))));
        cb("Touches.max_id", ClientBoundTouchesPacket.create(Integer.MAX_VALUE, List.of(tf(NaN))));
        cb("Touches.min_id", ClientBoundTouchesPacket.create(Integer.MIN_VALUE, frames()));
        for (int i = 0; i < F16.length; i += 3) {
            cb("Touches.f16_" + i, ClientBoundTouchesPacket.create(i, List.of(tf(TIMES[i % TIMES.length], tp(i, F16[i], F16[(i * 7) % F16.length])))));
        }

        cb("Judges.empty", ClientBoundJudgesPacket.create(0, List.of()));
        cb("Judges.all", ClientBoundJudgesPacket.create(5, judges()));
        cb("Judges.neg_id", ClientBoundJudgesPacket.create(-9, List.of(je(0.5f, 1, 1, Judgement.Good))));

        for (Named<Message> m : messages()) {
            cb("Message." + m.name(), ClientBoundMessagePacket.create(m.value()));
        }

        List<GameState> states = states();
        for (int i = 0; i < states.size(); i++) {
            cb("ChangeState." + i, ClientBoundChangeStatePacket.create(states.get(i)));
        }
        cb("ChangeHost.true", ClientBoundChangeHostPacket.create(true));
        cb("ChangeHost.false", ClientBoundChangeHostPacket.create(false));

        cb("JoinRoom.failed_empty", ClientBoundJoinRoomPacket.failed(""));
        cb("JoinRoom.failed_msg", ClientBoundJoinRoomPacket.failed("room not found"));
        cb("JoinRoom.failed_zh", ClientBoundJoinRoomPacket.failed("房间不存在"));
        cb("JoinRoom.success_empty", ClientBoundJoinRoomPacket.success(new SelectChart(), ups(), ups(), false));
        cb("JoinRoom.success_users", ClientBoundJoinRoomPacket.success(new WaitForReady(), ups(up(1, "alice"), up(2, "鲍勃")), ups(up(3, "obs")), true));
        cb("JoinRoom.success_playing", ClientBoundJoinRoomPacket.success(new Playing(), ups(up(Integer.MIN_VALUE, "n")), ups(), true));
        cb("JoinRoom.success_chart", ClientBoundJoinRoomPacket.success(new SelectChart(99), ups(up(5, "e")), ups(up(6, "🎥"), up(7, "g")), false));
        List<UserProfile> many = new ArrayList<>();
        for (int i = 0; i < 130; i++) many.add(up(i, "u" + i));
        cb("JoinRoom.success_many130", ClientBoundJoinRoomPacket.success(new SelectChart(1), many, ups(), false));

        cb("OnJoinRoom.player", ClientBoundOnJoinRoomPacket.create(up(1, "alice"), false));
        cb("OnJoinRoom.monitor", ClientBoundOnJoinRoomPacket.create(up(2, "观察者"), true));
        cb("OnJoinRoom.extremes", ClientBoundOnJoinRoomPacket.create(fup(Integer.MIN_VALUE, "", true)));
        cb("OnJoinRoom.max", ClientBoundOnJoinRoomPacket.create(fup(Integer.MAX_VALUE, rep("n", 200), false)));
    }

    interface Failer<T extends ClientBoundPacket> {
        T failed(String message);
    }

    static <T extends ClientBoundPacket> void voidPacket(String name, T success, Failer<T> failer) {
        cb(name + ".success", success);
        cb(name + ".failed_empty", failer.failed(""));
        cb(name + ".failed_msg", failer.failed("operation failed"));
        cb(name + ".failed_zh", failer.failed("没有权限"));
        cb(name + ".failed_emoji", failer.failed("🚫 nope"));
        cb(name + ".failed_long", failer.failed(rep("long ", 60)));
    }

    // ---------------------------------------------------------------- messages

    static List<Named<Message>> messages() {
        List<Named<Message>> m = new ArrayList<>();
        m.add(new Named<>("Chat.empty", new ChatMessage(0, "")));
        m.add(new Named<>("Chat.system", new ChatMessage(-1, "欢迎加入 Zenith 音游战队 服务器！")));
        m.add(new Named<>("Chat.user", new ChatMessage(42, "hello")));
        m.add(new Named<>("Chat.emoji", new ChatMessage(Integer.MAX_VALUE, EMOJI)));
        m.add(new Named<>("Chat.long", new ChatMessage(Integer.MIN_VALUE, rep("长", 400))));
        m.add(new Named<>("CreateRoom.0", new CreateRoomMessage(0)));
        m.add(new Named<>("CreateRoom.1", new CreateRoomMessage(1)));
        m.add(new Named<>("CreateRoom.neg", new CreateRoomMessage(-123456)));
        m.add(new Named<>("JoinRoom.a", new JoinRoomMessage(1, "alice")));
        m.add(new Named<>("JoinRoom.zh", new JoinRoomMessage(2, "鲍勃")));
        m.add(new Named<>("JoinRoom.empty", new JoinRoomMessage(Integer.MIN_VALUE, "")));
        m.add(new Named<>("LeaveRoom.a", new LeaveRoomMessage(1, "alice")));
        m.add(new Named<>("LeaveRoom.emoji", new LeaveRoomMessage(-2, "🎮")));
        m.add(new Named<>("NewHost.a", new NewHostMessage(5)));
        m.add(new Named<>("NewHost.neg", new NewHostMessage(-5)));
        m.add(new Named<>("SelectChart.a", new SelectChartMessage(1, "Chart A", 1234)));
        m.add(new Named<>("SelectChart.zh", new SelectChartMessage(2, "谱面 ✨", -7)));
        m.add(new Named<>("SelectChart.empty", new SelectChartMessage(0, "", 0)));
        m.add(new Named<>("GameStart.a", new GameStartMessage(3)));
        m.add(new Named<>("Ready.a", new ReadyMessage(4)));
        m.add(new Named<>("CancelReady.a", new CancelReadyMessage(5)));
        m.add(new Named<>("CancelGame.a", new CancelGameMessage(6)));
        m.add(new Named<>("StartPlaying", StartPlayingMessage.INSTANCE));
        m.add(new Named<>("Played.a", new PlayedMessage(1, 1000000, 99.5f, true)));
        m.add(new Named<>("Played.zero", new PlayedMessage(0, 0, 0f, false)));
        m.add(new Named<>("Played.neg", new PlayedMessage(-1, -1, -0f, true)));
        m.add(new Named<>("Played.nan", new PlayedMessage(2, Integer.MAX_VALUE, NaN, false)));
        m.add(new Named<>("Played.nan_payload", new PlayedMessage(2, Integer.MIN_VALUE, NAN_PAYLOAD, false)));
        m.add(new Named<>("Played.snan", new PlayedMessage(2, 5, SNAN, true)));
        m.add(new Named<>("Played.inf", new PlayedMessage(3, 5, INF, true)));
        m.add(new Named<>("Played.ninf", new PlayedMessage(3, 5, NINF, false)));
        m.add(new Named<>("Played.denormal", new PlayedMessage(3, 5, Float.MIN_VALUE, false)));
        m.add(new Named<>("Played.max", new PlayedMessage(3, 5, Float.MAX_VALUE, true)));
        m.add(new Named<>("GameEnd", GameEndMessage.INSTANCE));
        m.add(new Named<>("Abort.a", new AbortMessage(7)));
        m.add(new Named<>("LockRoom.true", new LockRoomMessage(true)));
        m.add(new Named<>("LockRoom.false", new LockRoomMessage(false)));
        m.add(new Named<>("CycleRoom.true", new CycleRoomMessage(true)));
        m.add(new Named<>("CycleRoom.false", new CycleRoomMessage(false)));
        return m;
    }

    // ---------------------------------------------------------------- data types

    static void buildTypes() {
        type("UserProfile.simple", up(1, "alice"), UserProfile::decode);
        type("UserProfile.empty", up(0, ""), UserProfile::decode);
        type("UserProfile.zh", up(-1, ZH), UserProfile::decode);
        type("UserProfile.emoji", up(Integer.MAX_VALUE, EMOJI), UserProfile::decode);
        type("UserProfile.min", up(Integer.MIN_VALUE, MIX), UserProfile::decode);
        type("UserProfile.long", up(5, rep("n", 1000)), UserProfile::decode);

        type("FullUserProfile.player", fup(1, "alice", false), FullUserProfile::decode);
        type("FullUserProfile.monitor", fup(2, "观察者", true), FullUserProfile::decode);
        type("FullUserProfile.empty", fup(0, "", false), FullUserProfile::decode);

        List<RoomInfo> rooms = rooms();
        for (int i = 0; i < rooms.size(); i++) type("RoomInfo." + i, rooms.get(i), RoomInfo::decode);

        List<GameState> states = states();
        for (int i = 0; i < states.size(); i++) type("GameState." + i, states.get(i), GameState::decode);

        for (Named<Message> m : messages()) type("Message." + m.name(), m.value(), Message::decode);

        for (Judgement j : Judgement.values()) type("Judgement." + j.name(), j, Judgement::decode);
        for (int i = 0; i < 6; i++) {
            type("JudgeEvent." + i, je(TIMES[i], i - 3, i * 100, Judgement.values()[i]), JudgeEvent::decode);
        }

        for (int i = 0; i < F16.length; i++) {
            type("CompactPos." + i, new CompactPos(F16[i], F16[(i * 5 + 3) % F16.length]), CompactPos::decode);
        }
        type("TouchPoint.a", tp(0, 0.5f, -0.5f), TouchPoint::decode);
        type("TouchPoint.neg", tp(-128, -1f, 1f), TouchPoint::decode);
        type("TouchPoint.max", tp(127, 65504f, 0f), TouchPoint::decode);
        type("TouchFrame.empty", tf(0f), TouchFrame::decode);
        type("TouchFrame.points", tf(2.5f, tp(1, 0.25f, 0.75f), tp(2, -0.25f, -0.75f)), TouchFrame::decode);
        type("TouchFrame.nan_time", tf(NaN, tp(1, NaN, INF)), TouchFrame::decode);

        type("PacketResult.void_success", PacketResult.successVoid(), PacketResultVoid::decode);
        type("PacketResult.void_failed", PacketResult.<EncodeableVoid>failed("no"), PacketResultVoid::decode);
        type("PacketResult.void_failed_empty", PacketResult.<EncodeableVoid>failed(""), PacketResultVoid::decode);
        type("PacketResult.user_success", PacketResult.success(up(3, "carol")), PacketResultUser::decode);
        type("PacketResult.user_failed", PacketResult.<UserProfile>failed("没有"), PacketResultUser::decode);
    }

    /** Adapters so the generic {@code PacketResult.decode} fits the {@code Function<ByteBuf, Encodeable>} shape. */
    static final class PacketResultVoid {
        static PacketResult<EncodeableVoid> decode(ByteBuf b) {
            return PacketResult.decodeVoid(b);
        }
    }

    static final class PacketResultUser {
        static PacketResult<UserProfile> decode(ByteBuf b) {
            return PacketResult.decode(b, UserProfile::decode);
        }
    }
}
