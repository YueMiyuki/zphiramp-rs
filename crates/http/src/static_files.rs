use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use http::header::{CACHE_CONTROL, HeaderValue};
use http::{Response, StatusCode};

use crate::reply::{TEXT_HTML_UTF8, TEXT_PLAIN, text};

fn mime_for(name: &str) -> &'static str {
    let ext = name.rfind('.').map_or("", |dot| &name[dot + 1..]);
    match ext {
        "html" => TEXT_HTML_UTF8,
        "js" | "mjs" => "application/javascript",
        "css" => "text/css",
        "json" | "map" => "application/json",
        "txt" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

pub(crate) fn absolute_normalized(dir: &Path) -> PathBuf {
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(dir))
            .unwrap_or_else(|_| dir.to_path_buf())
    };
    normalize(&absolute)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut names = 0usize;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if names > 0 {
                    out.pop();
                    names -= 1;
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            Component::Normal(name) => {
                out.push(name);
                names += 1;
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn resolve(base: &Path, relative: &str) -> PathBuf {
    let joined = if relative.starts_with('/') {
        PathBuf::from(relative)
    } else {
        base.join(relative)
    };
    normalize(&joined)
}

fn not_found() -> Response<Body> {
    text(StatusCode::NOT_FOUND, TEXT_PLAIN, "Not Found")
}

pub(crate) async fn serve(frontend_dir: &Path, path: &str) -> Response<Body> {
    if path.starts_with("/api/") {
        return not_found();
    }
    let raw = path.strip_prefix('/').unwrap_or(path);
    let Some(file) = guarded(frontend_dir, raw) else {
        return bad_request();
    };
    if is_regular_file(&file).await {
        return serve_file(frontend_dir, &file, false).await;
    }

    let mut relative = raw.to_owned();
    // The web manager links some assets percent-encoded (`[id]` as `%5Bid%5D`).
    if let Some(decoded) = percent_decode_utf8(raw).filter(|d| d != raw) {
        if decoded.contains('\\') {
            return bad_request();
        }
        let Some(file) = guarded(frontend_dir, &decoded) else {
            return bad_request();
        };
        if is_regular_file(&file).await {
            return serve_file(frontend_dir, &file, false).await;
        }
        relative = decoded;
    }

    // A missing asset (anything with an extension) is a 404, never the HTML page.
    let last_segment = relative.rsplit('/').next().unwrap_or(&relative);
    if last_segment.contains('.') {
        return not_found();
    }

    let html = if relative.starts_with("room") {
        frontend_dir.join("room/__fallback__/index.html")
    } else if relative.starts_with("pool") {
        frontend_dir.join("pool/__fallback__/index.html")
    } else {
        frontend_dir.join("index.html")
    };
    serve_file(frontend_dir, &html, true).await
}

fn bad_request() -> Response<Body> {
    text(StatusCode::BAD_REQUEST, TEXT_PLAIN, "Bad Request")
}

fn guarded(frontend_dir: &Path, relative: &str) -> Option<PathBuf> {
    if relative.contains('\0') {
        return None;
    }
    let file = resolve(frontend_dir, relative);
    file.starts_with(frontend_dir).then_some(file)
}

async fn is_regular_file(file: &Path) -> bool {
    tokio::fs::metadata(file)
        .await
        .is_ok_and(|meta| meta.is_file())
}

fn percent_decode_utf8(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = |b: Option<&u8>| b.and_then(|b| char::from(*b).to_digit(16));
            let (hi, lo) = (hex(bytes.get(i + 1))?, hex(bytes.get(i + 2))?);
            out.push(u8::try_from(hi * 16 + lo).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

async fn serve_file(frontend_dir: &Path, file: &Path, html: bool) -> Response<Body> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let content_type = mime_for(&name);
    let html = html || content_type == TEXT_HTML_UTF8;
    let (status, body) = match tokio::fs::read(file).await {
        Ok(bytes) => (StatusCode::OK, bytes),
        Err(_) => (StatusCode::NOT_FOUND, b"Not Found".to_vec()),
    };
    if html {
        return text(status, TEXT_HTML_UTF8, body);
    }
    let mut response = text(status, content_type, body);
    if status == StatusCode::OK {
        let hashed = file
            .strip_prefix(frontend_dir)
            .is_ok_and(|relative| relative.starts_with("_next/static"));
        let cache = if hashed {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=300"
        };
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_like_java() {
        let base = Path::new("/srv/frontend/dist");
        assert_eq!(
            resolve(base, "a/b.js"),
            Path::new("/srv/frontend/dist/a/b.js")
        );
        assert_eq!(
            resolve(base, "a/../b.js"),
            Path::new("/srv/frontend/dist/b.js")
        );
        assert_eq!(
            resolve(base, "../secret"),
            Path::new("/srv/frontend/secret")
        );
        assert_eq!(resolve(base, "/etc/passwd"), Path::new("/etc/passwd"));
        assert_eq!(resolve(base, "a//b/"), Path::new("/srv/frontend/dist/a/b"));
        assert_eq!(
            resolve(base, "..%2fsecret"),
            Path::new("/srv/frontend/dist/..%2fsecret")
        );
        assert!(!resolve(base, "../dist2/x").starts_with(base));
        assert_eq!(resolve(Path::new("/"), "../../x"), Path::new("/x"));
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode_utf8("a%20b").as_deref(), Some("a b"));
        assert_eq!(
            percent_decode_utf8("%5Bid%5D/x+y").as_deref(),
            Some("[id]/x+y")
        );
        assert_eq!(percent_decode_utf8("%E4%B8%AD").as_deref(), Some("中"));
        assert_eq!(percent_decode_utf8("%zz"), None);
        assert_eq!(percent_decode_utf8("a%2"), None);
        assert_eq!(percent_decode_utf8("%C3%28"), None);
        assert_eq!(percent_decode_utf8("%252e").as_deref(), Some("%2e"));
    }

    #[test]
    fn mime_table() {
        assert_eq!(mime_for("a.html"), TEXT_HTML_UTF8);
        assert_eq!(mime_for("a.PNG"), "application/octet-stream");
        assert_eq!(mime_for("LICENSE"), "application/octet-stream");
        assert_eq!(mime_for("x.webmanifest"), "application/octet-stream");
        assert_eq!(mime_for("x.tar.gz"), "application/octet-stream");
        assert_eq!(mime_for("x.min.js"), "application/javascript");
    }
}
