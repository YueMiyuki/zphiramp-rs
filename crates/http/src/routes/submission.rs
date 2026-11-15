use std::sync::Arc;

use axum::body::Body;
use http::Response;
use serde::{Deserialize, Serialize};
use zphira_catalog::{ApproveOutcome, CatalogError, SubmissionStatus, SubmissionView};

use super::path_int;
use super::pool::ChartIdsBody;
use crate::auth::{require_admin, require_user};
use crate::body::body_or_null;
use crate::reply::{self, Failure};
use crate::request::Ctx;
use crate::state::AppState;
use crate::views::{PoolJson, SubmissionJson};

const MAX_SUBMIT_BATCH: usize = 20;
pub(crate) const REASON_UNKNOWN_CHART: &str = "Phira 上找不到这张谱面";
pub(crate) const REASON_LOOKUP_FAILED: &str = "暂时无法从 Phira 获取这张谱面，请稍后再试";

#[derive(Serialize)]
struct PoolsJson<'a> {
    ok: bool,
    pools: Vec<PoolJson<'a>>,
}

pub(super) async fn open_pools(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_user(ctx)?;
    let pools = state.pools.list_open_for_submission();
    reply::json(&PoolsJson {
        ok: true,
        pools: pools.iter().map(|p| PoolJson::from(p.as_ref())).collect(),
    })
}

#[derive(Serialize)]
struct ResultJson {
    #[serde(rename = "chartId")]
    chart_id: i32,
    accepted: bool,
    reason: Option<String>,
}

#[derive(Serialize)]
struct SubmitJson {
    ok: bool,
    accepted: usize,
    results: Vec<ResultJson>,
}

pub(super) async fn submit(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    let user_id = require_user(ctx)?;
    let pool_id = path_int(ctx, "id")?;
    let chart_ids = ChartIdsBody::required(body_or_null(ctx).await?)?;
    if chart_ids.len() > MAX_SUBMIT_BATCH {
        return Err(Failure::bad_request(format!(
            "一次最多投稿 {MAX_SUBMIT_BATCH} 张谱面"
        )));
    }
    let pool = state
        .pools
        .find_pool(pool_id)
        .ok_or_else(|| Failure::api(404, "谱池不存在"))?;
    if !pool.submission_open {
        return Err(Failure::bad_request("该谱池未开启投稿"));
    }

    let lookups = state.index.ensure_indexed(&chart_ids).await;
    let known: Vec<i32> = lookups
        .iter()
        .filter(|l| l.result.is_ok())
        .map(|l| l.chart_id)
        .collect();
    let name = if known.is_empty() {
        String::new()
    } else {
        resolve_player_name(state, user_id).await
    };
    let mut submitted = state
        .submissions
        .submit(pool_id, &known, user_id, Some(&name))
        .await
        .map_err(Failure::internal)?
        .into_iter();

    let results: Vec<ResultJson> = lookups
        .into_iter()
        .map(|lookup| match lookup.result {
            Ok(_) => match submitted.next() {
                Some(r) => ResultJson {
                    chart_id: r.chart_id,
                    accepted: r.accepted,
                    reason: r.reason,
                },
                None => ResultJson {
                    chart_id: lookup.chart_id,
                    accepted: false,
                    reason: Some(REASON_LOOKUP_FAILED.to_owned()),
                },
            },
            Err(e) => ResultJson {
                chart_id: lookup.chart_id,
                accepted: false,
                reason: Some(lookup_reason(&e).to_owned()),
            },
        })
        .collect();
    let accepted = results.iter().filter(|r| r.accepted).count();
    tracing::info!(
        "HTTP submission: user {} backed {} chart(s) in pool {} (accepted {})",
        user_id,
        chart_ids.len(),
        pool_id,
        accepted
    );
    reply::json(&SubmitJson {
        ok: true,
        accepted,
        results,
    })
}

fn lookup_reason(error: &CatalogError) -> &'static str {
    match error {
        CatalogError::Upstream(e) if e.is_not_found() => REASON_UNKNOWN_CHART,
        _ => REASON_LOOKUP_FAILED,
    }
}

async fn resolve_player_name(state: &Arc<AppState>, user_id: i32) -> String {
    if let Some(name) = state.players.player_name(user_id) {
        return name;
    }
    let fallback = || format!("用户 {user_id}");
    match tokio::time::timeout(
        state.config.name_lookup_timeout,
        state.phira.user_by_id(user_id),
    )
    .await
    {
        Ok(Ok(user)) => user.name.clone().unwrap_or_else(fallback),
        _ => fallback(),
    }
}

#[derive(Serialize)]
struct SubmissionsJson {
    ok: bool,
    submissions: Vec<SubmissionJson>,
}

fn submissions_json(
    state: &AppState,
    views: Vec<SubmissionView>,
) -> Result<Response<Body>, Failure> {
    let submissions = views
        .into_iter()
        .map(|v| SubmissionJson::new(v, &state.index))
        .collect();
    reply::json(&SubmissionsJson {
        ok: true,
        submissions,
    })
}

pub(super) async fn pool_list(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "id")?;
    let filter = match ctx.query_param("status") {
        None => None,
        Some(raw) if raw.trim().is_empty() => None,
        Some(raw) => Some(
            SubmissionStatus::parse_ignore_case(raw)
                .ok_or_else(|| Failure::bad_request(format!("未知状态：{raw}")))?,
        ),
    };
    submissions_json(state, state.submissions.list_by_pool(pool_id, filter))
}

pub(super) async fn mine(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    let user_id = require_user(ctx)?;
    submissions_json(state, state.submissions.list_by_submitter(user_id))
}

pub(super) async fn pending(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    submissions_json(state, state.submissions.list_pending())
}

pub(crate) const APPROVE_FAILED: &str = "加入谱池失败：";

pub(super) async fn approve(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    let reviewer_id = require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "poolId")?;
    let chart_id = path_int(ctx, "chartId")?;
    match state
        .submissions
        .approve(&state.pools, pool_id, chart_id, reviewer_id)
        .await
        .map_err(Failure::internal)?
    {
        ApproveOutcome::NotFound => Err(Failure::api(404, "投稿不存在")),
        ApproveOutcome::AddFailed(e) => Err(Failure::bad_request(format!(
            "{APPROVE_FAILED}{}",
            e.message()
        ))),
        ApproveOutcome::Approved => {
            tracing::info!(
                "HTTP submission approved: pool {} chart {} by {}",
                pool_id,
                chart_id,
                reviewer_id
            );
            reply::ok()
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RejectBody {
    reason: Option<String>,
}

pub(super) async fn reject(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    let reviewer_id = require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "poolId")?;
    let chart_id = path_int(ctx, "chartId")?;
    let body: Option<RejectBody> = body_or_null(ctx).await?;
    let reason = body.and_then(|b| b.reason);
    let found = state
        .submissions
        .review(pool_id, chart_id, false, reviewer_id, reason.as_deref())
        .await
        .map_err(Failure::internal)?;
    if !found {
        return Err(Failure::api(404, "投稿不存在"));
    }
    tracing::info!(
        "HTTP submission rejected: pool {} chart {} by {}",
        pool_id,
        chart_id,
        reviewer_id
    );
    reply::ok()
}

pub(super) async fn withdraw(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    let user_id = require_user(ctx)?;
    let pool_id = path_int(ctx, "id")?;
    let chart_id = path_int(ctx, "chartId")?;
    let withdrawn = state
        .submissions
        .withdraw(pool_id, chart_id, user_id)
        .await
        .map_err(Failure::internal)?;
    if !withdrawn {
        return Err(Failure::bad_request(
            "无法撤回：没有投过这张谱面，或它已经通过审核",
        ));
    }
    reply::ok()
}
