use std::sync::Arc;

use axum::body::Body;
use http::Response;
use serde::{Deserialize, Serialize};
use zphira_ledger::RemoveOutcome;

use super::{blocking, path_int};
use crate::auth::require_admin;
use crate::body::{body_or_null, primitive};
use crate::reply::{self, Failure};
use crate::request::Ctx;
use crate::state::AppState;

#[derive(Serialize)]
struct AdminsJson {
    ok: bool,
    admins: Vec<i32>,
}

pub(super) async fn list(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let admins = state.admins.clone();
    let admins = blocking(move || admins.list()).await?;
    reply::json(&AdminsJson { ok: true, admins })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AdminBody {
    #[serde(rename = "userId", deserialize_with = "primitive")]
    user_id: i32,
}

#[derive(Serialize)]
struct AddedJson {
    ok: bool,
    added: bool,
    admins: Vec<i32>,
}

pub(super) async fn add(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let body: AdminBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    if body.user_id <= 0 {
        return Err(Failure::bad_format());
    }
    let admins = state.admins.clone();
    let user_id = body.user_id;
    let (added, list) = blocking(move || admins.add(user_id).map(|added| (added, admins.list())))
        .await?
        .map_err(Failure::internal)?;
    tracing::info!("HTTP admin add: {} ({} now)", user_id, list.len());
    reply::json(&AddedJson {
        ok: true,
        added,
        admins: list,
    })
}

#[derive(Serialize)]
struct RemovedJson {
    ok: bool,
    removed: bool,
    admins: Vec<i32>,
}

pub(super) async fn remove(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let user_id = path_int(ctx, "userId")?;
    let admins = state.admins.clone();
    let (outcome, list) = blocking(move || {
        admins
            .remove_keeping_one(user_id)
            .map(|outcome| (outcome, admins.list()))
    })
    .await?
    .map_err(Failure::internal)?;
    if outcome == RemoveOutcome::LastAdmin {
        return Err(Failure::bad_request("至少保留一名管理员"));
    }
    tracing::info!("HTTP admin remove: {} ({} left)", user_id, list.len());
    reply::json(&RemovedJson {
        ok: true,
        removed: outcome == RemoveOutcome::Removed,
        admins: list,
    })
}
