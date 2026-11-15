use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    HeaderValue,
};
use http::{Request, Response, StatusCode};

use crate::auth;
use crate::reply::{self, Failure, JavalinError};
use crate::request::Ctx;
use crate::routes::{self, Route};
use crate::state::AppState;

pub(crate) struct Endpoint {
    pub(crate) method: &'static str,
    pub(crate) pattern: &'static str,
    pub(crate) route: Route,
}

pub(crate) async fn dispatch(state: Arc<AppState>, request: Request<Body>) -> Response<Body> {
    let client = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    let mut ctx = Ctx::new(request, client);

    if !ctx.path.starts_with('/') {
        return reply::jetty_not_found();
    }
    if !jetty_accepts(&ctx.path) {
        return reply::jetty_bad_request();
    }

    let api = is_api_path(&ctx.path);
    let mut response = if api {
        match auth::authenticate(&state, &mut ctx).await {
            Ok(()) => run_endpoint(&state, &mut ctx).await,
            Err(failure) => render_failure(&ctx, failure),
        }
    } else {
        run_endpoint(&state, &mut ctx).await
    };
    if ctx.method == "HEAD" {
        response = without_body(response).await;
    }

    if api {
        let headers = response.headers_mut();
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
        headers.insert(
            ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
        );
        headers.insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Content-Type, Authorization"),
        );
    }
    response
}

pub(crate) fn is_api_path(path: &str) -> bool {
    path.starts_with("/api/v1/")
}

async fn without_body(response: Response<Body>) -> Response<Body> {
    let (mut parts, body) = response.into_parts();
    let length = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_or(0, |bytes| bytes.len());
    parts
        .headers
        .insert(http::header::CONTENT_LENGTH, HeaderValue::from(length));
    Response::from_parts(parts, Body::empty())
}

async fn run_endpoint(state: &Arc<AppState>, ctx: &mut Ctx) -> Response<Body> {
    let method = ctx.method.clone();
    let route_method = if method == "HEAD" {
        "GET"
    } else {
        method.as_str()
    };
    match find_endpoint(route_method, &ctx.path) {
        Some((endpoint, params)) => {
            ctx.raw_params = params;
            ctx.body_idle_timeout = state.config.idle_timeout;
            if endpoint.route == Route::DataImport {
                ctx.max_body = state.config.max_import_size;
            } else {
                ctx.max_body = state.config.max_request_size;
            }
            match routes::handle(endpoint.route, state, ctx).await {
                Ok(response) => response,
                Err(failure) => render_failure(ctx, failure),
            }
        }
        None => JavalinError::endpoint_not_found(route_method, &ctx.path).render(&ctx.headers),
    }
}

fn render_failure(ctx: &Ctx, failure: Failure) -> Response<Body> {
    match failure {
        Failure::Api { status, reason } => reply::api_error(status, &reason),
        Failure::Internal(error) => {
            tracing::error!(
                error = &*error as &(dyn std::error::Error + 'static),
                "HTTP API error on {}",
                ctx.path
            );
            reply::api_error(StatusCode::INTERNAL_SERVER_ERROR, reply::INTERNAL_ERROR)
        }
        Failure::Javalin(error) => error.render(&ctx.headers),
        Failure::Aborted(reason) => {
            tracing::debug!("HTTP request on {} aborted: {}", ctx.path, reason);
            let mut response = reply::empty(StatusCode::INTERNAL_SERVER_ERROR);
            response.headers_mut().remove(http::header::CONTENT_TYPE);
            response
        }
    }
}

fn find_endpoint(
    method: &str,
    path: &str,
) -> Option<(&'static Endpoint, Vec<(&'static str, String)>)> {
    routes::ENDPOINTS
        .iter()
        .filter(|e| e.method == method)
        .find_map(|e| match_pattern(e.pattern, path).map(|params| (e, params)))
}

pub(crate) fn match_pattern(pattern: &str, path: &str) -> Option<Vec<(&'static str, String)>> {
    if pattern == "/" {
        return (path == "/").then(Vec::new);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return path.starts_with(prefix).then(Vec::new);
    }
    let trimmed = if path.len() > 1 {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    let rest = trimmed.strip_prefix('/')?;
    let mut segments = rest.split('/');
    let mut params = Vec::new();
    for expected in pattern.split('/').filter(|s| !s.is_empty()) {
        let actual = segments.next()?;
        if let Some(name) = expected.strip_prefix('{').and_then(|n| n.strip_suffix('}')) {
            if actual.is_empty() {
                return None;
            }
            params.push((static_name(name), actual.to_owned()));
        } else if expected != actual {
            return None;
        }
    }
    if segments.next().is_some() {
        return None;
    }
    Some(params)
}

fn static_name(name: &str) -> &'static str {
    match name {
        "id" => "id",
        "chartId" => "chartId",
        "poolId" => "poolId",
        "kind" => "kind",
        "userId" => "userId",
        "playerId" => "playerId",
        _ => "param",
    }
}

fn jetty_accepts(path: &str) -> bool {
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let valid = bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
            if !valid {
                return false;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut depth: i64 = 0;
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    let last = segments.len().saturating_sub(1);
    for (index, segment) in segments.iter().enumerate() {
        let decoded = segment.to_ascii_lowercase().replace("%2e", ".");
        match decoded.as_str() {
            "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            "" if index == last => {}
            _ => depth += 1,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(
            match_pattern("/api/v1/pool/list", "/api/v1/pool/list"),
            Some(vec![])
        );
        assert_eq!(
            match_pattern("/api/v1/pool/list", "/api/v1/pool/list/"),
            Some(vec![])
        );
        assert_eq!(
            match_pattern("/api/v1/pool/list", "/api/v1/pool/list//"),
            None
        );
        assert_eq!(
            match_pattern("/api/v1/pool/list", "/api/v1//pool/list"),
            None
        );
        assert_eq!(
            match_pattern("/api/v1/pool/{id}", "/api/v1/pool/list"),
            Some(vec![("id", "list".to_owned())])
        );
        assert_eq!(
            match_pattern("/api/v1/pool/{id}/chart/{chartId}", "/api/v1/pool//chart/5"),
            None
        );
        assert_eq!(match_pattern("/", "/"), Some(vec![]));
        assert_eq!(
            match_pattern("/api/v1/room/{id}/", "/api/v1/room/r1"),
            Some(vec![("id", "r1".to_owned())])
        );
        assert_eq!(match_pattern("/", "//"), None);
        assert_eq!(match_pattern("/*", "/anything/at/all"), Some(vec![]));
    }

    #[test]
    fn jetty_rules() {
        assert!(jetty_accepts("/docs/../style.css"));
        assert!(jetty_accepts("/api/v1/record/player/../list"));
        assert!(!jetty_accepts("/../secret.txt"));
        assert!(!jetty_accepts("/room/../../secret.txt"));
        assert!(!jetty_accepts("/%2e%2e/secret.txt"));
        assert!(jetty_accepts("/..%2fsecret.txt"));
        assert!(!jetty_accepts("/api/v1/record/%zz"));
        assert!(!jetty_accepts("/api/v1/record/abc%2"));
        assert!(jetty_accepts("/api/v1/record/a%2Fb"));
        assert!(jetty_accepts("/"));
        assert!(jetty_accepts("//etc/passwd"));
    }
}
