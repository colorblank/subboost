use anyhow::{Context, Result};
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::router;
use crate::service::Service;

async fn call(
    router: Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    body: Value,
) -> Result<(StatusCode, HeaderMap, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let response = router
        .oneshot(builder.body(Body::from(body.to_string()))?)
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((status, headers, serde_json::from_slice(&bytes)?))
}

#[tokio::test]
async fn guests_save_without_login_and_cannot_cross_owner_boundary() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let service = Service::open({
        let mut configuration = crate::config::Config::default();
        configuration.application.database = (&dir.path().join("test.db")).to_path_buf();
        configuration.application.app_url = ("http://127.0.0.1:3001").to_owned();
        configuration
    })
    .await?;
    let app = router(service);
    let (status, headers, me) =
        call(app.clone(), Method::GET, "/api/auth/me", None, json!({})).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["authenticated"], true);
    assert_eq!(me["isGuest"], true);
    let cookie = headers[header::SET_COOKIE]
        .to_str()?
        .split(';')
        .next()
        .context("cookie")?;
    let (_, other_headers, _) =
        call(app.clone(), Method::GET, "/api/auth/me", None, json!({})).await?;
    let other = other_headers[header::SET_COOKIE]
        .to_str()?
        .split(';')
        .next()
        .context("cookie")?;
    let input = json!({"name":"private","nodes":[{"name":"node","type":"ss","server":"example.com","port":443,"cipher":"aes-128-gcm","password":"secret"}]});
    let (status, _, created) = call(
        app.clone(),
        Method::POST,
        "/api/subscriptions",
        Some(cookie),
        input,
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["subscription"]["id"].as_str().context("id")?;
    let (status, _, _) = call(
        app.clone(),
        Method::GET,
        &format!("/api/subscriptions/{id}"),
        Some(other),
        json!({}),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, _, list) = call(
        app.clone(),
        Method::GET,
        "/api/subscriptions",
        Some(cookie),
        json!({}),
    )
    .await?;
    assert_eq!(list["subscriptions"][0]["id"], id);
    let (_, _, list) = call(
        app.clone(),
        Method::GET,
        "/api/subscriptions",
        Some(other),
        json!({}),
    )
    .await?;
    assert_eq!(list["subscriptions"], json!([]));
    let (status, _, _) = call(
        app,
        Method::PATCH,
        "/api/settings/source-import",
        Some(other),
        json!({"allowUnsafeSubscriptionSources":true}),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn csrf_rejects_cross_origin_mutations() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let service = Service::open({
        let mut configuration = crate::config::Config::default();
        configuration.application.database = (&dir.path().join("test.db")).to_path_buf();
        configuration.application.app_url = ("http://127.0.0.1:3001").to_owned();
        configuration
    })
    .await?;
    let response = router(service)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("Origin", "https://attacker.example")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}
