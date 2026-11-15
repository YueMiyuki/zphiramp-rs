use std::sync::Arc;

use axum::body::Body;
use http::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zphira_catalog::PoolCategory;

use super::{or_null, path_int};
use crate::auth::require_admin;
use crate::body::{body_or_null, category};
use crate::reply::{self, Failure};
use crate::request::Ctx;
use crate::state::AppState;
use crate::views::PoolJson;

fn catalog_400(e: zphira_catalog::CatalogError) -> Failure {
    Failure::bad_request(e.message())
}

fn category_name(category: Option<PoolCategory>) -> &'static str {
    category.map_or("null", PoolCategory::name)
}

#[derive(Serialize)]
struct PoolsJson<'a> {
    ok: bool,
    pools: Vec<PoolJson<'a>>,
}

pub(super) async fn list(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pools = state.pools.list_pools();
    reply::json(&PoolsJson {
        ok: true,
        pools: pools.iter().map(|p| PoolJson::from(p.as_ref())).collect(),
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PoolCreateBody {
    id: i32,
    #[serde(rename = "chartIds")]
    chart_ids: Option<Vec<i32>>,
    category: Option<String>,
    #[serde(rename = "sizeLimit")]
    size_limit: Option<i32>,
    #[serde(rename = "roundsPerStay")]
    rounds_per_stay: Option<i32>,
    #[serde(rename = "favoriteId")]
    favorite_id: Option<i32>,
}

pub(super) async fn create(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let body: PoolCreateBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    let category = category(body.category.as_deref())?;
    let chart_ids = body.chart_ids;
    state
        .pools
        .add_pool_with(
            body.id,
            chart_ids.as_deref().unwrap_or(&[]),
            category,
            body.size_limit,
            body.rounds_per_stay,
            body.favorite_id,
        )
        .await
        .map_err(catalog_400)?;
    tracing::info!(
        "HTTP create pool: {} charts={}",
        body.id,
        chart_ids.map_or_else(|| "null".to_owned(), |ids| format!("{ids:?}"))
    );
    reply::ok()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PoolUpdateBody {
    category: Option<String>,
    #[serde(rename = "sizeLimit")]
    size_limit: Option<i32>,
    #[serde(rename = "roundsPerStay")]
    rounds_per_stay: Option<i32>,
    order: Option<i32>,
    #[serde(rename = "submissionOpen")]
    submission_open: Option<bool>,
}

pub(super) async fn update(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "id")?;
    let body: PoolUpdateBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    let category = category(body.category.as_deref())?;
    state
        .pools
        .update_pool(
            pool_id,
            category,
            body.size_limit,
            body.rounds_per_stay,
            body.order,
            body.submission_open,
        )
        .await
        .map_err(catalog_400)?;
    tracing::info!(
        "HTTP update pool {}: category={} sizeLimit={} roundsPerStay={} order={} submissionOpen={}",
        pool_id,
        category_name(category),
        or_null(body.size_limit),
        or_null(body.rounds_per_stay),
        or_null(body.order),
        or_null(body.submission_open)
    );
    reply::ok()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PoolBatchBody {
    count: i32,
    category: Option<String>,
    #[serde(rename = "sizeLimit")]
    size_limit: Option<i32>,
    #[serde(rename = "roundsPerStay")]
    rounds_per_stay: Option<i32>,
}

#[derive(Serialize)]
struct PoolIdsJson {
    ok: bool,
    #[serde(rename = "poolIds")]
    pool_ids: Vec<i32>,
}

pub(super) async fn batch_create(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let body: PoolBatchBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    if body.count < 1 {
        return Err(Failure::bad_format());
    }
    let category = category(body.category.as_deref())?;
    let created = state
        .pools
        .create_empty_pools(body.count, category, body.size_limit, body.rounds_per_stay)
        .await
        .map_err(catalog_400)?;
    reply::json(&PoolIdsJson {
        ok: true,
        pool_ids: created,
    })
}

pub(super) async fn remove(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "id")?;
    state
        .pools
        .remove_pool(pool_id)
        .await
        .map_err(catalog_400)?;
    tracing::info!("HTTP remove pool: {}", pool_id);
    reply::ok()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ChartAddBody {
    #[serde(rename = "chartId")]
    chart_id: i32,
}

pub(super) async fn chart_add(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "id")?;
    let body: ChartAddBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    state
        .pools
        .add_chart(pool_id, body.chart_id)
        .await
        .map_err(catalog_400)?;
    tracing::info!("HTTP add chart {} to pool {}", body.chart_id, pool_id);
    reply::ok()
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct ChartIdsBody {
    #[serde(rename = "chartIds")]
    pub(super) chart_ids: Option<Vec<i32>>,
}

impl ChartIdsBody {
    pub(super) fn required(body: Option<Self>) -> Result<Vec<i32>, Failure> {
        let ids = body.and_then(|b| b.chart_ids).unwrap_or_default();
        if ids.is_empty() {
            return Err(Failure::bad_format());
        }
        Ok(ids)
    }
}

#[derive(Serialize)]
struct AddedJson {
    ok: bool,
    added: i32,
}

pub(super) async fn chart_add_batch(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "id")?;
    let chart_ids = ChartIdsBody::required(body_or_null(ctx).await?)?;
    let added = state
        .pools
        .add_charts(pool_id, &chart_ids)
        .await
        .map_err(catalog_400)?;
    tracing::info!(
        "HTTP add {} charts to pool {} (requested {})",
        added,
        pool_id,
        chart_ids.len()
    );
    reply::json(&AddedJson { ok: true, added })
}

pub(super) async fn chart_remove(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "id")?;
    let chart_id = path_int(ctx, "chartId")?;
    state
        .pools
        .remove_chart(pool_id, chart_id)
        .await
        .map_err(catalog_400)?;
    tracing::info!("HTTP remove chart {} from pool {}", chart_id, pool_id);
    reply::ok()
}

pub(crate) async fn require_favorite_body(ctx: &mut Ctx) -> Result<Option<i32>, Failure> {
    let body: Option<Value> = body_or_null(ctx).await?;
    match body.as_ref().and_then(|b| b.get("favoriteId")) {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .and_then(|id| i32::try_from(id).ok())
            .map(Some)
            .ok_or_else(Failure::bad_format),
        None => Err(Failure::bad_format()),
    }
}

pub(super) async fn favorite(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "id")?;
    let favorite_id = require_favorite_body(ctx).await?;
    state
        .pools
        .set_favorite_id(pool_id, favorite_id)
        .await
        .map_err(catalog_400)?;
    tracing::info!(
        "HTTP pool {} favorite set to {}",
        pool_id,
        or_null(favorite_id)
    );
    reply::ok()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PoolDefaultBody {
    enabled: bool,
}

pub(super) async fn default_flag(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let pool_id = path_int(ctx, "id")?;
    let body: PoolDefaultBody = body_or_null(ctx).await?.ok_or_else(Failure::bad_format)?;
    state
        .pools
        .set_default_flag(pool_id, body.enabled)
        .await
        .map_err(catalog_400)?;
    tracing::info!("HTTP pool {} default set to {}", pool_id, body.enabled);
    reply::ok()
}
