use std::sync::Arc;

use axum::body::Body;
use http::header::{CONTENT_DISPOSITION, HeaderValue};
use http::{Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::auth::require_admin;
use crate::body::body_or_null;
use crate::java;
use crate::reply::{self, APPLICATION_JSON, Failure};
use crate::request::Ctx;
use crate::state::AppState;
use crate::transfer::{self, DataKind, TransferError};

fn require_kind(ctx: &Ctx) -> Result<DataKind, Failure> {
    let raw = ctx.path_param("kind")?;
    DataKind::of(&raw).ok_or_else(|| Failure::bad_request(format!("未知数据类型：{raw}")))
}

fn transfer_failure(prefix: &str, e: TransferError) -> Failure {
    match e {
        TransferError::Invalid(message) => Failure::bad_request(message),
        TransferError::Io(message) => Failure::api(500, format!("{prefix}{message}")),
    }
}

pub(super) async fn export(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let kind = require_kind(ctx)?;
    let document = transfer::export(state, kind)
        .await
        .map_err(|message| Failure::api(500, format!("导出失败：{message}")))?;
    let mut response = reply::text(StatusCode::OK, APPLICATION_JSON, document);
    if let Ok(value) =
        HeaderValue::from_str(&format!("attachment; filename=\"{}.json\"", kind.key()))
    {
        response.headers_mut().insert(CONTENT_DISPOSITION, value);
    }
    tracing::info!("HTTP export {}", kind.key());
    Ok(response)
}

#[derive(Serialize)]
struct ImportJson {
    ok: bool,
    imported: i64,
    total: i64,
    backup: String,
}

pub(super) async fn import(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let kind = require_kind(ctx)?;
    let merge = java::blank_to_null(ctx.query_param("merge"))
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || java::blank_to_null(ctx.query_param("mode"))
            .is_some_and(|v| v.eq_ignore_ascii_case("merge"));
    let document = ctx.body_string().await?;
    if java::is_blank(&document) {
        return Err(Failure::bad_request("内容为空"));
    }
    let result = transfer::import(state, kind, document, merge)
        .await
        .map_err(|e| transfer_failure("导入失败：", e))?;
    reply::json(&ImportJson {
        ok: true,
        imported: result.imported,
        total: result.total,
        backup: result.backup,
    })
}

#[derive(Serialize)]
struct BackupsJson {
    ok: bool,
    backups: Vec<String>,
}

pub(super) async fn backups(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let kind = require_kind(ctx)?;
    let backups = transfer::list_backups(state, kind)
        .await
        .map_err(|_| Failure::api(500, "无法列出备份"))?;
    reply::json(&BackupsJson { ok: true, backups })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RestoreBody {
    backup: Option<String>,
}

#[derive(Serialize)]
struct RestoredJson {
    ok: bool,
    imported: i64,
}

pub(super) async fn restore(
    state: &Arc<AppState>,
    ctx: &mut Ctx,
) -> Result<Response<Body>, Failure> {
    require_admin(state, ctx).await?;
    let kind = require_kind(ctx)?;
    let body: Option<RestoreBody> = body_or_null(ctx).await?;
    let name = body
        .and_then(|b| b.backup)
        .filter(|name| !java::is_blank(name))
        .ok_or_else(|| Failure::bad_request("请指定要恢复的备份"))?;
    let result = transfer::restore(state, kind, &name)
        .await
        .map_err(|e| transfer_failure("恢复失败：", e))?;
    reply::json(&RestoredJson {
        ok: true,
        imported: result.imported,
    })
}
