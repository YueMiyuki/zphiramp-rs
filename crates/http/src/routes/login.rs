use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use http::Response;
use serde::{Deserialize, Serialize};

use crate::auth;
use crate::body::body_or_null;
use crate::reply::{self, Failure};
use crate::request::Ctx;
use crate::state::AppState;

#[derive(Deserialize, Default)]
#[serde(default)]
struct LoginBody {
    email: Option<String>,
    password: Option<String>,
}

#[derive(Serialize)]
struct LoginOk {
    ok: bool,
    token: String,
    phira_token: String,
    #[serde(rename = "isAdmin")]
    is_admin: bool,
    #[serde(rename = "userId")]
    user_id: i32,
}

pub(crate) const TOO_MANY_ATTEMPTS: &str = "登录尝试过于频繁，请稍后再试";

pub(super) async fn login(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<Response<Body>, Failure> {
    let address = ctx
        .client
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |a| a.ip());
    if !state.login_limiter.try_acquire(address, Instant::now()) {
        return Err(Failure::api(429, TOO_MANY_ATTEMPTS));
    }
    let body: Option<LoginBody> = body_or_null(ctx).await?;
    let Some(LoginBody {
        email: Some(email),
        password: Some(password),
    }) = body
    else {
        return Err(Failure::bad_format());
    };

    let failed = || Failure::bad_request("登录失败，请检查邮箱与密码");
    let result = match state.phira.login(&email, &password).await {
        Ok(result) => result,
        Err(e) if e.status().is_some() || e.is_transport() => return Err(failed()),
        Err(_) => {
            tracing::warn!("HTTP login: upstream answer had no usable token");
            return Err(failed());
        }
    };
    let phira_token = match result.token {
        Some(token) if result.id > 0 && !token.is_empty() => token,
        _ => {
            tracing::warn!("HTTP login: upstream answer had no usable token");
            return Err(failed());
        }
    };
    let is_admin = auth::is_admin(state, result.id).await;
    let token = state.jwt.issue_token(result.id, is_admin);
    tracing::info!("HTTP login: user {} isAdmin={}", result.id, is_admin);
    reply::json(&LoginOk {
        ok: true,
        token,
        phira_token,
        is_admin,
        user_id: result.id,
    })
}
