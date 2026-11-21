//! 内嵌静态前端资源托管模块。
//!
//! 构建期由 `backend/build.rs` 扫描 `local/out`（或 `SUBBOOST_EMBED_FRONTEND_DIR`）
//! 生成静态资源表，使单一 Rust 二进制可直接提供完整 Web UI 与 `/api/*` 接口。

use axum::{
    body::Body,
    http::{HeaderValue, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
};

include!(concat!(env!("OUT_DIR"), "/embedded_frontend.rs"));

/// 判断当前二进制是否已内嵌静态前端产物。
#[must_use]
pub fn has_embedded_frontend() -> bool {
    !EMBEDDED_ASSETS.is_empty()
}

fn find_asset<'a>(
    assets: &'a [(&'static str, &'static str, &'static [u8])],
    key: &str,
) -> Option<(&'static str, &'static str, &'static [u8])> {
    assets
        .binary_search_by_key(&key, |entry| entry.0)
        .ok()
        .map(|idx| assets[idx])
}

fn resolve_asset<'a>(
    path: &str,
    assets: &'a [(&'static str, &'static str, &'static [u8])],
) -> Option<(StatusCode, &'static str, &'static str, &'static [u8])> {
    if assets.is_empty() {
        return None;
    }

    let normalized = if path.is_empty() || path == "/" {
        "/"
    } else {
        path.trim_end_matches('/')
    };

    if normalized == "/" {
        if let Some((key, mime, bytes)) = find_asset(assets, "/index.html") {
            return Some((StatusCode::OK, key, mime, bytes));
        }
    } else {
        if let Some((key, mime, bytes)) = find_asset(assets, normalized) {
            return Some((StatusCode::OK, key, mime, bytes));
        }
        let html_candidate = format!("{normalized}.html");
        if let Some((key, mime, bytes)) = find_asset(assets, &html_candidate) {
            return Some((StatusCode::OK, key, mime, bytes));
        }
        let index_candidate = format!("{normalized}/index.html");
        if let Some((key, mime, bytes)) = find_asset(assets, &index_candidate) {
            return Some((StatusCode::OK, key, mime, bytes));
        }
    }

    if let Some((key, mime, bytes)) = find_asset(assets, "/404.html") {
        return Some((StatusCode::NOT_FOUND, key, mime, bytes));
    }

    None
}

fn cache_control_for(asset_path: &str, mime: &str) -> &'static str {
    if asset_path.starts_with("/_next/static/") {
        "public, max-age=31536000, immutable"
    } else if mime.starts_with("text/html")
        || mime.starts_with("application/manifest+json")
        || mime.starts_with("text/plain")
    {
        "no-store, no-cache, max-age=0, must-revalidate"
    } else {
        "public, max-age=3600"
    }
}

fn apply_security_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
}

fn build_asset_response(
    method: &Method,
    path: &str,
    assets: &[(&'static str, &'static str, &'static [u8])],
) -> Response {
    if !matches!(*method, Method::GET | Method::HEAD) {
        let mut response = (StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed").into_response();
        apply_security_headers(&mut response);
        return response;
    }

    if path == "/favicon.ico" {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::PERMANENT_REDIRECT;
        let headers = response.headers_mut();
        headers.insert(header::LOCATION, HeaderValue::from_static("/icon.png"));
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=0, must-revalidate"),
        );
        apply_security_headers(&mut response);
        return response;
    }

    let Some((status, asset_path, mime, bytes)) = resolve_asset(path, assets) else {
        let mut response = (
            StatusCode::NOT_FOUND,
            if assets.is_empty() {
                "Frontend static assets are not embedded in this build."
            } else {
                "Not Found"
            },
        )
            .into_response();
        apply_security_headers(&mut response);
        return response;
    };

    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control_for(asset_path, mime)),
    );
    apply_security_headers(&mut response);
    response
}

/// Axum fallback 处理器：托管内嵌的 Next.js 静态前端资源。
pub(super) async fn serve_embedded_asset(request: Request<Body>) -> Response {
    build_asset_response(request.method(), request.uri().path(), EMBEDDED_ASSETS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ASSETS: &[(&str, &str, &[u8])] = &[
        ("/404.html", "text/html; charset=utf-8", b"<h1>404</h1>"),
        (
            "/_next/static/chunks/app.js",
            "application/javascript; charset=utf-8",
            b"console.log(1);",
        ),
        (
            "/dashboard.html",
            "text/html; charset=utf-8",
            b"<h1>Dashboard</h1>",
        ),
        (
            "/dashboard/settings.html",
            "text/html; charset=utf-8",
            b"<h1>Settings</h1>",
        ),
        ("/index.html", "text/html; charset=utf-8", b"<h1>Home</h1>"),
        ("/logo.png", "image/png", b"PNG"),
    ];

    #[test]
    fn resolves_root_clean_routes_and_exact_assets() {
        let root = build_asset_response(&Method::GET, "/", TEST_ASSETS);
        assert_eq!(root.status(), StatusCode::OK);
        assert_eq!(
            root.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            root.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store, no-cache, max-age=0, must-revalidate"
        );
        assert_eq!(root.headers().get("x-frame-options").unwrap(), "DENY");

        let dashboard = build_asset_response(&Method::GET, "/dashboard", TEST_ASSETS);
        assert_eq!(dashboard.status(), StatusCode::OK);

        let settings = build_asset_response(&Method::GET, "/dashboard/settings/", TEST_ASSETS);
        assert_eq!(settings.status(), StatusCode::OK);

        let chunk = build_asset_response(&Method::GET, "/_next/static/chunks/app.js", TEST_ASSETS);
        assert_eq!(chunk.status(), StatusCode::OK);
        assert_eq!(
            chunk.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
    }

    #[test]
    fn redirects_favicon_and_handles_missing_or_disallowed_requests() {
        let favicon = build_asset_response(&Method::GET, "/favicon.ico", TEST_ASSETS);
        assert_eq!(favicon.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            favicon.headers().get(header::LOCATION).unwrap(),
            "/icon.png"
        );

        let missing = build_asset_response(&Method::GET, "/does-not-exist", TEST_ASSETS);
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let post = build_asset_response(&Method::POST, "/", TEST_ASSETS);
        assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);

        let head = build_asset_response(&Method::HEAD, "/logo.png", TEST_ASSETS);
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(
            head.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
    }
}
