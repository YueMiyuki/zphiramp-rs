mod admin;
mod chart;
mod data;
mod login;
mod pool;
mod record;
mod room;
mod submission;

use std::sync::Arc;

use std::fmt::Display;

use axum::body::Body;
use http::{Response, StatusCode};

use crate::reply::{self, BAD_PARAM, Failure};
use crate::request::Ctx;
use crate::router::Endpoint;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    Options,
    Login,
    RoomCreate,
    RoomUpdate,
    RoomList,
    RoomGet,
    RoomDelete,
    RoomEnd,
    RoomSay,
    Broadcast,
    RoomPool,
    RoomPoolSwitch,
    RoomPoolFavorite,
    RoomClear,
    PoolList,
    PoolCreate,
    PoolBatchCreate,
    PoolRemove,
    PoolUpdate,
    PoolChartAdd,
    PoolChartAddBatch,
    PoolChartRemove,
    PoolFavorite,
    PoolDefault,
    SubmitCharts,
    PoolSubmissions,
    SubmissionWithdraw,
    OpenPools,
    MySubmissions,
    PendingSubmissions,
    SubmissionApprove,
    SubmissionReject,
    ChartSearch,
    PoolGenerate,
    ChartDuration,
    DataExport,
    DataImport,
    DataBackups,
    DataRestore,
    AdminList,
    AdminAdd,
    AdminRemove,
    RecordList,
    PlayerRecords,
    RecordGet,
    PointRanking,
    Frontend,
}

const fn ep(method: &'static str, pattern: &'static str, route: Route) -> Endpoint {
    Endpoint {
        method,
        pattern,
        route,
    }
}

// First match wins: specific paths must stay before parameter paths.
pub(crate) static ENDPOINTS: &[Endpoint] = &[
    ep("OPTIONS", "/api/v1/*", Route::Options),
    ep("POST", "/api/v1/login", Route::Login),
    ep("POST", "/api/v1/room/{id}/create", Route::RoomCreate),
    ep("PUT", "/api/v1/room/{id}/update", Route::RoomUpdate),
    ep("GET", "/api/v1/room/list", Route::RoomList),
    ep("GET", "/api/v1/room/{id}", Route::RoomGet),
    ep("DELETE", "/api/v1/room/{id}", Route::RoomDelete),
    ep("POST", "/api/v1/room/{id}/end", Route::RoomEnd),
    ep("POST", "/api/v1/room/{id}/say", Route::RoomSay),
    ep("POST", "/api/v1/broadcast", Route::Broadcast),
    ep("GET", "/api/v1/room/{id}/pool", Route::RoomPool),
    ep(
        "PUT",
        "/api/v1/room/{id}/pool/switch",
        Route::RoomPoolSwitch,
    ),
    ep(
        "PUT",
        "/api/v1/room/{id}/pool/favorite",
        Route::RoomPoolFavorite,
    ),
    ep("POST", "/api/v1/room/{id}/clear", Route::RoomClear),
    ep("GET", "/api/v1/pool/list", Route::PoolList),
    ep("POST", "/api/v1/pool", Route::PoolCreate),
    ep("POST", "/api/v1/pool/batch", Route::PoolBatchCreate),
    ep("DELETE", "/api/v1/pool/{id}", Route::PoolRemove),
    ep("PUT", "/api/v1/pool/{id}", Route::PoolUpdate),
    ep("POST", "/api/v1/pool/{id}/chart", Route::PoolChartAdd),
    ep("POST", "/api/v1/pool/{id}/charts", Route::PoolChartAddBatch),
    ep(
        "DELETE",
        "/api/v1/pool/{id}/chart/{chartId}",
        Route::PoolChartRemove,
    ),
    ep("PUT", "/api/v1/pool/{id}/favorite", Route::PoolFavorite),
    ep("PUT", "/api/v1/pool/{id}/default", Route::PoolDefault),
    ep("POST", "/api/v1/pool/{id}/submissions", Route::SubmitCharts),
    ep(
        "GET",
        "/api/v1/pool/{id}/submissions",
        Route::PoolSubmissions,
    ),
    ep(
        "DELETE",
        "/api/v1/pool/{id}/submissions/{chartId}",
        Route::SubmissionWithdraw,
    ),
    ep("GET", "/api/v1/submission/open-pools", Route::OpenPools),
    ep("GET", "/api/v1/submission/mine", Route::MySubmissions),
    ep(
        "GET",
        "/api/v1/submission/pending",
        Route::PendingSubmissions,
    ),
    ep(
        "POST",
        "/api/v1/submission/{poolId}/{chartId}/approve",
        Route::SubmissionApprove,
    ),
    ep(
        "POST",
        "/api/v1/submission/{poolId}/{chartId}/reject",
        Route::SubmissionReject,
    ),
    ep("GET", "/api/v1/chart/search", Route::ChartSearch),
    ep("POST", "/api/v1/pool/generate", Route::PoolGenerate),
    ep("POST", "/api/v1/chart/duration", Route::ChartDuration),
    ep("GET", "/api/v1/data/{kind}/export", Route::DataExport),
    ep("POST", "/api/v1/data/{kind}/import", Route::DataImport),
    ep("GET", "/api/v1/data/{kind}/backups", Route::DataBackups),
    ep("POST", "/api/v1/data/{kind}/restore", Route::DataRestore),
    ep("GET", "/api/v1/admin", Route::AdminList),
    ep("POST", "/api/v1/admin", Route::AdminAdd),
    ep("DELETE", "/api/v1/admin/{userId}", Route::AdminRemove),
    ep("GET", "/api/v1/record/list", Route::RecordList),
    ep(
        "GET",
        "/api/v1/record/player/{playerId}",
        Route::PlayerRecords,
    ),
    ep("GET", "/api/v1/record/{id}", Route::RecordGet),
    ep("GET", "/api/v1/point/ranking", Route::PointRanking),
    ep("GET", "/", Route::Frontend),
    ep("GET", "/*", Route::Frontend),
];

pub(crate) async fn handle(
    route: Route,
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    match route {
        Route::Options => Ok(reply::plain(StatusCode::NO_CONTENT, "")),
        Route::Login => login::login(state, ctx).await,
        Route::RoomCreate => room::create(state, ctx).await,
        Route::RoomUpdate => room::update(state, ctx).await,
        Route::RoomList => room::list(state, ctx).await,
        Route::RoomGet => room::get(state, ctx).await,
        Route::RoomDelete => room::delete(state, ctx).await,
        Route::RoomEnd => room::end(state, ctx).await,
        Route::RoomSay => room::say(state, ctx).await,
        Route::Broadcast => room::broadcast(state, ctx).await,
        Route::RoomPool => room::pool(state, ctx).await,
        Route::RoomPoolSwitch => room::pool_switch(state, ctx).await,
        Route::RoomPoolFavorite => room::pool_favorite(state, ctx).await,
        Route::RoomClear => room::clear(state, ctx).await,
        Route::PoolList => pool::list(state, ctx).await,
        Route::PoolCreate => pool::create(state, ctx).await,
        Route::PoolBatchCreate => pool::batch_create(state, ctx).await,
        Route::PoolRemove => pool::remove(state, ctx).await,
        Route::PoolUpdate => pool::update(state, ctx).await,
        Route::PoolChartAdd => pool::chart_add(state, ctx).await,
        Route::PoolChartAddBatch => pool::chart_add_batch(state, ctx).await,
        Route::PoolChartRemove => pool::chart_remove(state, ctx).await,
        Route::PoolFavorite => pool::favorite(state, ctx).await,
        Route::PoolDefault => pool::default_flag(state, ctx).await,
        Route::SubmitCharts => submission::submit(state, ctx).await,
        Route::PoolSubmissions => submission::pool_list(state, ctx).await,
        Route::SubmissionWithdraw => submission::withdraw(state, ctx).await,
        Route::OpenPools => submission::open_pools(state, ctx).await,
        Route::MySubmissions => submission::mine(state, ctx).await,
        Route::PendingSubmissions => submission::pending(state, ctx).await,
        Route::SubmissionApprove => submission::approve(state, ctx).await,
        Route::SubmissionReject => submission::reject(state, ctx).await,
        Route::ChartSearch => chart::search(state, ctx).await,
        Route::PoolGenerate => chart::generate(state, ctx).await,
        Route::ChartDuration => chart::duration(state, ctx).await,
        Route::DataExport => data::export(state, ctx).await,
        Route::DataImport => data::import(state, ctx).await,
        Route::DataBackups => data::backups(state, ctx).await,
        Route::DataRestore => data::restore(state, ctx).await,
        Route::AdminList => admin::list(state, ctx).await,
        Route::AdminAdd => admin::add(state, ctx).await,
        Route::AdminRemove => admin::remove(state, ctx).await,
        Route::RecordList => record::list(state, ctx).await,
        Route::PlayerRecords => record::player(state, ctx).await,
        Route::RecordGet => record::get(state, ctx).await,
        Route::PointRanking => record::ranking(state, ctx).await,
        Route::Frontend => Ok(crate::static_files::serve(&state.frontend_dir, &ctx.path).await),
    }
}

const MAX_PAGE_SIZE: i32 = 200;

pub(crate) fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

pub(crate) fn or_null<T: Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_owned(), |v| v.to_string())
}

pub(crate) fn path_int(ctx: &Ctx, name: &str) -> Result<i32, Failure> {
    ctx.path_param(name)?
        .parse()
        .map_err(|_| Failure::bad_request(BAD_PARAM))
}

pub(crate) fn query_number(ctx: &Ctx, name: &str) -> Result<Option<f64>, Failure> {
    let Some(raw) = non_blank(ctx.query_param(name)) else {
        return Ok(None);
    };
    raw.trim()
        .parse::<f64>()
        .map(Some)
        .map_err(|_| Failure::bad_request(format!("参数格式错误：{name}")))
}

pub(crate) fn query_double(ctx: &Ctx, name: &str, fallback: f64) -> Result<f64, Failure> {
    Ok(query_number(ctx, name)?.unwrap_or(fallback))
}

pub(crate) fn query_limit(ctx: &Ctx, max: i32) -> Result<usize, Failure> {
    let limit = (query_double(ctx, "limit", 50.0)? as i32).clamp(1, max);
    Ok(usize::try_from(limit).unwrap_or(1))
}

pub(crate) fn query_page_size(ctx: &Ctx) -> Result<usize, Failure> {
    query_limit(ctx, MAX_PAGE_SIZE)
}

pub(crate) async fn blocking<R: Send + 'static>(
    work: impl FnOnce() -> R + Send + 'static,
) -> Result<R, Failure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(Failure::internal)
}
