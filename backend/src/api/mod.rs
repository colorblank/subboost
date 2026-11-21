//! HTTP API 服务模块。
//!
//! 基于 Axum 框架提供 RESTful 接口，包含安全响应头注入、身份认证提取、
//! CSRF 防护、全局速率限制以及后台订阅自动更新调度的生命周期管理。

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, State},
    http::{HeaderValue, Request, header},
    response::Response,
    routing::any,
};

use crate::service::Service;

mod auth;
mod csrf;
mod frontend;
mod rate_limit;
mod routes;

/// 传递给 Axum 路由处理器的共享状态。
#[derive(Clone)]
pub(crate) struct ApiState {
    /// 核心服务编排器，提供底层存储与解析访问。
    pub(crate) service: Service,
    /// 基于客户端标识（IP 或会话）的速率限制滑动窗口表。
    pub(crate) rates: Arc<Mutex<HashMap<String, rate_limit::RateWindow>>>,
    /// 限制并发登录校验的信号量，防止高负载消耗过多算力。
    pub(crate) logins: Arc<tokio::sync::Semaphore>,
}

/// 构造包含全量 `/api/{*path}` 分发规则的 Axum 路由器。
pub fn router(service: Service) -> Router {
    let parallel_logins = service.config.authentication.parallel_logins;
    let state = ApiState {
        service,
        rates: Arc::default(),
        logins: Arc::new(tokio::sync::Semaphore::new(parallel_logins)),
    };
    Router::new()
        .route("/api/{*path}", any(dispatch))
        .fallback(frontend::serve_embedded_asset)
        .with_state(state)
}

/// 启动 HTTP API 服务并绑定指定监听地址，同时按配置启动订阅后台更新任务。
pub async fn serve(service: Service, listen: SocketAddr) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let mode = if frontend::has_embedded_frontend() {
        "Web UI + API"
    } else {
        "API"
    };
    eprintln!(
        "SubBoost Rust {mode} listening on http://{}",
        listener.local_addr()?
    );
    let updates = service
        .config
        .refresh
        .enabled
        .then(|| tokio::spawn(crate::refresh::scheduler::run(service.clone())));
    let result = axum::serve(
        listener,
        router(service).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await;
    if let Some(updates) = updates {
        updates.abort();
    }
    result.context("HTTP server stopped")
}

/// 统一 API 请求分发网关，提取对端地址，并在最终响应中附加安全防护头。
async fn dispatch(State(state): State<ApiState>, request: Request<Body>) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| "unknown".into(), |p| p.0.ip().to_string());
    let result = routes::dispatch_inner(&state, request, &peer).await;
    let mut response = match result {
        Ok(response) => response,
        Err(error) => routes::error_response(error),
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    response
}

#[cfg(test)]
mod tests;
