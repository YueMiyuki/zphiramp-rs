use std::sync::Arc;

use crate::reply::Failure;
use crate::request::Ctx;
use crate::state::AppState;

const LOGIN_PATH: &str = "/api/v1/login";

fn need_login() -> Failure {
    Failure::api(401, "需要登录")
}

pub(crate) async fn authenticate(state: &Arc<AppState>, ctx: &mut Ctx) -> Result<(), Failure> {
    if ctx.method == "OPTIONS" || crate::router::match_pattern(LOGIN_PATH, &ctx.path).is_some() {
        return Ok(());
    }
    let token = ctx
        .header("Authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(need_login)?;
    let user_id = state.jwt.parse_user_id(token).ok_or_else(need_login)?;
    ctx.user_id = Some(user_id);
    Ok(())
}

pub(crate) fn require_user(ctx: &Ctx) -> Result<i32, Failure> {
    ctx.user_id.ok_or_else(need_login)
}

pub(crate) async fn require_admin(state: &Arc<AppState>, ctx: &Ctx) -> Result<i32, Failure> {
    let user_id = require_user(ctx)?;
    if is_admin(state, user_id).await {
        Ok(user_id)
    } else {
        Err(Failure::api(403, "需要权限"))
    }
}

pub(crate) async fn is_admin(state: &Arc<AppState>, user_id: i32) -> bool {
    let admins = state.admins.clone();
    tokio::task::spawn_blocking(move || admins.is_admin(user_id))
        .await
        .unwrap_or(false)
}
