use std::sync::Arc;

use zphira_catalog::{
    ApproveOutcome, ChartIndex, ChartPool, PoolCategory, ScreeningRule, SubmissionView, Submissions,
};
use zphira_common::java_fmt::{self, JavaFloat};
use zphira_game::{
    CreateRoomError, EndRoundError, GameContext, RemoveRoomError, RoomPools, RoomUpdate, RoomView,
    SwitchPoolError,
};
use zphira_ledger::Admins;

pub trait Out: Send + Sync + 'static {
    fn info(&self, message: String);
    fn warn(&self, message: String);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LogOut;

impl Out for LogOut {
    fn info(&self, message: String) {
        tracing::info!("{}", message);
    }

    fn warn(&self, message: String) {
        tracing::warn!("{}", message);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Stop,
}

#[derive(Clone)]
pub struct Services {
    pub game: GameContext,
    pub pools: ChartPool,
    pub index: ChartIndex,
    pub submissions: Submissions,
    pub admins: Admins,
}

#[derive(Clone)]
pub struct Commands {
    svc: Services,
    out: Arc<dyn Out>,
}

const USAGE_ROOM: &str = "Usage: room list | room create <id> [poolId...] | room remove <id> | room <id> end | room <id> clear | room <id> config <key> <value> | room <id> pool switch <poolId> | room <id> pool favorite <favoriteId|none>";
const USAGE_ADMIN: &str = "Usage: admin list | admin add <userId> | admin remove <userId>";
const USAGE_SUBMISSION: &str = "Usage: submission list | submission list <poolId> | submission approve <poolId> <chartId> | submission reject <poolId> <chartId> [reason]";
const USAGE_CHART: &str = "Usage: chart index | chart refresh [plain|full] | chart search <category> [limit] | chart duration <chartIds...>";
const USAGE_POOL: &str = "Usage: pool list | pool add <id> [chartIds...] | pool remove <id> | pool chart add <poolId> <chartId> | pool chart remove <poolId> <chartId> | pool favorite <poolId> <favoriteId|none> | pool default <poolId> <on|off> | pool meta <poolId> <category> [sizeLimit] [roundsPerStay] [order] | pool generate <category> [sizeLimit] [roundsPerStay] [probeDuration]";

const CONFIG_KEYS: [&str; 9] = [
    "minPlayer",
    "maxPlayer",
    "selectCountdown",
    "readyCountdown",
    "forceFinish",
    "interval",
    "cancelReady",
    "startAllReady",
    "allReadyDelay",
];

const CATEGORY_CLASS: &str = "top.rymc.phira.main.game.room.chart.PoolCategory";

type CmdResult = Result<bool, String>;

pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Trims the line; `None` when it is blank.
pub fn normalize(input: &str) -> Option<&str> {
    let command = java_trim(input);
    (!command.is_empty()).then_some(command)
}

pub fn is_stop(command: &str) -> bool {
    eq_ic(command, "stop")
}

fn is_regex_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

pub fn java_split(s: &str, limit: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !is_regex_space(c) {
            continue;
        }
        let mut end = i + c.len_utf8();
        while let Some(&(j, d)) = chars.peek() {
            if !is_regex_space(d) {
                break;
            }
            end = j + d.len_utf8();
            chars.next();
        }
        if limit > 0 && parts.len() + 1 == limit {
            break;
        }
        parts.push(&s[start..i]);
        start = end;
    }
    parts.push(&s[start..]);
    if limit == 0 {
        while parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
            parts.pop();
        }
    }
    parts
}

fn upper1(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

fn lower1(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

pub fn eq_ic(a: &str, b: &str) -> bool {
    let mut x = a.chars();
    let mut y = b.chars();
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(c), Some(d)) => {
                if c != d {
                    let (uc, ud) = (upper1(c), upper1(d));
                    if uc != ud && lower1(uc) != lower1(ud) {
                        return false;
                    }
                }
            }
            _ => return false,
        }
    }
}

fn parse_int(value: &str) -> Result<i32, String> {
    java_fmt::parse_int(value).map_err(|e| e.message().to_owned())
}

fn parse_chart_ids(args: &[&str]) -> Result<Vec<i32>, String> {
    args.iter().map(|a| parse_int(a)).collect()
}

fn parse_switch(value: &str, name: &str) -> Result<bool, String> {
    if eq_ic(value, "on") || eq_ic(value, "true") || value == "1" {
        Ok(true)
    } else if eq_ic(value, "off") || eq_ic(value, "false") || value == "0" {
        Ok(false)
    } else {
        Err(format!("Unknown {name} value: {value}. Use on or off"))
    }
}

fn parse_category(raw: &str) -> Result<PoolCategory, String> {
    let upper = java_trim(raw).to_uppercase();
    PoolCategory::from_name(&upper)
        .ok_or_else(|| format!("No enum constant {CATEGORY_CLASS}.{upper}"))
}

fn or_null<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_owned(), |v| v.to_string())
}

fn java_list(ids: &[i32]) -> String {
    let parts: Vec<String> = ids.iter().map(i32::to_string).collect();
    format!("[{}]", parts.join(", "))
}

async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())
}

impl Commands {
    pub fn new(services: Services, out: Arc<dyn Out>) -> Self {
        Self { svc: services, out }
    }

    fn info(&self, message: String) {
        self.out.info(message);
    }

    fn warn(&self, message: String) {
        self.out.warn(message);
    }

    pub async fn run(&self, command: &str) -> Flow {
        if is_stop(command) {
            return Flow::Stop;
        }
        let lower = command.to_lowercase();
        if lower.starts_with("room ") {
            let args = java_split(command, 0);
            self.finish("Room", USAGE_ROOM, self.room(&args).await);
        } else if lower.starts_with("pool ") {
            let args = java_split(command, 0);
            self.finish("Pool", USAGE_POOL, self.pool(&args).await);
        } else if lower.starts_with("admin ") {
            let args = java_split(command, 0);
            self.finish("Admin", USAGE_ADMIN, self.admin(&args).await);
        } else if lower.starts_with("chart ") {
            let args = java_split(command, 0);
            self.finish("Chart", USAGE_CHART, self.chart(&args).await);
        } else if lower.starts_with("submission ") {
            let args = java_split(command, 5);
            self.finish("Submission", USAGE_SUBMISSION, self.submission(&args).await);
        } else if lower.starts_with("say ") {
            // The prefix matched in lower case, so its four characters are ASCII.
            let message = java_trim(&command[4..]);
            self.svc.game.say_to_all(message);
        } else {
            self.warn(format!("Unknown command: {command}"));
        }
        Flow::Continue
    }

    fn finish(&self, group: &str, usage: &str, result: CmdResult) {
        match result {
            Ok(true) => {}
            Ok(false) => self.warn(usage.to_owned()),
            Err(message) => self.warn(format!("{group} command failed: {message}")),
        }
    }

    async fn admin(&self, a: &[&str]) -> CmdResult {
        let admins = self.svc.admins.clone();
        if a.len() == 2 && eq_ic(a[1], "list") {
            let list = blocking(move || admins.list()).await?;
            if list.is_empty() {
                self.info("No admins".to_owned());
            } else {
                self.info(format!("Admins: {}", java_list(&list)));
            }
            return Ok(true);
        }
        if a.len() == 3 && eq_ic(a[1], "add") {
            let user_id = parse_int(a[2])?;
            let added = blocking(move || admins.add(user_id))
                .await?
                .map_err(|e| e.to_string())?;
            if added {
                self.info(format!("Added admin {user_id}"));
            } else {
                self.warn(format!("User {user_id} is already an admin"));
            }
            return Ok(true);
        }
        if a.len() == 3 && eq_ic(a[1], "remove") {
            let user_id = parse_int(a[2])?;
            let removed = blocking(move || admins.remove(user_id))
                .await?
                .map_err(|e| e.to_string())?;
            if removed {
                self.info(format!("Removed admin {user_id}"));
            } else {
                self.warn(format!("User {user_id} is not an admin"));
            }
            return Ok(true);
        }
        Ok(false)
    }

    async fn room(&self, a: &[&str]) -> CmdResult {
        let n = a.len();
        if n == 2 && eq_ic(a[1], "list") {
            self.list_rooms();
            return Ok(true);
        }
        if n >= 3 && eq_ic(a[1], "create") {
            self.create_room(a)?;
            return Ok(true);
        }
        if n == 3 && eq_ic(a[1], "remove") {
            self.remove_room(a[2])?;
            return Ok(true);
        }
        if n >= 3 {
            let room_id = a[1];
            if eq_ic(a[2], "end") {
                self.force_end_room(room_id).await?;
                return Ok(true);
            }
            if eq_ic(a[2], "clear") && n == 3 {
                self.clear_room(room_id).await?;
                return Ok(true);
            }
            if eq_ic(a[2], "config") && n == 5 {
                self.set_room_config(room_id, a[3], a[4])?;
                return Ok(true);
            }
            if eq_ic(a[2], "pool") {
                if n == 5 && eq_ic(a[3], "switch") {
                    // The pool number is read before the room is looked up.
                    let pool_id = parse_int(a[4])?;
                    self.switch_room_pool(room_id, pool_id)?;
                    return Ok(true);
                }
                if n == 5 && eq_ic(a[3], "favorite") {
                    self.set_room_pool_favorite(room_id, a[4])?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn require_room(&self, room_id: &str) -> Result<(), String> {
        if self.svc.game.room_exists(room_id) {
            Ok(())
        } else {
            Err(format!("Room not found: {room_id}"))
        }
    }

    fn list_rooms(&self) {
        let rooms = self.svc.game.rooms();
        if rooms.is_empty() {
            self.info("No rooms".to_owned());
            return;
        }
        for room in rooms {
            self.info(room_line(&room));
        }
    }

    fn create_room(&self, a: &[&str]) -> Result<(), String> {
        let room_id = a[2];
        let game = &self.svc.game;
        if !game.is_valid_room_id(room_id) {
            return Err("Room id must be letters, digits, - or _ with length <= 20".to_owned());
        }
        if game.room_exists(room_id) {
            return Err(format!("Room already exists: {room_id}"));
        }
        let pools = if a.len() > 3 {
            let mut ids = Vec::new();
            for raw in &a[3..] {
                let pool_id = parse_int(raw)?;
                if self.svc.pools.find_pool(pool_id).is_none() {
                    return Err(format!("Pool not found: {pool_id}"));
                }
                ids.push(pool_id);
            }
            RoomPools::Exact(ids)
        } else {
            RoomPools::Default
        };
        game.create_room(room_id, pools)
            .map_err(|e: CreateRoomError| e.console_message(room_id))?;
        self.info(format!("Created room {room_id}"));
        Ok(())
    }

    fn remove_room(&self, room_id: &str) -> Result<(), String> {
        self.svc.game.remove_room(room_id).map_err(|e| match e {
            RemoveRoomError::RoomNotFound => format!("Room not found: {room_id}"),
            RemoveRoomError::NotEmpty => "Room is not empty, cannot remove".to_owned(),
        })?;
        self.info(format!("Removed room {room_id}"));
        Ok(())
    }

    async fn force_end_room(&self, room_id: &str) -> Result<(), String> {
        self.svc
            .game
            .end_round(room_id)
            .await
            .map_err(|e| match e {
                EndRoundError::RoomNotFound => format!("Room not found: {room_id}"),
                EndRoundError::NotPlaying => "Room is not playing".to_owned(),
            })?;
        self.info(format!("Force ended current game in room {room_id}"));
        Ok(())
    }

    async fn clear_room(&self, room_id: &str) -> Result<(), String> {
        let removed = self
            .svc
            .game
            .clear_room(room_id)
            .await
            .map_err(|_| format!("Room not found: {room_id}"))?;
        self.info(format!(
            "Cleared room {room_id}: {removed} member(s) removed"
        ));
        Ok(())
    }

    fn set_room_config(&self, room_id: &str, key: &str, value: &str) -> Result<(), String> {
        self.require_room(room_id)?;
        let mut update = RoomUpdate::default();
        match key.to_lowercase().as_str() {
            "cancelready" => {
                update.cancel_ready_in_vote = Some(parse_switch(value, "cancelReady")?);
            }
            "startallready" => {
                update.start_when_all_ready = Some(parse_switch(value, "startAllReady")?);
            }
            other => {
                let int_value = parse_int(value)?;
                match other {
                    "minplayer" => update.min_player = Some(int_value),
                    "maxplayer" => update.max_player = Some(int_value),
                    "selectcountdown" => update.select_countdown = Some(int_value),
                    "readycountdown" => update.ready_countdown = Some(int_value),
                    "forcefinish" => update.force_finish = Some(int_value),
                    "interval" => update.interval = Some(int_value),
                    "allreadydelay" => update.all_ready_delay = Some(int_value),
                    _ => {
                        return Err(format!(
                            "Unknown config key: {key}. Keys: {}",
                            CONFIG_KEYS.join(", ")
                        ));
                    }
                }
            }
        }
        self.svc
            .game
            .update_room(room_id, &update)
            .map_err(|e| e.console_message(room_id))?;
        self.info(format!("Room {room_id} config {key} = {value}"));
        Ok(())
    }

    fn switch_room_pool(&self, room_id: &str, pool_id: i32) -> Result<(), String> {
        self.svc
            .game
            .switch_room_pool(room_id, pool_id)
            .map_err(|e| match e {
                SwitchPoolError::RoomNotFound => format!("Room not found: {room_id}"),
                SwitchPoolError::NotInRoom(message) => message,
            })?;
        self.info(format!("Room {room_id} pending pool set to {pool_id}"));
        Ok(())
    }

    fn set_room_pool_favorite(&self, room_id: &str, value: &str) -> Result<(), String> {
        self.require_room(room_id)?;
        let favorite = if eq_ic(value, "none") {
            None
        } else {
            Some(parse_int(value)?)
        };
        self.svc
            .game
            .set_room_favorite(room_id, favorite)
            .map_err(|_| format!("Room not found: {room_id}"))?;
        match favorite {
            None => self.info(format!(
                "Cleared favorite_id for room {room_id} current pool"
            )),
            Some(id) => self.info(format!(
                "Set favorite_id for room {room_id} current pool to {id}"
            )),
        }
        Ok(())
    }

    async fn pool(&self, a: &[&str]) -> CmdResult {
        let n = a.len();
        if n == 2 && eq_ic(a[1], "list") {
            self.list_pools()?;
            return Ok(true);
        }
        if n >= 3 && eq_ic(a[1], "add") {
            let pool_id = parse_int(a[2])?;
            let chart_ids = parse_chart_ids(&a[3..])?;
            self.svc
                .pools
                .add_pool(pool_id, &chart_ids)
                .await
                .map_err(|e| e.message())?;
            self.info(format!(
                "Added chart pool {pool_id} with {} chart(s)",
                chart_ids.len()
            ));
            return Ok(true);
        }
        if n == 3 && eq_ic(a[1], "remove") {
            let pool_id = parse_int(a[2])?;
            self.svc
                .pools
                .remove_pool(pool_id)
                .await
                .map_err(|e| e.message())?;
            self.info(format!("Removed chart pool {}", a[2]));
            return Ok(true);
        }
        if n == 5 && eq_ic(a[1], "chart") && eq_ic(a[2], "add") {
            let pool_id = parse_int(a[3])?;
            let chart_id = parse_int(a[4])?;
            self.add_chart(pool_id, chart_id).await?;
            return Ok(true);
        }
        if n == 5 && eq_ic(a[1], "chart") && eq_ic(a[2], "remove") {
            let pool_id = parse_int(a[3])?;
            let chart_id = parse_int(a[4])?;
            self.svc
                .pools
                .remove_chart(pool_id, chart_id)
                .await
                .map_err(|e| e.message())?;
            self.info(format!("Removed chart {} from pool {}", a[4], a[3]));
            return Ok(true);
        }
        if n == 4 && eq_ic(a[1], "favorite") {
            let pool_id = parse_int(a[2])?;
            self.set_pool_favorite(pool_id, a[3]).await?;
            return Ok(true);
        }
        if n == 4 && eq_ic(a[1], "default") {
            let pool_id = parse_int(a[2])?;
            self.set_pool_default_flag(pool_id, a[3]).await?;
            return Ok(true);
        }
        if n >= 4 && eq_ic(a[1], "meta") {
            let pool_id = parse_int(a[2])?;
            self.set_pool_metadata(pool_id, a[3], a.get(4), a.get(5), a.get(6))
                .await?;
            return Ok(true);
        }
        if n >= 3 && eq_ic(a[1], "generate") {
            self.generate_pools(a[2], a.get(3), a.get(4), a.get(5))
                .await?;
            return Ok(true);
        }
        Ok(false)
    }

    fn list_pools(&self) -> Result<(), String> {
        for pool in self.svc.pools.list_pools() {
            self.info(format!(
                "Pool {} category={} default={} order={} sizeLimit={} roundsPerStay={} favorite_id={} charts={}:",
                pool.id,
                pool.category,
                pool.default_flag,
                pool.order,
                or_null(pool.size_limit),
                or_null(pool.rounds_per_stay),
                or_null(pool.favorite_id),
                pool.chart_ids.len()
            ));
            for &chart_id in &pool.chart_ids {
                let chart = self
                    .svc
                    .pools
                    .get_chart_info(chart_id)
                    .map_err(|e| e.message())?;
                self.info(format!(
                    "  {} | Lv.{} | ID:{}",
                    chart.name_or_null(),
                    chart.level_or_null(),
                    chart.id
                ));
            }
        }
        Ok(())
    }

    async fn add_chart(&self, pool_id: i32, chart_id: i32) -> Result<(), String> {
        let pools = &self.svc.pools;
        pools
            .add_chart(pool_id, chart_id)
            .await
            .map_err(|e| e.message())?;
        let chart = pools.get_chart_info(chart_id).map_err(|e| e.message())?;
        self.info(format!(
            "Added chart to pool {pool_id}: {} | Lv.{} | ID:{}",
            chart.name_or_null(),
            chart.level_or_null(),
            chart.id
        ));
        Ok(())
    }

    async fn set_pool_favorite(&self, pool_id: i32, value: &str) -> Result<(), String> {
        let favorite = if eq_ic(value, "none") {
            None
        } else {
            Some(parse_int(value)?)
        };
        self.svc
            .pools
            .set_favorite_id(pool_id, favorite)
            .await
            .map_err(|e| e.message())?;
        match favorite {
            None => self.info(format!("Cleared favorite_id for pool {pool_id}")),
            Some(id) => self.info(format!("Set favorite_id for pool {pool_id} to {id}")),
        }
        Ok(())
    }

    async fn set_pool_default_flag(&self, pool_id: i32, value: &str) -> Result<(), String> {
        let flag = parse_switch(value, "default flag")?;
        self.svc
            .pools
            .set_default_flag(pool_id, flag)
            .await
            .map_err(|e| e.message())?;
        self.info(format!("Set default flag for pool {pool_id} to {flag}"));
        Ok(())
    }

    async fn set_pool_metadata(
        &self,
        pool_id: i32,
        category: &str,
        size_limit: Option<&&str>,
        rounds_per_stay: Option<&&str>,
        order: Option<&&str>,
    ) -> Result<(), String> {
        let parsed = parse_category(category)?;
        let size = size_limit.map(|v| parse_int(v)).transpose()?;
        let rounds = rounds_per_stay.map(|v| parse_int(v)).transpose()?;
        let order_value = order.map(|v| parse_int(v)).transpose()?;
        self.svc
            .pools
            .update_pool(pool_id, Some(parsed), size, rounds, order_value, None)
            .await
            .map_err(|e| e.message())?;
        self.info(format!(
            "Pool {pool_id} metadata set: category={parsed} sizeLimit={} roundsPerStay={} order={}",
            or_null(size_limit.copied()),
            or_null(rounds_per_stay.copied()),
            or_null(order.copied())
        ));
        Ok(())
    }

    async fn generate_pools(
        &self,
        category: &str,
        size_limit: Option<&&str>,
        rounds_per_stay: Option<&&str>,
        probe_duration: Option<&&str>,
    ) -> Result<(), String> {
        let parsed = parse_category(category)?;
        let rule = ScreeningRule::of(parsed)
            .ok_or_else(|| format!("Category has no screening rule: {parsed}"))?;
        if let Some(raw) = probe_duration {
            let budget = parse_int(raw)?;
            let relaxed = rule.without_duration();
            // A negative limit is refused with the number as the message.
            let limit = budget.wrapping_mul(4);
            let Ok(limit) = usize::try_from(limit) else {
                return Err(limit.to_string());
            };
            let candidates = self.svc.index.search(&relaxed, limit);
            let probed = self
                .svc
                .index
                .probe_durations(&candidates, usize::try_from(budget).unwrap_or(0))
                .await;
            self.info(format!("Probed {probed} durations for {parsed}"));
        }
        // These two are read after the probe, so a bad number is reported after it ran.
        let size = match size_limit {
            Some(v) => parse_int(v)?,
            None => 15,
        };
        let rounds = rounds_per_stay.map(|v| parse_int(v)).transpose()?;
        let outcome = self
            .svc
            .pools
            .generate_pools(&self.svc.index, &rule, size, rounds)
            .await
            .map_err(|e| {
                if e.is_io_exception() {
                    format!("Pool generation failed: {}", e.message())
                } else {
                    e.message()
                }
            })?;
        self.info(format!(
            "Generated {} {parsed} pools: {}",
            outcome.pool_ids.len(),
            java_list(&outcome.pool_ids)
        ));
        Ok(())
    }

    async fn chart(&self, a: &[&str]) -> CmdResult {
        let n = a.len();
        let index = &self.svc.index;
        if n == 2 && eq_ic(a[1], "index") {
            self.info(format!(
                "Chart index holds {} charts (remote total {}, refreshing {})",
                index.indexed_count(),
                index.remote_count(),
                index.is_refreshing()
            ));
            return Ok(true);
        }
        if (n == 2 || n == 3) && eq_ic(a[1], "refresh") {
            let full = n == 3 && eq_ic(a[2], "full");
            let division = a.get(2).map(|d| (*d).to_owned());
            let started = if full {
                self.info("Refreshing chart index in background (full sweep)".to_owned());
                index.refresh_full_async()
            } else {
                self.info(format!(
                    "Refreshing chart index in background (division={})",
                    division.as_deref().unwrap_or("all")
                ));
                index.refresh_async(division)
            };
            if !started {
                self.warn("A chart index refresh is already running, request ignored".to_owned());
            }
            return Ok(true);
        }
        if n >= 3 && eq_ic(a[1], "search") {
            self.search_charts(a[2], a.get(3).copied())?;
            return Ok(true);
        }
        if n >= 3 && eq_ic(a[1], "duration") {
            let ids = parse_chart_ids(&a[2..])?;
            self.probe_durations(&ids).await?;
            return Ok(true);
        }
        Ok(false)
    }

    fn search_charts(&self, category: &str, limit: Option<&str>) -> Result<(), String> {
        let parsed = parse_category(category)?;
        let rule = ScreeningRule::of(parsed)
            .ok_or_else(|| format!("Category has no screening rule: {parsed}"))?;
        let max = match limit {
            Some(v) => parse_int(v)?,
            None => 20,
        };
        // A negative limit is refused with the number as the message.
        let Ok(max) = usize::try_from(max) else {
            return Err(max.to_string());
        };
        let matched = self.svc.index.search(&rule, max);
        self.info(format!(
            "{parsed} matched {} of {} indexed charts, showing {}:",
            matched.len(),
            self.svc.index.indexed_count(),
            matched.len()
        ));
        for chart in matched {
            self.info(format!(
                "  {} | {} | rating={} votes={} | ID:{}",
                chart.name_or_null(),
                chart.level_or_null(),
                JavaFloat(chart.rating),
                chart.rating_count,
                chart.id
            ));
        }
        Ok(())
    }

    async fn probe_durations(&self, chart_ids: &[i32]) -> Result<(), String> {
        let mut charts = Vec::new();
        for &chart_id in chart_ids {
            match self.svc.index.get(chart_id) {
                Some(chart) => charts.push(chart),
                None => return Err(format!("Chart not in index: {chart_id}")),
            }
        }
        let probed = self.svc.index.probe_durations(&charts, charts.len()).await;
        for chart in &charts {
            let seconds = chart
                .duration_seconds()
                .map_or_else(|| "unknown".to_owned(), |s| format!("{s}s"));
            self.info(format!(
                "  {} | {} -> {}",
                chart.name_or_null(),
                chart.level_or_null(),
                seconds
            ));
        }
        self.info(format!("Probed {probed} of {} chart(s)", chart_ids.len()));
        Ok(())
    }

    async fn submission(&self, a: &[&str]) -> CmdResult {
        let n = a.len();
        let subs = &self.svc.submissions;
        if n == 2 && eq_ic(a[1], "list") {
            self.print_submissions(&subs.list_pending(), "待审投稿");
            return Ok(true);
        }
        if n == 3 && eq_ic(a[1], "list") {
            let pool_id = parse_int(a[2])?;
            self.print_submissions(
                &subs.list_by_pool(pool_id, None),
                &format!("池 {pool_id} 的投稿"),
            );
            return Ok(true);
        }
        if n >= 4 && eq_ic(a[1], "approve") {
            let pool_id = parse_int(a[2])?;
            let chart_id = parse_int(a[3])?;
            match subs
                .approve(&self.svc.pools, pool_id, chart_id, 0)
                .await
                .map_err(|e| e.message())?
            {
                ApproveOutcome::NotFound => return Err("投稿不存在".to_owned()),
                ApproveOutcome::AddFailed(e) => return Err(e.message()),
                ApproveOutcome::Approved => {}
            }
            self.info(format!(
                "Approved chart {chart_id} into pool {pool_id} from console"
            ));
            return Ok(true);
        }
        if n >= 4 && eq_ic(a[1], "reject") {
            let pool_id = parse_int(a[2])?;
            let chart_id = parse_int(a[3])?;
            let reason = a.get(4).copied();
            if !subs
                .review(pool_id, chart_id, false, 0, reason)
                .await
                .map_err(|e| e.message())?
            {
                return Err("投稿不存在".to_owned());
            }
            self.info(format!(
                "Rejected chart {chart_id} in pool {pool_id} from console"
            ));
            return Ok(true);
        }
        Ok(false)
    }

    fn print_submissions(&self, views: &[SubmissionView], title: &str) {
        if views.is_empty() {
            self.info(format!("{title}: 无"));
            return;
        }
        self.info(format!("{title}: {} 条", views.len()));
        for view in views {
            let name = match self.svc.index.get(view.chart_id) {
                Some(chart) => chart.name_or_null().to_owned(),
                None => "-".to_owned(),
            };
            let submitters: Vec<String> = view
                .submitters
                .iter()
                .map(|s| format!("{}({})", s.name.as_deref().unwrap_or("null"), s.user_id))
                .collect();
            self.info(format!(
                "  池{} 谱面{} [{}] {} | {} 人: {}",
                view.pool_id,
                view.chart_id,
                view.status.name(),
                name,
                view.submitters.len(),
                submitters.join(", ")
            ));
        }
    }
}

fn room_line(room: &RoomView) -> String {
    let chart = match &room.chart {
        Some(chart) => chart.name_or_null().to_owned(),
        None => "-".to_owned(),
    };
    format!(
        "Room {} [{}] chart={} players={}/{} monitors={} pool={} rounds={}/{}",
        room.room_id,
        room.state,
        chart,
        room.online_players,
        room.players.len(),
        room.monitors.len(),
        room.pool.current_pool.id,
        room.pool.finished_rounds_since_refresh,
        room.pool.effective_rounds_per_stay
    )
}

#[cfg(test)]
mod tests;
