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
mod rate_limit;
mod routes;

#[derive(Clone)]
pub(crate) struct ApiState {
    pub(crate) service: Service,
    pub(crate) rates: Arc<Mutex<HashMap<String, rate_limit::RateWindow>>>,
    pub(crate) logins: Arc<tokio::sync::Semaphore>,
}

pub fn router(service: Service) -> Router {
    let parallel_logins = service.config.authentication.parallel_logins;
    let state = ApiState {
        service,
        rates: Arc::default(),
        logins: Arc::new(tokio::sync::Semaphore::new(parallel_logins)),
    };
    Router::new()
        .route("/api/{*path}", any(dispatch))
        .with_state(state)
}

pub async fn serve(service: Service, listen: SocketAddr) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    eprintln!(
        "SubBoost Rust API listening on http://{}",
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
