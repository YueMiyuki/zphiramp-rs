import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import io.netty.channel.ChannelHandlerContext;
import io.netty.channel.ChannelInboundHandlerAdapter;
import io.netty.channel.embedded.EmbeddedChannel;
import top.rymc.phira.protocol.PacketRegistry;
import top.rymc.phira.protocol.codec.decoder.ClientPacketDecoder;
import top.rymc.phira.protocol.codec.decoder.FrameDecoder;
import top.rymc.phira.protocol.codec.decoder.HandshakeDecoder;
import top.rymc.phira.protocol.codec.decoder.ServerPacketDecoder;
import top.rymc.phira.protocol.codec.encoder.ClientPacketEncoder;
import top.rymc.phira.protocol.codec.encoder.FrameEncoder;
import top.rymc.phira.protocol.codec.encoder.ServerPacketEncoder;
import top.rymc.phira.protocol.packet.ClientBoundPacket;
import top.rymc.phira.protocol.packet.ServerBoundPacket;

import java.util.ArrayList;
import java.util.List;

/**
 * Replays byte streams through the real Netty pipeline of the server (and of a client) and records
 * what comes out the other end, the way {@code PlayerConnection} would see it.
 *
 * <p>The tail handler mirrors {@code PlayerConnection}: it records packets, and an exception only if
 * the channel is still active when it arrives ({@code PlayerConnection.exceptionCaught} returns early
 * on a channel that is already closed), then closes the channel like the real one does.
 */
final class Streams {
    private Streams() {}

    record Result(List<String> events, boolean closed, List<byte[]> packets, String firstError, int eofEvents) {
        String render() {
            return String.join(";", events) + ";" + (closed ? "CLOSED" : "OPEN");
        }

        String summary() {
            byte[] all = Util.concat(packets.toArray(new byte[0][]));
            return packets.size() + "." + (closed ? "C" : "O") + "." + (firstError == null ? "-" : firstError) + "." + Util.crc(all) + "." + eofEvents;
        }
    }

    static final class Tail extends ChannelInboundHandlerAdapter {
        final boolean serverBound;
        final List<String> ev;
        final List<byte[]> packets;
        String firstError;

        Tail(boolean serverBound, List<String> ev, List<byte[]> packets) {
            this.serverBound = serverBound;
            this.ev = ev;
            this.packets = packets;
        }

        @Override
        public void channelRead(ChannelHandlerContext ctx, Object msg) {
            ByteBuf b = serverBound
                    ? PacketRegistry.ServerBound.encode((ServerBoundPacket) msg, Unpooled::buffer)
                    : PacketRegistry.ClientBound.encode((ClientBoundPacket) msg, Unpooled::buffer);
            byte[] a = Util.bytes(b);
            packets.add(a);
            ev.add("P:" + Util.hex(a));
        }

        @Override
        public void exceptionCaught(ChannelHandlerContext ctx, Throwable t) {
            if (!ctx.channel().isActive()) {
                return;
            }
            String c = Util.code(t);
            ev.add("E:" + t.getClass().getSimpleName());
            if (firstError == null) firstError = c;
            ctx.close();
        }
    }

    /**
     * @param serverSide true: the server pipeline (decodes ServerBound packets); false: the client one.
     * @param handshake  true: start with {@code HandshakeDecoder}, wired up the way {@code ServerChannelInitializer} does.
     */
    static Result run(boolean serverSide, boolean handshake, List<byte[]> chunks) {
        List<String> ev = new ArrayList<>();
        List<byte[]> packets = new ArrayList<>();
        Tail tail = new Tail(serverSide, ev, packets);
        EmbeddedChannel ch;
        if (!handshake) {
            ch = serverSide
                    ? new EmbeddedChannel(new FrameDecoder(), new FrameEncoder(), new ServerPacketDecoder(), new ServerPacketEncoder(), tail)
                    : new EmbeddedChannel(new FrameDecoder(), new FrameEncoder(), new ClientPacketDecoder(), new ClientPacketEncoder(), tail);
        } else {
            // Like ServerChannelInitializer: the decoder is in the pipeline before the channel turns active.
            HandshakeDecoder hd = new HandshakeDecoder();
            ch = new EmbeddedChannel(hd);
            hd.getClientProtocolVersion().whenComplete((v, t) -> {
                if (t != null) {
                    ev.add("H:" + t.getClass().getName() + ": " + t.getMessage());
                    if (ch.isActive()) ch.close();
                    return;
                }
                ev.add("V:" + v);
                ch.pipeline().addLast(new FrameDecoder()).addLast(new FrameEncoder())
                        .addLast(new ServerPacketDecoder()).addLast(new ServerPacketEncoder()).addLast(tail);
            });
        }
        for (byte[] c : chunks) {
            if (!ch.isOpen()) {
                break; // a closed socket delivers nothing more
            }
            try {
                ch.writeInbound(Unpooled.wrappedBuffer(c));
            } catch (Throwable t) {
                ev.add("THROWN:" + t);
            }
            ch.runPendingTasks();
        }
        boolean closedByPipeline = !ch.isActive();
        int before = ev.size();
        if (!closedByPipeline) {
            // The peer hangs up: record anything the pipeline still does about leftover bytes.
            try {
                ch.close();
                ch.runPendingTasks();
            } catch (Throwable t) {
                ev.add("THROWN:" + t);
            }
        }
        int eofEvents = ev.size() - before;
        List<String> events = new ArrayList<>(ev.subList(0, before));
        for (String e : ev.subList(before, ev.size())) events.add("EOF:" + e);
        Result result = new Result(events, closedByPipeline, new ArrayList<>(packets), tail.firstError, eofEvents);
        try {
            ch.finishAndReleaseAll();
        } catch (Throwable ignored) {
            // cleanup only
        }
        return result;
    }

    /** The handshake outcome events when nothing arrives for longer than the timeout, then when the peer hangs up first. */
    static String handshakeTimeout() {
        List<String> ev = new ArrayList<>();
        EmbeddedChannel ch = handshakeChannel(ev, new Tail(true, ev, new ArrayList<>()));
        ch.advanceTimeBy(5001, java.util.concurrent.TimeUnit.MILLISECONDS);
        ch.runScheduledPendingTasks();
        ch.runPendingTasks();
        return String.join(";", ev) + ";" + (!ch.isActive() ? "CLOSED" : "OPEN");
    }

    static String handshakeEof() {
        List<String> ev = new ArrayList<>();
        EmbeddedChannel ch = handshakeChannel(ev, new Tail(true, ev, new ArrayList<>()));
        ch.close();
        ch.runPendingTasks();
        return String.join(";", ev) + ";" + (!ch.isActive() ? "CLOSED" : "OPEN");
    }

    private static EmbeddedChannel handshakeChannel(List<String> ev, Tail tail) {
        HandshakeDecoder hd = new HandshakeDecoder();
        EmbeddedChannel ch = new EmbeddedChannel(hd);
        hd.getClientProtocolVersion().whenComplete((v, t) -> {
            if (t != null) {
                ev.add("H:" + t.getClass().getName() + ": " + t.getMessage());
                if (ch.isActive()) ch.close();
                return;
            }
            ev.add("V:" + v);
        });
        return ch;
    }
}
