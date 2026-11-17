use crate::service::{Fault, Service, fault, now, required, token};
use anyhow::{Context, Result, ensure};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, State},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct ApiState {
    service: Service,
    rates: Arc<Mutex<HashMap<String, RateWindow>>>,
    logins: Arc<tokio::sync::Semaphore>,
}
#[derive(Clone)]
struct Identity {
    owner: String,
    username: String,
    guest: bool,
    cookie: Option<String>,
}

struct RateWindow {
    count: u32,
    expires_at: Instant,
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
        .then(|| tokio::spawn(crate::scheduler::run(service.clone())));
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
fn rate(state: &ApiState, key: String, limit: u32, window: Duration) -> Result<()> {
    let mut rates = state
        .rates
        .lock()
        .map_err(|_| fault(500, "INTERNAL_ERROR", "Rate limiter unavailable"))?;
    let maximum = state.service.config.rate_limits.maximum_clients;
    if rates.len() >= maximum {
        rates.retain(|_, entry| entry.expires_at > Instant::now());
    }
    ensure!(
        rates.len() < maximum || rates.contains_key(&key),
        fault(429, "RATE_LIMITED", "Too many active clients")
    );
    let entry = rates.entry(key).or_insert_with(|| RateWindow {
        count: 0,
        expires_at: Instant::now() + window,
    });
    if entry.expires_at <= Instant::now() {
        *entry = RateWindow {
            count: 0,
            expires_at: Instant::now() + window,
        };
    }
    entry.count = entry.count.saturating_add(1);
    ensure!(
        entry.count <= limit,
        fault(429, "RATE_LIMITED", "Too many requests. Try again later.")
    );
    Ok(())
}
fn json_response(value: Value, status: u16) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(value),
    )
        .into_response()
}
fn error_response(error: anyhow::Error) -> Response {
    if let Some(f) = error.downcast_ref::<Fault>() {
        return json_response(json!({"error":f.message,"code":f.code}), f.status);
    }
    eprintln!("API error: {error:#}");
    json_response(json!({"error":error.to_string(),"code":"BAD_REQUEST"}), 400)
}
fn append_cookie(response: &mut Response, cookie: &str) -> Result<()> {
    response
        .headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(cookie)?);
    Ok(())
}
fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|item| item.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}
fn cookie(service: &Service, name: &str, value: &str, seconds: i64) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={seconds}{}",
        if service.config.application.app_url.starts_with("https:") {
            "; Secure"
        } else {
            ""
        }
    )
}
async fn identity(service: &Service, headers: &HeaderMap) -> Result<Identity> {
    let authentication = &service.config.authentication;
    let admin = cookie_value(headers, &authentication.administrator_cookie)
        .filter(|v| v.len() == crate::service::TOKEN_LENGTH)
        .map(str::to_owned);
    let guest = cookie_value(headers, &authentication.guest_cookie)
        .filter(|v| v.len() == crate::service::TOKEN_LENGTH)
        .map(str::to_owned);
    for (value, want_guest) in [(admin, false), (guest, true)] {
        if let Some(value) = value {
            let row=service.store.run(move |db| Ok(db.query_row("SELECT owner,username,guest FROM sessions WHERE token=?1 AND expires>?2 AND guest=?3",params![value,Utc::now().timestamp(),want_guest],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?))).optional()?)).await?;
            if let Some((owner, username, guest)) = row {
                return Ok(Identity {
                    owner,
                    username,
                    guest,
                    cookie: None,
                });
            }
        }
    }
    let value = token();
    let owner = format!("guest:{}", token());
    let stored = value.clone();
    let stored_owner = owner.clone();
    let session_seconds = authentication.guest_session_seconds;
    service.store.run(move |db| {
        db.execute("DELETE FROM sessions WHERE expires<=?1",[Utc::now().timestamp()])?;
        db.execute("INSERT INTO sessions(token,owner,username,guest,expires) VALUES(?1,?2,'游客',1,?3)",params![stored,stored_owner,Utc::now().timestamp()+session_seconds])?;Ok(())
    }).await?;
    Ok(Identity {
        owner,
        username: "游客".into(),
        guest: true,
        cookie: Some(cookie(
            service,
            &authentication.guest_cookie,
            &value,
            session_seconds,
        )),
    })
}
fn check_origin(service: &Service, headers: &HeaderMap) -> Result<()> {
    ensure!(
        headers.get("sec-fetch-site").and_then(|h| h.to_str().ok()) != Some("cross-site"),
        fault(403, "FORBIDDEN", "Cross-site requests are not allowed")
    );
    if let Some(origin) = headers.get(header::ORIGIN) {
        let origin = origin
            .to_str()
            .map_err(|_| fault(403, "FORBIDDEN", "Invalid Origin"))?;
        let expected = url::Url::parse(&service.config.application.app_url)?
            .origin()
            .ascii_serialization();
        ensure!(
            origin == expected,
            fault(403, "FORBIDDEN", "Origin does not match APP_URL")
        );
    }
    Ok(())
}
async fn dispatch(State(state): State<ApiState>, request: Request<Body>) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| "unknown".into(), |p| p.0.ip().to_string());
    let result = dispatch_inner(&state, request, &peer).await;
    let mut response = match result {
        Ok(response) => response,
        Err(error) => error_response(error),
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
async fn dispatch_inner(state: &ApiState, request: Request<Body>, peer: &str) -> Result<Response> {
    let service = &state.service;
    let configuration = &service.config;
    let rates = &configuration.rate_limits;
    let authentication = &configuration.authentication;
    let method = request.method().clone();
    let path = request.uri().path().trim_start_matches("/api/").to_owned();
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let headers = request.headers().clone();
    rate(
        state,
        format!("requests:{peer}"),
        rates.requests,
        Duration::from_secs(rates.request_window_seconds),
    )?;
    if !matches!(method, Method::GET | Method::HEAD) {
        check_origin(service, &headers)?;
    }
    if path == "health/live" && method == Method::GET {
        return Ok(json_response(json!({"status":"ok","runtime":"rust"}), 200));
    }
    if path == "health/ready" && method == Method::GET {
        service
            .store
            .run(|db| {
                db.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))?;
                Ok(())
            })
            .await?;
        return Ok(json_response(
            json!({"status":"ok","database":"sqlite"}),
            200,
        ));
    }
    if path == "config" && method == Method::GET {
        let refresh = &configuration.refresh;
        return Ok(json_response(
            json!({"autoUpdateIntervalPolicy":{
            "defaultHours":refresh.default_interval_seconds as f64/3600.0,
            "minHours":refresh.minimum_interval_seconds as f64/3600.0,
            "stepHours":refresh.step_interval_seconds as f64/3600.0,
            "requireIntegerHours":false
        },"minimumPasswordCharacters":authentication.minimum_password_characters}),
            200,
        ));
    }
    let limit = if matches!(
        path.as_str(),
        "auth/login" | "setup/admin" | "source-import" | "settings/source-import"
    ) {
        configuration.limits.small_http_body_bytes
    } else {
        configuration.limits.http_body_bytes
    };
    let mut body = if matches!(method, Method::POST | Method::PUT | Method::PATCH)
        && path != "auth/logout"
        && !path.ends_with("/reset-link")
        && !path.ends_with("/refresh")
    {
        ensure!(
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .is_some_and(|h| h
                    .split(';')
                    .next()
                    .is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json"))),
            fault(
                415,
                "UNSUPPORTED_MEDIA_TYPE",
                "Content-Type must be application/json"
            )
        );
        let bytes = tokio::time::timeout(
            Duration::from_secs(configuration.limits.body_timeout_seconds),
            to_bytes(request.into_body(), limit),
        )
        .await
        .map_err(|_| fault(408, "BAD_REQUEST", "Request body timed out"))?
        .map_err(|_| fault(413, "PAYLOAD_TOO_LARGE", "Request body too large"))?;
        let body: Value = serde_json::from_slice(&bytes)
            .map_err(|_| fault(400, "BAD_REQUEST", "Invalid JSON body"))?;
        ensure!(
            body.is_object(),
            fault(400, "BAD_REQUEST", "Request body must be an object")
        );
        body
    } else {
        json!({})
    };
    if matches!(path.as_str(), "auth/login" | "setup/admin") && method == Method::POST {
        rate(
            state,
            format!("login:{peer}"),
            rates.logins,
            Duration::from_secs(rates.login_window_seconds),
        )?;
        let _permit = state
            .logins
            .try_acquire()
            .map_err(|_| fault(429, "RATE_LIMITED", "Login queue is busy"))?;
        let username = required(&body, "username")?.to_owned();
        let password = required(&body, "password")?.to_owned();
        ensure!(
            username.len() <= configuration.limits.identifier_bytes
                && password.len() <= authentication.maximum_password_bytes,
            fault(400, "BAD_REQUEST", "Credentials too long")
        );
        let (owner, username) = if path == "setup/admin" {
            let configured = std::env::var("LOCAL_SETUP_TOKEN")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| {
                    fault(
                        503,
                        "CONFIGURATION_ERROR",
                        "Set LOCAL_SETUP_TOKEN to enable optional administrator setup",
                    )
                })?;
            let provided = headers
                .get("x-subboost-setup-token")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            ensure!(
                secure_equal(configured.as_bytes(), provided.as_bytes()),
                fault(403, "FORBIDDEN", "Invalid setup token")
            );
            ensure!(
                password.chars().count() >= authentication.minimum_password_characters,
                fault(
                    400,
                    "BAD_REQUEST",
                    format!(
                        "密码至少需要 {} 个字符",
                        authentication.minimum_password_characters
                    )
                )
            );
            ensure!(
                body["passwordConfirm"].as_str().map(str::trim) == Some(password.as_str()),
                fault(400, "BAD_REQUEST", "两次输入的密码不一致")
            );
            let password_hash = tokio::task::spawn_blocking(move || -> Result<String> {
                let salt = SaltString::encode_b64(&rand::random::<[u8; 16]>())
                    .map_err(|e| anyhow::anyhow!("Salt: {e}"))?;
                Ok(Argon2::default()
                    .hash_password(password.as_bytes(), &salt)
                    .map_err(|e| anyhow::anyhow!("Password hash: {e}"))?
                    .to_string())
            })
            .await??;
            let owner = uuid::Uuid::new_v4().to_string();
            let stored_owner = owner.clone();
            let stored_username = username.clone();
            service
                .store
                .run(move |db| {
                    let tx = db.transaction()?;
                    ensure!(
                        tx.query_row("SELECT count(*) FROM admin", [], |r| r.get::<_, i64>(0))?
                            == 0,
                        fault(409, "CONFLICT", "已有管理员账号，请直接登录")
                    );
                    tx.execute(
                        "INSERT INTO admin(singleton,id,username,password_hash) VALUES(1,?1,?2,?3)",
                        params![stored_owner, stored_username, password_hash],
                    )?;
                    tx.commit()?;
                    Ok(())
                })
                .await?;
            (owner, username)
        } else {
            let lookup = username.clone();
            let admin = service
                .store
                .run(move |db| {
                    Ok(db
                        .query_row(
                            "SELECT id,username,password_hash FROM admin WHERE username=?1",
                            [lookup],
                            |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, String>(1)?,
                                    r.get::<_, String>(2)?,
                                ))
                            },
                        )
                        .optional()?)
                })
                .await?;
            let admin =
                admin.ok_or_else(|| fault(401, "UNAUTHORIZED", "Invalid username or password"))?;
            let (owner, username, hash) = admin;
            let valid = tokio::task::spawn_blocking(move || {
                if hash.starts_with("$2") {
                    bcrypt::verify(password.as_bytes(), &hash).unwrap_or(false)
                } else {
                    PasswordHash::new(&hash).ok().is_some_and(|hash| {
                        Argon2::default()
                            .verify_password(password.as_bytes(), &hash)
                            .is_ok()
                    })
                }
            })
            .await?;
            ensure!(
                valid,
                fault(401, "UNAUTHORIZED", "Invalid username or password")
            );
            (owner, username)
        };
        let value = token();
        let stored = value.clone();
        let stored_owner = owner.clone();
        let stored_username = username.clone();
        let session_seconds = authentication.administrator_session_seconds;
        service.store.run(move |db| {db.execute("INSERT INTO sessions(token,owner,username,guest,expires) VALUES(?1,?2,?3,0,?4)",params![stored,stored_owner,stored_username,Utc::now().timestamp()+session_seconds])?;Ok(())}).await?;
        let mut response = json_response(
            json!({"success":true,"user":{"id":owner,"username":username}}),
            200,
        );
        append_cookie(
            &mut response,
            &cookie(
                service,
                &authentication.administrator_cookie,
                &value,
                session_seconds,
            ),
        )?;
        return Ok(response);
    }
    if path == "auth/logout" && method == Method::POST {
        if let Some(value) = cookie_value(&headers, &authentication.administrator_cookie) {
            let value = value.to_owned();
            service
                .store
                .run(move |db| {
                    db.execute("DELETE FROM sessions WHERE token=?1 AND guest=0", [value])?;
                    Ok(())
                })
                .await?;
        }
        let mut response = json_response(json!({"success":true}), 200);
        append_cookie(
            &mut response,
            &cookie(service, &authentication.administrator_cookie, "", 0),
        )?;
        return Ok(response);
    }
    let segments: Vec<_> = path.split('/').collect();
    if segments.len() == 3
        && segments[0] == "subscriptions"
        && segments[2] == "config.yaml"
        && matches!(method, Method::GET | Method::HEAD)
    {
        rate(
            state,
            format!("yaml:{}", segments[1]),
            rates.subscription_downloads,
            Duration::from_secs(rates.download_window_seconds),
        )?;
        let (yaml, row) = service.public_yaml(segments[1]).await?;
        let mut response = if method == Method::HEAD {
            Body::empty().into_response()
        } else {
            yaml.into_response()
        };
        let h = response.headers_mut();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/yaml;charset=utf-8"),
        );
        let name: String = row["name"]
            .as_str()
            .unwrap_or("config")
            .trim()
            .chars()
            .filter(|c| !matches!(c, '\r' | '\n' | '"'))
            .take(80)
            .collect();
        let name = name.trim_end_matches(".yaml").trim_end_matches(".yml");
        let encoded: String = url::form_urlencoded::byte_serialize(name.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        h.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{encoded}"))?,
        );
        let interval = row["autoUpdateInterval"]
            .as_u64()
            .unwrap_or(configuration.refresh.cache_seconds as u64)
            .max(configuration.refresh.minimum_interval_seconds)
            .div_ceil(3600);
        h.insert(
            "profile-update-interval",
            HeaderValue::from_str(&interval.to_string())?,
        );
        let userinfo: [Option<String>; 4] = ["upload", "download", "total", "expire"].map(|key| {
            row["subscriptionInfo"][key]
                .as_f64()
                .map(|v| format!("{key}={v}"))
        });
        let userinfo = userinfo
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; ");
        if !userinfo.is_empty() {
            h.insert("subscription-userinfo", HeaderValue::from_str(&userinfo)?);
        }
        for (field, key) in [
            ("profileWebPageUrl", "profile-web-page-url"),
            ("planName", "plan-name"),
        ] {
            if let Some(value) = row["subscriptionInfo"][field].as_str() {
                if let Ok(value) = HeaderValue::from_str(value) {
                    h.insert(key, value);
                }
            }
        }
        return Ok(response);
    }
    let identity = identity(service, &headers).await?;
    let mut response = if path == "auth/me" && method == Method::GET {
        let setup = service
            .store
            .run(|db| {
                Ok(db.query_row("SELECT count(*) FROM admin", [], |r| r.get::<_, i64>(0))? == 0)
            })
            .await?;
        let subscriptions = service.store.count("subscription", &identity.owner).await?;
        let templates = service.store.count("template", &identity.owner).await?;
        let timestamp = now();
        json_response(
            json!({"setupRequired":false,"adminSetupRequired":setup,"authenticated":true,"isGuest":identity.guest,
            "user":{"id":identity.owner,"username":identity.username,"name":identity.username,"avatarUrl":null,"trustLevel":4,"aiAssistantEnabled":false,"isAdmin":!identity.guest,"isGuest":identity.guest,"isBanned":false,"active":true,"silenced":false,"saveRequirementSatisfied":true,"saveRequirementSatisfiedAt":timestamp,"createdAt":timestamp,"updatedAt":timestamp,"accounts":[],"quota":{"maxSubscriptions":configuration.document_quota(&identity.owner),"maxNodesPerSubscription":configuration.limits.nodes_per_subscription,"maxCustomTemplates":configuration.document_quota(&identity.owner),"maxImportSourcesPerType":configuration.limits.import_sources,"canUseSubscriptionLink":true},"subscriptionCount":subscriptions,"templateCount":templates}}),
            200,
        )
    } else if path == "releases/latest" && method == Method::GET {
        json_response(latest_release(&configuration.network).await, 200)
    } else {
        let query_args: serde_json::Map<String, Value> = query
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        if method == Method::GET || method == Method::DELETE {
            body = Value::Object(query_args);
        }
        let (operation, status) = match (segments.as_slice(), &method) {
            (["subscriptions"], &Method::GET) => ("subscriptions.list", 200),
            (["subscriptions"], &Method::POST) => ("subscriptions.create", 201),
            (["subscriptions"], &Method::PUT) => ("subscriptions.update", 200),
            (["subscriptions"], &Method::DELETE) => ("subscriptions.delete", 200),
            (["subscriptions", id], &Method::GET) => {
                body["id"] = (*id).into();
                ("subscriptions.get", 200)
            }
            (["subscriptions", id], &Method::PUT) => {
                body["id"] = (*id).into();
                ("subscriptions.update", 200)
            }
            (["subscriptions", id], &Method::DELETE) => {
                body["id"] = (*id).into();
                ("subscriptions.delete", 200)
            }
            (["subscriptions", id, "refresh"], &Method::POST) => {
                body["id"] = (*id).into();
                ("subscriptions.refresh", 200)
            }
            (["subscriptions", id, "reset-link"], &Method::POST) => {
                body["id"] = (*id).into();
                let expected = headers
                    .get(header::IF_MATCH)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .trim_matches('"');
                ensure!(
                    expected.len() == crate::service::TOKEN_LENGTH,
                    fault(400, "BAD_REQUEST", "请刷新页面后重试")
                );
                body["expectedToken"] = expected.into();
                ("subscriptions.reset-link", 200)
            }
            (["templates"], &Method::GET) => {
                if let Some(ids) = query.get("ids") {
                    body["ids"] = json!(
                        ids.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                    );
                }
                ("templates.list", 200)
            }
            (["templates"], &Method::POST) => ("templates.create", 201),
            (["templates"], &Method::DELETE) => ("templates.delete", 200),
            (["templates", id], &Method::GET) => {
                body["id"] = (*id).into();
                ("templates.get", 200)
            }
            (["templates", id], &Method::PUT) => {
                body["id"] = (*id).into();
                ("templates.update", 200)
            }
            (["templates", id], &Method::DELETE) => {
                body["id"] = (*id).into();
                ("templates.delete", 200)
            }
            (["source-import"], &Method::POST) => ("source.import", 200),
            (["parse"], &Method::POST) => ("parse", 200),
            (["convert"], &Method::POST) => ("convert", 200),
            (["rules", "search"], &Method::GET) => ("rules.search", 200),
            (["rules", "cn-candidates"], &Method::GET) => ("rules.cn-candidates", 200),
            (["rules", "detect"], &Method::POST) => ("rules.detect", 200),
            (["settings", "source-import"], &Method::GET) => ("settings.get", 200),
            (["settings", "source-import"], &Method::PATCH) => ("settings.update", 200),
            (["backup"], &Method::GET) => ("backup.export", 200),
            (["backup"], &Method::POST) => ("backup.import", 200),
            _ => return Err(fault(404, "NOT_FOUND", "Route not found")),
        };
        if matches!(
            operation,
            "source.import" | "rules.detect" | "convert" | "parse"
        ) {
            rate(
                state,
                format!("work:{}", identity.owner),
                rates.expensive_operations,
                Duration::from_secs(rates.operation_window_seconds),
            )?;
        }
        let result = service.execute(&identity.owner, operation, body).await?;
        let status = if result.get("error").is_some_and(Value::is_string) {
            503
        } else {
            status
        };
        json_response(result, status)
    };
    if let Some(cookie) = identity.cookie {
        append_cookie(&mut response, &cookie)?;
    }
    Ok(response)
}
fn secure_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}
async fn latest_release(settings: &crate::config::Network) -> Value {
    fn version(v: &str) -> Option<Vec<u64>> {
        let parts: Vec<_> = v
            .trim_start_matches('v')
            .split('.')
            .map(str::parse::<u64>)
            .collect::<std::result::Result<_, _>>()
            .ok()?;
        (parts.len() == 3).then_some(parts)
    }
    let current = env!("CARGO_PKG_VERSION");
    let fallback = json!({"currentVersion":current,"latestVersion":null,"latestTag":null,"releaseUrl":null,"hasUpdate":false});
    let result = crate::network::fetch(
        &settings.latest_release_url,
        &settings.user_agents[0],
        crate::network::RequestPurpose::Content,
        crate::network::SourcePolicy::PublicOnly,
        settings,
    )
    .await;
    let Ok(response) = result else {
        return fallback;
    };
    let Ok(data) = serde_json::from_str::<Value>(&response.content) else {
        return fallback;
    };
    let Some(tag) = data["tag_name"].as_str() else {
        return fallback;
    };
    let Some(latest) = version(tag) else {
        return fallback;
    };
    json!({"currentVersion":current,"latestVersion":tag.trim_start_matches('v'),"latestTag":tag,"releaseUrl":data["html_url"],"hasUpdate":version(current).is_some_and(|v|latest>v)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
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
}
