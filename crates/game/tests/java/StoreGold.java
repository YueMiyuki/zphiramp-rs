// Records tests/fixtures/rooms_flush_java.json with the real Java RoomStore. Compile and run
// against target/audit-game (see its run.sh): javac -cp "$D/out:$(cat $D/cp.txt)", run in an
// empty directory, then copy data/rooms.json.

import top.rymc.phira.main.Server;
import top.rymc.phira.main.config.ServerArgs;
import top.rymc.phira.main.game.room.RoomStore;
import top.rymc.phira.main.game.room.chart.ChartPool;
import top.rymc.phira.main.game.room.chart.PoolCategory;
import top.rymc.phira.main.game.room.local.LocalRoom;
import top.rymc.phira.main.game.room.local.LocalRoomBuilder;
import java.lang.reflect.Field;
import java.nio.file.*;
import java.util.*;
public class StoreGold {
  public static void main(String[] a) throws Exception {
    Field f = Server.class.getDeclaredField("args"); f.setAccessible(true);
    f.set(Server.getInstance(), new ServerArgs(new String[0]));
    Field ci = ChartPool.class.getDeclaredField("CHART_INFOS"); ci.setAccessible(true);
    @SuppressWarnings("unchecked") Map<Integer, top.rymc.phira.main.data.ChartInfo> infos = (Map<Integer, top.rymc.phira.main.data.ChartInfo>) ci.get(null);
    for (int id = 1; id <= 4; id++) infos.put(id, top.rymc.phira.main.util.GsonUtil.getGson().fromJson("{\"id\":" + id + ",\"name\":\"C\",\"level\":\"IN\"}", top.rymc.phira.main.data.ChartInfo.class));
    ChartPool.PoolSnapshot p1 = new ChartPool.PoolSnapshot(1, null, true, List.of(1,2), PoolCategory.MANUAL, null, null, 0, false);
    ChartPool.PoolSnapshot p2 = new ChartPool.PoolSnapshot(2, null, false, List.of(3,4), PoolCategory.MANUAL, null, null, 1, false);
    new LocalRoomBuilder().host(false).cycle(false).chat(true).autoDestroy(false).pools(List.of(p1)).build("zeta");
    LocalRoom alpha = new LocalRoomBuilder().host(false).cycle(false).chat(true).autoDestroy(false).pools(List.of(p1, p2)).build("alpha");
    alpha.getSetting().setAdminIds(new LinkedHashSet<>(List.of(9, 3)));
    alpha.getChartPool().switchPool(2);
    RoomStore.save();
    RoomStore.flush();
    System.out.print(Files.readString(Path.of("data", "rooms.json")));
    System.exit(0);
  }
}
