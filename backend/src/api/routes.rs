//! HTTP API 路由分发器与处理器实现。

use std::{collections::HashMap, sync::OnceLock, time::Duration};

use anyhow::{Result, ensure};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Method, Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::service::{Fault, fault, now, required, token};

use super::{
    ApiState,
    auth::{append_cookie, cookie, cookie_value, identity},
    csrf::{check_origin, rewrite_subscription_urls, secure_equal},
    rate_limit::rate,
};

/// 生成与 `Argon2::default()` 参数一致的哑哈希，用于用户名不存在时拉平校验耗时，防止时序枚举。
fn dummy_password_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let salt = SaltString::encode_b64(&[0u8; 16]).expect("static salt");
        Argon2::default()
            .hash_password(b"subboost-dummy-password", &salt)
            .expect("dummy argon2 hash")
            .to_string()
    })
}

/// 构造标准的 JSON HTTP 响应。
pub(super) fn json_response(value: Value, status: u16) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(value),
    )
        .into_response()
}

/// 将内部错误（包括业务 Fault 与未知错误）转换为标准 JSON 错误响应。
pub(super) fn error_response(error: anyhow::Error) -> Response {
    if let Some(f) = error.downcast_ref::<Fault>() {
        return json_response(json!({"error":f.message,"code":f.code}), f.status);
    }
    eprintln!("API error: {error:#}");
    json_response(json!({"error":error.to_string(),"code":"BAD_REQUEST"}), 400)
}

/// 核心路由分发逻辑，处理身份识别、请求校验及各类业务操作。
pub(super) async fn dispatch_inner(
    state: &ApiState,
    request: Request<Body>,
    peer: &str,
) -> Result<Response> {
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
            rate(
                state,
                format!("login_user:{}", username.to_ascii_lowercase()),
                rates.logins,
                Duration::from_secs(rates.login_window_seconds),
            )?;
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
            let (owner, username, hash, account_exists) = match admin {
                Some((owner, username, hash)) => (owner, username, hash, true),
                None => (
                    String::new(),
                    String::new(),
                    dummy_password_hash().to_owned(),
                    false,
                ),
            };
            let valid = tokio::task::spawn_blocking(move || {
                let verified = if hash.starts_with("$2") {
                    bcrypt::verify(password.as_bytes(), &hash).unwrap_or(false)
                } else {
                    PasswordHash::new(&hash).ok().is_some_and(|hash| {
                        Argon2::default()
                            .verify_password(password.as_bytes(), &hash)
                            .is_ok()
                    })
                };
                account_exists && verified
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
        service
            .store
            .run(move |db| {
                db.execute(
                    "INSERT INTO sessions(token,owner,username,guest,expires) VALUES(?1,?2,?3,0,?4)",
                    params![
                        stored,
                        stored_owner,
                        stored_username,
                        Utc::now().timestamp() + session_seconds
                    ],
                )?;
                Ok(())
            })
            .await?;
        let mut response = json_response(
            json!({"success":true,"user":{"id":owner,"username":username}}),
            200,
        );
        append_cookie(
            &mut response,
            &cookie(
                service,
                &headers,
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
            &cookie(
                service,
                &headers,
                &authentication.administrator_cookie,
                "",
                0,
            ),
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
        let mut result = service.execute(&identity.owner, operation, body).await?;
        rewrite_subscription_urls(service, &headers, &mut result);
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
