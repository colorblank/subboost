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
async fn custom_templates_match_builtin_response_contract() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut configuration = crate::config::Config::default();
    configuration.application.database = dir.path().join("test.db");
    let config = crate::templates::default_config("minimal", &configuration.generation);
    let service = Service::open(configuration).await?;
    service
        .store
        .insert(
            "template",
            "local",
            &json!({
                "id": "existing-template", "name": "Existing", "description": null,
                "config": config, "usageCount": 2, "favoriteCount": 1
            }),
            100,
        )
        .await?;
    let existing = service
        .execute("local", "templates.get", json!({"id":"existing-template"}))
        .await?;
    assert_eq!(existing["template"]["kind"], "config");
    assert_eq!(existing["template"]["downloads"], 2);
    assert_eq!(existing["template"]["engagementCount"], 1);
    assert_eq!(existing["template"]["description"], "");
    assert_eq!(existing["template"]["tags"], json!([]));
    let app = router(service);
    let (_, headers, _) = call(app.clone(), Method::GET, "/api/auth/me", None, json!({})).await?;
    let cookie = headers[header::SET_COOKIE]
        .to_str()?
        .split(';')
        .next()
        .context("cookie")?;
    let (status, _, created) = call(
        app.clone(),
        Method::POST,
        "/api/templates",
        Some(cookie),
        json!({"name":"Saved", "config":config}),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["template"]["id"].as_str().context("id")?;
    for path in [
        "/api/templates?type=my".to_owned(),
        format!("/api/templates/{id}"),
    ] {
        let (status, _, result) =
            call(app.clone(), Method::GET, &path, Some(cookie), json!({})).await?;
        assert_eq!(status, StatusCode::OK);
        let template = result.get("template").unwrap_or(&result["templates"][0]);
        assert_eq!(template["kind"], "config");
        assert_eq!(template["description"], "");
        assert_eq!(template["downloads"], 0);
        assert_eq!(template["engagementCount"], 0);
        assert_eq!(template["tags"], json!([]));
    }
    Ok(())
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
    let response = router(service.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header(header::HOST, "192.168.1.50:3001")
                .header("Origin", "https://attacker.example")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let lan_cross_origin = router(service)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header(header::HOST, "192.168.1.50:3001")
                .header("Origin", "http://192.168.1.99:8080")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(lan_cross_origin.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn nas_multi_entry_allows_lan_tailscale_and_rewrites_subscription_urls() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let service = Service::open({
        let mut configuration = crate::config::Config::default();
        configuration.application.database = (&dir.path().join("test.db")).to_path_buf();
        configuration.application.app_url = ("http://127.0.0.1:3001").to_owned();
        configuration
    })
    .await?;
    let app = router(service);

    let input = json!({
        "name": "nas-sub",
        "nodes": [{
            "name": "node",
            "type": "ss",
            "server": "example.com",
            "port": 443,
            "cipher": "aes-128-gcm",
            "password": "secret"
        }]
    });
    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/subscriptions")
                .header(header::HOST, "192.168.1.50:3001")
                .header("Origin", "http://192.168.1.50:3001")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(input.to_string()))?,
        )
        .await?;
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let cookie = create_response.headers()[header::SET_COOKIE]
        .to_str()?
        .split(';')
        .next()
        .context("cookie")?
        .to_owned();
    let created: Value =
        serde_json::from_slice(&create_response.into_body().collect().await?.to_bytes())?;
    assert!(
        created["subscription"]["subscriptionUrl"]
            .as_str()
            .unwrap_or_default()
            .starts_with("http://192.168.1.50:3001/api/subscriptions/")
    );

    let list_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/subscriptions")
                .header(header::COOKIE, &cookie)
                .header("X-Forwarded-Proto", "https")
                .header("X-Forwarded-Host", "sub.nas.example.com")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(list_response.status(), StatusCode::OK);
    let listed: Value =
        serde_json::from_slice(&list_response.into_body().collect().await?.to_bytes())?;
    assert!(
        listed["subscriptions"][0]["subscriptionUrl"]
            .as_str()
            .unwrap_or_default()
            .starts_with("https://sub.nas.example.com/api/subscriptions/")
    );
    Ok(())
}
