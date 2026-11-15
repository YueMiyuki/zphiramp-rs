use std::collections::HashSet;
use std::sync::Arc;

use axum::body::Body;
use http::Response;
use serde::{Deserialize, Serialize};
use zphira_catalog::{CatalogError, MAX_PROBE_BUDGET, PoolCategory, ScreeningRule};
use zphira_common::gson::JsonElement;

use super::pool::ChartIdsBody;
use super::{query_double, query_number};
use crate::auth::require_admin;
use crate::body::{body_or_null, category};
use crate::java;
use crate::reply::{self, Failure};
use crate::request::Ctx;
use crate::state::AppState;
use crate::views::ChartJson;

const MAX_SEARCH_LIMIT: f64 = 500.0;
const MAX_PROBE_BATCH: usize = 20;
const DEFAULT_POOL_SIZE: i32 = 15;

#[derive(Serialize)]
struct SearchResult<'a> {
    indexed: usize,
    #[serde(rename = "remoteTotal")]
    remote_total: i32,
    refreshing: bool,
    matched: usize,
    #[serde(rename = "pendingDuration")]
    pending_duration: bool,
    charts: Vec<ChartJson<'a>>,
}

#[derive(Serialize)]
struct SearchJson<'a> {
    ok: bool,
    result: SearchResult<'a>,
}

pub(super) async fn search(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let rule = rule_from_query(ctx)?;
    let limit = java::double_to_int(java::min_f64(
        java::max_f64(1.0, query_double(ctx, "limit", 50.0)?),
        MAX_SEARCH_LIMIT,
    ));
    let probe_budget = query_number(ctx, "probeDuration")?;
    // A refresh starts only once the whole query above is valid.
    match ctx.query_param("refresh") {
        Some("full") => {
            state.index.refresh_full_async();
        }
        Some(_) => {
            let division = java::blank_to_null(ctx.query_param("division")).map(str::to_owned);
            state.index.refresh_async(division);
        }
        None => {}
    }
    if let Some(budget) = probe_budget
        && budget > 0.0
    {
        probe_durations_for(state, &rule, java::double_to_int(budget)).await;
    }

    let charts = state
        .index
        .search(&rule, usize::try_from(limit).unwrap_or(0));
    let pending_duration = rule.min_duration_seconds.is_some()
        && state.index.count_matches(&rule.without_duration()) > 0;
    reply::json(&SearchJson {
        ok: true,
        result: SearchResult {
            indexed: state.index.indexed_count(),
            remote_total: state.index.remote_count(),
            refreshing: state.index.is_refreshing(),
            matched: state.index.count_matches(&rule),
            pending_duration,
            charts: charts.iter().map(|c| ChartJson::from(c.as_ref())).collect(),
        },
    })
}

fn query_category(ctx: &mut Ctx) -> Result<Option<PoolCategory>, Failure> {
    let Some(raw) = java::blank_to_null(ctx.query_param("category")) else {
        return Ok(None);
    };
    PoolCategory::parse_ignore_case(raw)
        .map(Some)
        .ok_or_else(|| Failure::bad_request(format!("未知类别：{raw}")))
}

fn rule_from_query(ctx: &mut Ctx) -> Result<ScreeningRule, Failure> {
    let category = query_category(ctx)?;
    let preset = category.and_then(ScreeningRule::of);
    let tag = ctx.query_param("tag").map(str::to_owned);
    let min_difficulty = query_number(ctx, "minDifficulty")?;
    let min_duration = query_number(ctx, "minDuration")?;

    let difficulty_bound = min_difficulty.or(preset.as_ref().and_then(|p| p.min_difficulty));
    let duration_bound = match min_duration {
        Some(d) => Some(java::double_to_int(d)),
        None => preset.as_ref().and_then(|p| p.min_duration_seconds),
    };
    let tags: Vec<String> = match tag.as_deref() {
        Some(tag) if !java::is_blank(tag) => {
            let parts = java::split_comma(tag);
            let mut seen = HashSet::new();
            if !parts.iter().all(|p| seen.insert(*p)) {
                return Err(Failure::bad_request("参数格式错误：tag"));
            }
            parts.into_iter().map(str::to_owned).collect()
        }
        _ => preset
            .as_ref()
            .map(|p| p.any_tags.iter().cloned().collect())
            .unwrap_or_default(),
    };
    let min_rating = query_double(
        ctx,
        "minRating",
        preset.as_ref().map_or(0.0, |p| f64::from(p.min_rating)),
    )? as f32;
    let min_rating_count = java::double_to_int(query_double(
        ctx,
        "minRatingCount",
        preset
            .as_ref()
            .map_or(0.0, |p| f64::from(p.min_rating_count)),
    )?);
    Ok(ScreeningRule::new(
        category.unwrap_or(PoolCategory::Manual),
        tags,
        min_rating,
        min_rating_count,
        difficulty_bound,
        duration_bound,
    ))
}

async fn probe_durations_for(state: &Arc<AppState>, rule: &ScreeningRule, budget: i32) {
    if rule.min_duration_seconds.is_none() {
        return;
    }
    let relaxed = rule.without_duration();
    let capped = usize::try_from(budget.max(0))
        .unwrap_or(0)
        .min(MAX_PROBE_BUDGET);
    let candidates = state.index.search(&relaxed, capped * 4);
    let probed = state.index.probe_durations(&candidates, capped).await;
    tracing::info!(
        "Probed {} durations for {} (budget {})",
        probed,
        rule.category,
        capped
    );
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PoolGenerateBody {
    category: Option<JsonElement>,
    #[serde(rename = "sizeLimit")]
    size_limit: Option<i32>,
    #[serde(rename = "roundsPerStay")]
    rounds_per_stay: Option<i32>,
    #[serde(rename = "probeDuration")]
    probe_duration: Option<i32>,
}

#[derive(Serialize)]
struct GenerateJson {
    ok: bool,
    #[serde(rename = "poolIds")]
    pool_ids: Vec<i32>,
    leftover: usize,
}

pub(super) async fn generate(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let body: PoolGenerateBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    let category = category(body.category.as_ref())?.ok_or_else(Failure::bad_format)?;
    let rule =
        ScreeningRule::of(category).ok_or_else(|| Failure::bad_request("该类别不支持规则生成"))?;
    if let Some(budget) = body.probe_duration
        && budget > 0
    {
        probe_durations_for(state, &rule, budget).await;
    }
    let outcome = state
        .pools
        .generate_pools(
            &state.index,
            &rule,
            body.size_limit.unwrap_or(DEFAULT_POOL_SIZE),
            body.rounds_per_stay,
        )
        .await
        .map_err(|e| match e {
            CatalogError::IllegalArgument(_) | CatalogError::Io(_) | CatalogError::Upstream(_) => {
                Failure::bad_request(e.message())
            }
            other => Failure::internal(other),
        })?;
    reply::json(&GenerateJson {
        ok: true,
        pool_ids: outcome.pool_ids,
        leftover: outcome.leftover,
    })
}

#[derive(Serialize)]
struct DurationJson<'a> {
    ok: bool,
    probed: usize,
    charts: Vec<ChartJson<'a>>,
}

pub(super) async fn duration(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let chart_ids = ChartIdsBody::required(body_or_null(ctx).await?)?;
    if chart_ids.len() > MAX_PROBE_BATCH {
        return Err(Failure::bad_request(format!(
            "一次最多探测 {MAX_PROBE_BATCH} 张谱面"
        )));
    }
    let charts: Vec<_> = chart_ids
        .iter()
        .filter_map(|&id| state.index.get(id))
        .collect();
    let probed = state.index.probe_durations(&charts, charts.len()).await;
    reply::json(&DurationJson {
        ok: true,
        probed,
        charts: charts.iter().map(|c| ChartJson::from(c.as_ref())).collect(),
    })
}
