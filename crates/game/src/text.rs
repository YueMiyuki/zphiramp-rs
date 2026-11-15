use zphira_common::java_fmt::JavaFloat;
use zphira_phira::{ChartInfo, GameRecord};

pub const SEPARATOR: &str = "————————————————————————————————————————";

pub const SYSTEM_SENDER: i32 = -1;

pub const WELCOME: &str = "欢迎加入 Zenith 音游战队 服务器！";
pub const WELCOME_ROOM_HINT: &str = "输入房间名可创建或加入房间，房间名仅限字母、数字、-、_。";

pub fn welcome_player(name: &str, id: i32) -> String {
    format!("玩家：{name}（#{id}）")
}

pub fn points_line(points: i32, rank: usize) -> String {
    format!("当前积分：{points}，积分排名：#{rank}")
}

pub const RANK_ROOM_ID: &str = "rank";
pub const RANKING_SENT: &str = "排行榜已发送";
pub const ROOM_ID_RULE: &str = "房间名仅限字母、数字、-、_。";
pub const NO_DEFAULT_POOL: &str = "当前没有默认启用的谱池，无法创建房间。";
pub const NO_ROOMS: &str = "当前没有房间，可创建一个。";
pub const AVAILABLE_ROOMS: &str = "当前可用房间：";
pub const ROOM_GUIDE_1: &str = "房间玩法：选谱阶段投票 → 准备阶段 → 游戏，票数最高者开局。";
pub const ROOM_GUIDE_2: &str = "投票仅限当前谱池，重复投票会改票，观战者不能投票。";
pub const RANKING_TITLE: &str = "积分排行榜 TOP 10";
pub const RANKING_REFRESH: &str = "排行榜每一分钟刷新一次，可能有滞后。";

pub fn room_not_found(room_id: &str) -> String {
    format!("房间 \"{room_id}\" 不存在。")
}

pub fn room_list_entry(room_id: &str, online: usize, players: usize) -> String {
    format!("- {room_id}（在线 {online}/{players} 人）")
}

pub fn your_points(points: i32, rank: usize) -> String {
    format!("你的积分：{points}，积分排名：#{rank}")
}

pub fn welcome_to_room(name: &str, id: i32, monitor: bool) -> String {
    let who = format!("{name}（#{id}）");
    if monitor {
        format!("{who} 进入房间观战")
    } else {
        format!("欢迎 {who} 加入房间")
    }
}

pub const VOTE_LOCKED: &str = "本轮曲目已锁定，无法继续改票。";
pub const FAVORITE_IMPORT_HINT: &str = "你可以通过导入收藏夹来一键导入谱池";
pub const VOTE_INSTRUCTIONS: &str = "使用选谱操作投票，只能选择下列歌曲。票数最高者开局。";
pub const VOTE_BOARD_HINT: &str = "点击锁定房间按钮查看当前谱池状态。";
pub const COUNTDOWN_CANCELLED: &str = "在线玩家不足，开局倒计时已取消。";
pub const ROUND_CANCELLED_SHORT: &str = "在线玩家不足，本轮取消。";
pub const TIE_BREAK: &str = "最高票出现并列，将随机抽选本轮曲目。";
pub const VOTE_UNLOCKED: &str = "曲目锁定已解除，可继续投票。";

pub fn chart_label(chart: &ChartInfo) -> String {
    format!(
        "{} | Lv.{} | ID:{}",
        chart.name_or_null(),
        chart.level_or_null(),
        chart.id
    )
}

pub fn vote_board_line(votes: usize, chart: &ChartInfo) -> String {
    format!(
        "[{votes}票] {}  |  Lv.{}  |  ID:{}",
        chart.name_or_null(),
        chart.level_or_null(),
        chart.id
    )
}

pub fn pool_header(room_id: &str, pool_id: i32) -> String {
    format!("{room_id} 本轮谱池 #{pool_id}")
}

pub fn favorite_line(favorite_id: i32) -> String {
    format!("谱面收藏夹 ID：{favorite_id}")
}

pub fn voted(name: &str, chart: &ChartInfo) -> String {
    format!("{name} 已投票：{}", chart_label(chart))
}

pub fn early_start(name: &str, chart: &ChartInfo) -> String {
    format!("{name} 提前开始本轮，曲目：{}", chart_label(chart))
}

pub fn countdown_started(seconds: i32) -> String {
    format!("已达到开局人数，{seconds} 秒后锁定投票并进入准备阶段。人数不足会取消倒计时。")
}

pub fn countdown_shortened(seconds: i32) -> String {
    format!("全员已投票，倒计时缩短为 {seconds} 秒，锁定前仍可改票。")
}

pub fn vote_countdown_notice(seconds: i32) -> String {
    format!("投票锁定倒计时：{seconds} 秒")
}

pub fn vote_finished(chart: &ChartInfo) -> String {
    format!("投票结束，本轮曲目：{}", chart_label(chart))
}

pub const READY_NOBODY_ONLINE: &str = "当前没有在线玩家，本轮准备已取消。";
pub const READY_NOBODY_READY: &str = "没有已准备的在线玩家，本轮取消。";
pub const ALL_READY_CALLED_OFF: &str = "有玩家尚未准备，开始已取消。";

pub fn all_ready_start(seconds: i32) -> String {
    format!("全员已准备，{seconds} 秒后开始游戏。")
}

pub fn ready_phase_started(seconds: i32) -> String {
    format!("进入准备阶段：请在 {seconds} 秒内准备。未准备玩家将作为观战跳过本轮。")
}

pub fn ready_countdown_notice(seconds: i32) -> String {
    format!("准备倒计时：{seconds} 秒。")
}

pub const ROOM_IS_PLAYING: &str = "房间当前正在游戏中，请静待游戏结束";
pub const SUBMIT_NETWORK_FAILED_SELF: &str = "网络错误导致成绩提交失败，本轮将视为放弃。";
pub const RANKING_HEADER: &str = "本轮排名";
pub const SUBMIT_INVALID_SELF: &str = "成绩记录无效，本轮将视为放弃。";

pub fn submit_network_failed_room(name: &str) -> String {
    format!("因网络错误导致 {name} 成绩提交失败，已视为放弃本轮。")
}

pub fn submit_invalid_room(name: &str) -> String {
    format!("{name} 提交的成绩记录无效，已视为放弃本轮。")
}

pub fn force_finish_notice(seconds: i32) -> String {
    format!("本轮游戏将在 {seconds} 秒后强制结束。")
}

pub fn played_record(player_id: i32, name: &str, record: &GameRecord) -> String {
    format!(
        "[{}] {} 的分数: {}, 准度: {}%, 误差: ±{}ms, 无暇度分数: {}\n    Perfect: {}, Good: {}, Bad: {}, Miss: {}\n",
        player_id,
        name,
        record.score,
        JavaFloat(record.accuracy * 100.0_f32),
        JavaFloat(record.std * 1000.0_f32),
        JavaFloat(record.std_score),
        record.perfect,
        record.good,
        record.bad,
        record.miss
    )
}

pub fn ranking_line(rank: i32, name: &str, record: &GameRecord, gained: i32, total: i32) -> String {
    format!(
        "{rank}. {name} - 分数: {}, 准度: {}%, 误差: ±{}ms, 积分: +{gained}, 总积分: {total}",
        record.score,
        JavaFloat(record.accuracy * 100.0_f32),
        JavaFloat(record.std * 1000.0_f32),
    )
}

pub fn record_recovered(name: &str, score: i32) -> String {
    format!("{name} 的成绩已从 Phira 同步：{score}")
}

pub fn pool_progress(pool_id: i32, finished: i32, quota: i32) -> String {
    format!("当前谱池 #{pool_id} 已进行 {finished}/{quota} 轮。")
}

pub fn pool_single(pool_id: i32, quota: i32) -> String {
    format!("谱池 #{pool_id} 已完成 {quota} 轮，但本房间只配置了这一个谱池，继续使用。")
}

pub fn pool_rotated(previous: i32, quota: i32, current: i32) -> String {
    format!("谱池 #{previous} 已完成 {quota} 轮，已轮换至谱池 #{current}。")
}

pub const ROOM_CLEARED: &str = "房间已被清空，你已被移出房间。";

pub const CLEAR_COMMAND: &str = "/clear";

pub const KEY_LOGGED_IN_ELSEWHERE: &str = "error.logged_in_elsewhere";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separator_is_forty_em_dashes() {
        assert_eq!(SEPARATOR.chars().count(), 40);
        assert!(SEPARATOR.chars().all(|c| c == '\u{2014}'));
    }

    #[test]
    fn played_block_ends_with_newline_and_uses_float_math() {
        let record = GameRecord {
            score: 987_654,
            accuracy: 0.9876,
            std: 0.0123,
            std_score: 95.5,
            perfect: 1,
            good: 2,
            bad: 3,
            miss: 4,
            ..GameRecord::default()
        };
        let text = played_record(7, "A", &record);
        assert!(text.ends_with('\n'));
        let accuracy = JavaFloat(0.9876_f32 * 100.0).to_string();
        assert!(text.contains(&format!("准度: {accuracy}%")));
        assert!(text.contains("\n    Perfect: 1, Good: 2, Bad: 3, Miss: 4\n"));
    }
}
