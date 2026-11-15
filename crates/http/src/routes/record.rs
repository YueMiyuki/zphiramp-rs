use std::sync::Arc;

use axum::body::Body;
use http::Response;
use serde::Serialize;
use zphira_ledger::{PlayerRoundView, RankEntry, RecordView};

use super::{non_blank, path_int, query_double, query_number, query_page_size};
use crate::auth::require_admin;
use crate::reply::{self, Failure};
use crate::request::Ctx;
use crate::state::AppState;

#[derive(Serialize)]
struct ListResult<'a> {
    total: usize,
    records: Vec<RecordView<'a>>,
}

#[derive(Serialize)]
struct ListJson<'a> {
    ok: bool,
    result: ListResult<'a>,
}

pub(super) async fn list(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let limit = query_page_size(ctx)?;
    let offset = (query_double(ctx, "offset", 0.0)? as i32).max(0);
    let total = state.records.len();
    let room_id = non_blank(ctx.query_param("roomId")).map(str::to_owned);
    let player_id = query_number(ctx, "playerId")?.map(|id| id as i32);
    let records = state.records.query(
        limit,
        usize::try_from(offset).unwrap_or(0),
        room_id.as_deref(),
        player_id,
    );
    reply::json(&ListJson {
        ok: true,
        result: ListResult {
            total,
            records: records.iter().map(|r| r.api_view()).collect(),
        },
    })
}

#[derive(Serialize)]
struct RecordJson<'a> {
    ok: bool,
    record: RecordView<'a>,
}

pub(super) async fn get(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let id = ctx.path_param("id")?;
    let record = state
        .records
        .find(&id)
        .ok_or_else(|| Failure::api(404, "记录不存在"))?;
    reply::json(&RecordJson {
        ok: true,
        record: record.api_view(),
    })
}

#[derive(Serialize)]
struct PlayerResult<'a> {
    #[serde(rename = "playerId")]
    player_id: i32,
    rounds: Vec<PlayerRoundView<'a>>,
}

#[derive(Serialize)]
struct PlayerJson<'a> {
    ok: bool,
    result: PlayerResult<'a>,
}

pub(super) async fn player(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let player_id = path_int(ctx, "playerId")?;
    let limit = query_page_size(ctx)?;
    let rounds = state.records.by_player(player_id, limit);
    reply::json(&PlayerJson {
        ok: true,
        result: PlayerResult {
            player_id,
            rounds: rounds.iter().map(|r| r.api_view()).collect(),
        },
    })
}

#[derive(Serialize)]
struct RankingJson {
    ok: bool,
    ranking: Vec<RankEntry>,
}

pub(super) async fn ranking(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let limit = query_page_size(ctx)?;
    reply::json(&RankingJson {
        ok: true,
        ranking: state.points.ranking_snapshot(limit),
    })
}
