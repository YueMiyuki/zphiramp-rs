// Records tests/fixtures/java_normal_round.txt with the real Java classes (handlers called
// directly on EmbeddedChannels, as the audit did). Needs H.java and SessionCheck.java from
// target/audit-game/src; run in a directory holding data/chart-pools.json with pool 1 = [1,2]
// (default) and pool 2 = [3,4]. Takes about 11 s of real time (the shortened vote countdown).

import io.netty.channel.embedded.EmbeddedChannel;
import top.rymc.phira.main.data.ChartInfo;
import top.rymc.phira.main.data.GameRecord;
import top.rymc.phira.main.game.room.chart.ChartPool;
import top.rymc.phira.main.network.PlayerConnection;
import top.rymc.phira.main.util.GsonUtil;
import top.rymc.phira.main.util.PhiraFetcher;
import top.rymc.phira.protocol.packet.serverbound.*;
import java.time.OffsetDateTime;
import java.util.*;

public class RoundGold {
  static final StringBuilder OUT = new StringBuilder();
  static Object get(Object o, String name) {
    for (Class<?> c = o.getClass(); c != null; c = c.getSuperclass()) {
      try { java.lang.reflect.Field f = c.getDeclaredField(name); f.setAccessible(true); return f.get(o); }
      catch (NoSuchFieldException e) { } catch (Exception e) { throw new RuntimeException(e); }
    }
    throw new RuntimeException("no field " + name + " in " + o.getClass());
  }
  static String state(Object s) {
    String n = s.getClass().getSimpleName();
    if (n.equals("SelectChart")) { Object id = get(s, "chartId"); return id == null ? "SelectChart" : "SelectChart(" + id + ")"; }
    return n;
  }
  static String users(List<?> us) {
    List<Object[]> l = new ArrayList<>();
    for (Object u : us) { Object p = get(u, "userProfile"); l.add(new Object[]{get(p, "userId"), get(u, "monitor")}); }
    l.sort(Comparator.comparingInt((Object[] x) -> (Boolean) x[1] ? 1 : 0).thenComparingInt(x -> (Integer) x[0]));
    StringBuilder sb = new StringBuilder();
    for (Object[] x : l) { if (sb.length() > 0) sb.append(","); sb.append(x[0]).append((Boolean) x[1] ? "m" : ""); }
    return sb.toString();
  }
  static String result(String name, Object r) {
    Object failed = get(r, "failedMessage");
    return failed == null ? name + " ok" : name + " failed: " + failed;
  }
  static String render(Object p) {
    String n = p.getClass().getSimpleName().replace("ClientBound", "").replace("Packet", "");
    switch (n) {
      case "Message": {
        Object m = get(p, "message");
        String k = m.getClass().getSimpleName().replace("Message", "");
        switch (k) {
          case "Chat": { int u = (Integer) get(m, "user"); String c = (String) get(m, "content"); return u == -1 ? "> " + c : "[" + u + "] " + c; }
          case "JoinRoom": return "msg JoinRoom " + get(m, "user") + " " + get(m, "name");
          case "LeaveRoom": return "msg LeaveRoom " + get(m, "user") + " " + get(m, "name");
          case "SelectChart": return "msg SelectChart " + get(m, "user") + " " + get(m, "name") + " " + get(m, "id");
          case "Ready": case "CancelReady": case "Abort": return "msg " + k + " " + get(m, "user");
          case "StartPlaying": case "GameEnd": return "msg " + k;
          case "CycleRoom": return "msg CycleRoom " + get(m, "cycle");
          default: return "msg " + k;
        }
      }
      case "ChangeState": return "state " + state(get(p, "gameState"));
      case "ChangeHost": return "host " + get(p, "host");
      case "OnJoinRoom": { Object u = get(p, "userProfile"); Object up = get(u, "userProfile");
        return "OnJoinRoom " + get(up, "userId") + " " + get(up, "userName") + ((Boolean) get(u, "monitor") ? " monitor" : ""); }
      case "JoinRoom": { Object r = get(p, "result"); Object d = get(r, "result");
        if (d == null) return result("JoinRoom", r);
        return "JoinRoom ok " + state(get(d, "gameState")) + " [" + users((List<?>) get(d, "users")) + "] live=" + get(d, "live"); }
      case "Authenticate": { Object r = get(p, "result"); Object d = get(r, "result");
        if (d == null) return result("Authenticate", r);
        Object up = get(get(d, "userProfile"), "userProfile");
        return "Authenticate ok " + get(up, "userId") + " " + get(up, "userName") + " room=-"; }
      case "Pong": return "Pong";
      default: return result(n, get(p, "result"));
    }
  }
  static void dump(String who, EmbeddedChannel ch) {
    OUT.append("== ").append(who).append("\n");
    Object o;
    while ((o = ch.readOutbound()) != null) OUT.append(render(o).replace("\n", "\\n")).append("\n");
  }
  public static void main(String[] a) throws Exception {
    H.initServerArgs();
    String[] names = {"", "Alpha", "Beta", "Gamma", "Delta"};
    String[] levels = {"", "IN 12", "AT 14", "HD 9", "EZ 3"};
    PhiraFetcher.GET_CHART_INFO = id -> GsonUtil.getGson().fromJson("{\"id\":" + id + ",\"name\":\"" + names[id] + "\",\"level\":\"" + levels[id] + "\"}", ChartInfo.class);
    ChartPool.preload();
    PhiraFetcher.GET_RECORD_INFO = id -> {
      GameRecord r = new GameRecord();
      r.setId(id); r.setChart(1); r.setPerfect(100); r.setGood(2); r.setBad(1); r.setMiss(0);
      r.setTime(OffsetDateTime.now());
      if (id == 9001) { r.setPlayer(100); r.setScore(990000); r.setAccuracy(0.987654f); r.setStd(0.0123456f); r.setStdScore(95.5f); }
      else { r.setPlayer(200); r.setScore(950000); r.setAccuracy(0.993f); r.setStd(0.0071f); r.setStdScore(90.5f); }
      return r;
    };
    PhiraFetcher.GET_RECENT_RECORDS = id -> List.of();
    EmbeddedChannel chA = new EmbeddedChannel(), chB = new EmbeddedChannel();
    PlayerConnection cA = SessionCheck.login(chA, 100, "Alice", 40000);
    dump("A login", chA);
    PlayerConnection cB = SessionCheck.login(chB, 200, "Bob", 40001);
    dump("B login", chB);
    ServerBoundCreateRoomPacket.create("r1").handle(cA.getPacketHandler());
    dump("A create", chA);
    ServerBoundJoinRoomPacket.create("r1", false).handle(cB.getPacketHandler());
    Thread.sleep(300);
    dump("B join", chB); dump("A on B join", chA);
    ServerBoundSelectChartPacket.create(1).handle(cA.getPacketHandler());
    dump("A vote", chA); dump("B on A vote", chB);
    ServerBoundSelectChartPacket.create(1).handle(cB.getPacketHandler());
    dump("B vote", chB); dump("A on B vote", chA);
    Thread.sleep(10500);
    dump("A countdown", chA); dump("B countdown", chB);
    ServerBoundReadyPacket.INSTANCE.handle(cA.getPacketHandler());
    dump("A ready", chA); dump("B on A ready", chB);
    ServerBoundReadyPacket.INSTANCE.handle(cB.getPacketHandler());
    dump("B ready", chB); dump("A on B ready", chA);
    ServerBoundPlayedPacket.create(9001).handle(cA.getPacketHandler());
    dump("A played", chA); dump("B on A played", chB);
    ServerBoundPlayedPacket.create(9002).handle(cB.getPacketHandler());
    dump("B played", chB); dump("A on B played", chA);
    java.nio.file.Files.writeString(java.nio.file.Path.of("transcript.txt"), OUT.toString());
    System.exit(0);
  }
}
