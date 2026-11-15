use std::sync::Arc;

use axum::body::Body;
use http::Response;
use serde::{Deserialize, Serialize};
use zphira_game::{
    EndRoundError, GameContext, PoolStatusView, RemoveRoomError, RoomPools, RoomUpdate, RoomView,
    SwitchPoolError, UpdateRoomError,
};

use super::pool::require_favorite_body;
use super::{non_blank, or_null};
use crate::auth::{is_admin, require_admin, require_user};
use crate::body::body_or_null;
use crate::reply::{self, Failure};
use crate::request::Ctx;
use crate::state::AppState;

const MAX_CHAT_LENGTH: usize = 200;

fn game(state: &AppState) -> Result<&GameContext, Failure> {
    state
        .game
        .as_ref()
        .ok_or_else(|| Failure::internal_msg("no game context: the room routes are not wired"))
}

fn room_not_found() -> Failure {
    Failure::api(404, "欲请求的内容不存在")
}

fn require_room(game: &GameContext, ctx: &Ctx) -> Result<String, Failure> {
    let room_id = ctx.path_param("id")?;
    if game.room_exists(&room_id) {
        Ok(room_id)
    } else {
        Err(room_not_found())
    }
}

async fn require_in_room(
    state: &Arc<AppState>,
    game: &GameContext,
    user_id: i32,
    room_id: &str,
) -> Result<(), Failure> {
    if is_admin(state, user_id).await || game.is_in_room(user_id, room_id) {
        Ok(())
    } else {
        Err(Failure::api(403, "需要权限"))
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CreateRoomBody {
    r#type: Option<String>,
    pools: Option<Vec<i32>>,
}

pub(super) async fn create(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let game = game(state)?;
    let room_id = ctx.path_param("id")?;
    if !game.is_valid_room_id(&room_id) {
        return Err(Failure::bad_request(
            zphira_game::CreateRoomError::InvalidId.http_message(),
        ));
    }
    if game.room_exists(&room_id) {
        return Err(Failure::bad_request(
            zphira_game::CreateRoomError::AlreadyExists.http_message(),
        ));
    }
    let body: Option<CreateRoomBody> = body_or_null(ctx).await?;
    let (kind, pools) = match body {
        Some(body) => (body.r#type, body.pools),
        None => (None, None),
    };
    let kind = kind.unwrap_or_else(|| "local".to_owned());
    if kind != "local" {
        return Err(Failure::bad_request(format!("不支持的房间类型：{kind}")));
    }
    let pools = match pools {
        Some(ids) if !ids.is_empty() => RoomPools::Resolve(ids),
        _ => RoomPools::Default,
    };
    let ids = game
        .create_room(&room_id, pools)
        .map_err(|e| Failure::bad_request(e.http_message()))?;
    tracing::info!("HTTP create room: {} pools={}", room_id, format!("{ids:?}"));
    reply::ok()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UpdateRoomBody {
    live: Option<bool>,
    lock: Option<bool>,
    #[serde(rename = "chatEnable")]
    chat_enable: Option<bool>,
    #[serde(rename = "minPlayer")]
    min_player: Option<i32>,
    #[serde(rename = "maxPlayer")]
    max_player: Option<i32>,
    #[serde(rename = "selectCountdown")]
    select_countdown: Option<i32>,
    #[serde(rename = "readyCountdown")]
    ready_countdown: Option<i32>,
    #[serde(rename = "forceFinish")]
    force_finish: Option<i32>,
    interval: Option<i32>,
    #[serde(rename = "adminIds")]
    admin_ids: Option<Vec<i32>>,
    #[serde(rename = "cancelReadyInVote")]
    cancel_ready_in_vote: Option<bool>,
    #[serde(rename = "startWhenAllReady")]
    start_when_all_ready: Option<bool>,
    #[serde(rename = "allReadyDelay")]
    all_ready_delay: Option<i32>,
}

pub(super) async fn update(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    let body: UpdateRoomBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    let update = RoomUpdate {
        live: body.live,
        locked: body.lock,
        admin_ids: body.admin_ids,
        chat_enable: body.chat_enable,
        min_player: body.min_player,
        max_player: body.max_player,
        select_countdown: body.select_countdown,
        ready_countdown: body.ready_countdown,
        force_finish: body.force_finish,
        interval: body.interval,
        cancel_ready_in_vote: body.cancel_ready_in_vote,
        start_when_all_ready: body.start_when_all_ready,
        all_ready_delay: body.all_ready_delay,
        reject_min_above_max: true,
    };
    game.update_room(&room_id, &update).map_err(|e| match e {
        UpdateRoomError::RoomNotFound => room_not_found(),
        other => Failure::bad_request(other.http_message()),
    })?;
    tracing::info!("HTTP update room: {} fields updated", room_id);
    reply::ok()
}

#[derive(Serialize)]
struct RoomsJson {
    ok: bool,
    rooms: Vec<RoomView>,
}

pub(super) async fn list(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    let user_id = require_user(ctx)?;
    let game = game(state)?;
    let rooms = if is_admin(state, user_id).await {
        game.rooms()
    } else {
        game.player_room(user_id)
            .and_then(|room_id| game.room(&room_id))
            .into_iter()
            .collect()
    };
    reply::json(&RoomsJson { ok: true, rooms })
}

#[derive(Serialize)]
struct RoomJson {
    ok: bool,
    info: RoomView,
}

pub(super) async fn get(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    let user_id = require_user(ctx)?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    require_in_room(state, game, user_id, &room_id).await?;
    let info = game.room(&room_id).ok_or_else(room_not_found)?;
    reply::json(&RoomJson { ok: true, info })
}

pub(super) async fn delete(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    game.remove_room(&room_id).map_err(|e| match e {
        RemoveRoomError::RoomNotFound => room_not_found(),
        RemoveRoomError::NotEmpty => Failure::bad_request("房间非空，无法删除"),
    })?;
    tracing::info!("HTTP delete room: {}", room_id);
    reply::ok()
}

pub(super) async fn end(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    game.end_round(&room_id).await.map_err(|e| match e {
        EndRoundError::RoomNotFound => room_not_found(),
        EndRoundError::NotPlaying => Failure::bad_request("房间不在游戏中"),
    })?;
    tracing::info!("HTTP force end room: {}", room_id);
    reply::ok()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SayBody {
    message: Option<String>,
}

async fn require_say_message(ctx: &mut Ctx) -> Result<String, Failure> {
    let body: Option<SayBody> = body_or_null(ctx).await?;
    let message = body.and_then(|b| b.message);
    let message =
        non_blank(message.as_deref()).ok_or_else(|| Failure::bad_request("消息不能为空"))?;
    Ok(say_text(message))
}

pub(crate) fn say_text(message: &str) -> String {
    let mut folded = String::with_capacity(message.len());
    let mut units = 0;
    let mut in_break = false;
    for c in message.trim().chars() {
        let c = match c {
            '\r' | '\n' if in_break => continue,
            '\r' | '\n' => {
                in_break = true;
                ' '
            }
            c => {
                in_break = false;
                c
            }
        };
        units += c.len_utf16();
        if units > MAX_CHAT_LENGTH {
            break;
        }
        folded.push(c);
    }
    folded
}

#[derive(Serialize)]
struct DeliveredJson {
    ok: bool,
    delivered: usize,
}

pub(super) async fn say(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    let message = require_say_message(ctx).await?;
    let delivered = game
        .say_to_room(&room_id, &message)
        .ok_or_else(room_not_found)?;
    tracing::info!("HTTP room message to {}: {}", room_id, message);
    reply::json(&DeliveredJson {
        ok: true,
        delivered,
    })
}

pub(super) async fn broadcast(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let game = game(state)?;
    let message = require_say_message(ctx).await?;
    let delivered = game.say_to_all(&message);
    tracing::info!("HTTP global message: {}", message);
    reply::json(&DeliveredJson {
        ok: true,
        delivered,
    })
}

#[derive(Serialize)]
struct PoolJson {
    ok: bool,
    pool: PoolStatusView,
}

pub(super) async fn pool(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    let user_id = require_user(ctx)?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    require_in_room(state, game, user_id, &room_id).await?;
    let pool = game.room_pool(&room_id).ok_or_else(room_not_found)?;
    reply::json(&PoolJson { ok: true, pool })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SwitchPoolBody {
    #[serde(rename = "poolId")]
    pool_id: i32,
}

pub(super) async fn pool_switch(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    let body: SwitchPoolBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    game.switch_room_pool(&room_id, body.pool_id)
        .map_err(|e| match e {
            SwitchPoolError::RoomNotFound => room_not_found(),
            SwitchPoolError::NotInRoom(message) => Failure::bad_request(message),
        })?;
    tracing::info!(
        "HTTP switch room {} pending pool to {}",
        room_id,
        body.pool_id
    );
    reply::ok()
}

pub(super) async fn pool_favorite(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    let favorite_id = require_favorite_body(ctx).await?;
    game.set_room_favorite(&room_id, favorite_id)
        .map_err(|_| room_not_found())?;
    tracing::info!(
        "HTTP room {} pool favorite set to {}",
        room_id,
        or_null(favorite_id)
    );
    reply::ok()
}

#[derive(Serialize)]
struct RemovedJson {
    ok: bool,
    removed: usize,
}

pub(super) async fn clear(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    let user_id = require_user(ctx)?;
    let game = game(state)?;
    let room_id = require_room(game, ctx)?;
    if !(is_admin(state, user_id).await || game.can_control_room(user_id, &room_id)) {
        return Err(Failure::api(403, "需要权限"));
    }
    let removed = game
        .clear_room(&room_id)
        .await
        .map_err(|_| room_not_found())?;
    tracing::info!("HTTP clear room: {} removed={}", room_id, removed);
    reply::json(&RemovedJson { ok: true, removed })
}

#[cfg(test)]
mod tests {
    use super::say_text;

    #[test]
    fn say_text_folds_and_cuts() {
        assert_eq!(say_text("  hi\r\n\nthere  "), "hi there");
        assert_eq!(say_text("a\rb\nc"), "a b c");
        let long = "长".repeat(250);
        assert_eq!(say_text(&long).chars().count(), 200);
        let emoji = format!("{}{}", "a".repeat(199), "😀");
        assert_eq!(say_text(&emoji), "a".repeat(199));
    }
}
