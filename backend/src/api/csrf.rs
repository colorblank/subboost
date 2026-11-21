//! CSRF 防护、多入口来源解析与恒定时间比对工具。

use std::net::IpAddr;

use anyhow::{Result, ensure};
use axum::http::{HeaderMap, header};
use serde_json::Value;

use crate::service::{Service, fault};

/// 判断主机名是否属于本地回环、RFC1918 局域网、Tailscale/CGNAT（100.64.0.0/10）或局域网域名。
pub(super) fn is_private_or_local_host(raw_host: &str) -> bool {
    let host = raw_host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']');
    if host.is_empty() {
        return false;
    }
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".local")
        || lower.ends_with(".lan")
        || lower.ends_with(".home.arpa")
        || lower.ends_with(".internal")
        || lower.ends_with(".ts.net")
    {
        return true;
    }
    let Ok(ip) = host.parse::<IpAddr>() else {
        return false;
    };
    is_private_or_local_ip(ip)
}

fn is_private_or_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            let is_cgnat = octets[0] == 100 && (octets[1] & 0b1100_0000) == 64;
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || is_cgnat
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_private_or_local_ip(IpAddr::V4(mapped));
            }
            let first = v6.segments()[0];
            let is_ula = (first & 0xfe00) == 0xfc00;
            let is_link_local = (first & 0xffc0) == 0xfe80;
            v6.is_loopback() || is_ula || is_link_local
        }
    }
}

/// 判断配置的 `app_url` 是否仍处于默认回环地址（`127.0.0.1` / `localhost` / `::1`）。
pub(super) fn is_loopback_app_url(app_url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(app_url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") || host == "::1"
}

fn is_internal_backend_authority(service: &Service, authority: &str) -> bool {
    let lower = authority.trim().to_ascii_lowercase();
    if lower == "backend" {
        return true;
    }
    let Some((host, port_str)) = lower.rsplit_once(':') else {
        return false;
    };
    let Ok(port) = port_str.parse::<u16>() else {
        return false;
    };
    port == service.config.application.listen.port()
        && matches!(
            host,
            "backend" | "127.0.0.1" | "localhost" | "[::1]" | "0.0.0.0"
        )
}

fn first_header_token<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)?
        .to_str()
        .ok()?
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// 判断当前请求是否通过 HTTPS 访问（兼容反向代理 `X-Forwarded-Proto` 与浏览器 `Origin`/`Referer`）。
pub(super) fn is_https_request(service: &Service, headers: &HeaderMap) -> bool {
    if let Some(proto) = first_header_token(headers, "x-forwarded-proto") {
        if proto.eq_ignore_ascii_case("https") {
            return true;
        }
        if proto.eq_ignore_ascii_case("http") {
            return false;
        }
    }
    for header_name in [header::ORIGIN, header::REFERER] {
        if let Some(raw) = headers.get(header_name).and_then(|v| v.to_str().ok()) {
            if raw.starts_with("https://") {
                return true;
            }
            if raw.starts_with("http://") {
                return false;
            }
        }
    }
    service.config.application.app_url.starts_with("https:")
}

/// 从请求头部（`X-Forwarded-Host`、`Origin`、`Referer`、`Host`）推断客户端实际访问的外部 Origin。
pub(super) fn resolve_request_origin(service: &Service, headers: &HeaderMap) -> Option<String> {
    let default_scheme = if is_https_request(service, headers) {
        "https"
    } else {
        "http"
    };

    if let Some(forwarded_host) = first_header_token(headers, "x-forwarded-host") {
        if !is_internal_backend_authority(service, forwarded_host) {
            if let Ok(parsed) = url::Url::parse(&format!("{default_scheme}://{forwarded_host}")) {
                if parsed.host_str().is_some() {
                    return Some(parsed.origin().ascii_serialization());
                }
            }
        }
    }

    for header_name in [header::ORIGIN, header::REFERER] {
        if let Some(raw) = headers.get(header_name).and_then(|v| v.to_str().ok()) {
            if let Ok(parsed) = url::Url::parse(raw) {
                if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() {
                    let authority = match parsed.port() {
                        Some(port) => format!("{}:{port}", parsed.host_str().unwrap_or_default()),
                        None => parsed.host_str().unwrap_or_default().to_owned(),
                    };
                    if !is_internal_backend_authority(service, &authority) {
                        return Some(parsed.origin().ascii_serialization());
                    }
                }
            }
        }
    }

    if let Some(host) = first_header_token(headers, header::HOST.as_str()) {
        if !is_internal_backend_authority(service, host) {
            if let Ok(parsed) = url::Url::parse(&format!("{default_scheme}://{host}")) {
                if parsed.host_str().is_some() {
                    return Some(parsed.origin().ascii_serialization());
                }
            }
        }
    }

    None
}

/// 当 `app_url` 处于默认回环地址时，自动将返回 JSON 中的订阅链接前缀重写为客户端当前访问的 Origin（适配 NAS 局域网 IP / Tailscale / 反代多入口）。
pub(super) fn rewrite_subscription_urls(
    service: &Service,
    headers: &HeaderMap,
    payload: &mut Value,
) {
    if !is_loopback_app_url(&service.config.application.app_url) {
        return;
    }
    let Some(origin) = resolve_request_origin(service, headers) else {
        return;
    };
    let configured_prefix = service.config.application.app_url.trim_end_matches('/');
    let target_prefix = origin.trim_end_matches('/');
    if configured_prefix == target_prefix {
        return;
    }
    rewrite_value_urls(payload, configured_prefix, target_prefix);
}

fn rewrite_value_urls(value: &mut Value, from_prefix: &str, to_prefix: &str) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                if matches!(key.as_str(), "subscriptionUrl" | "yamlUrl") {
                    if let Some(url_str) = field.as_str() {
                        if let Some(rest) = url_str.strip_prefix(from_prefix) {
                            *field = Value::String(format!("{to_prefix}{rest}"));
                        }
                    }
                } else {
                    rewrite_value_urls(field, from_prefix, to_prefix);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_value_urls(item, from_prefix, to_prefix);
            }
        }
        _ => {}
    }
}

/// 校验请求来源（Sec-Fetch-Site 与 Origin 头部），防止跨站请求伪造攻击，同时支持私有 NAS 多入口（局域网 IP / Tailscale / 反向代理）免配访问。
pub(super) fn check_origin(service: &Service, headers: &HeaderMap) -> Result<()> {
    let sec_fetch_site = headers.get("sec-fetch-site").and_then(|h| h.to_str().ok());
    ensure!(
        sec_fetch_site != Some("cross-site"),
        fault(403, "FORBIDDEN", "Cross-site requests are not allowed")
    );
    if let Some(origin) = headers.get(header::ORIGIN) {
        let origin = origin
            .to_str()
            .map_err(|_| fault(403, "FORBIDDEN", "Invalid Origin"))?;
        let expected = url::Url::parse(&service.config.application.app_url)?
            .origin()
            .ascii_serialization();
        if origin == expected {
            return Ok(());
        }
        let parsed =
            url::Url::parse(origin).map_err(|_| fault(403, "FORBIDDEN", "Invalid Origin"))?;
        let allowed = matches!(parsed.scheme(), "http" | "https")
            && parsed.host_str().is_some_and(|host| {
                let authority = match parsed.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_owned(),
                };
                let matches_request_host =
                    [headers.get("x-forwarded-host"), headers.get(header::HOST)]
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.to_str().ok())
                        .flat_map(|v| v.split(','))
                        .map(str::trim)
                        .any(|candidate| {
                            candidate.eq_ignore_ascii_case(&authority)
                                || candidate.eq_ignore_ascii_case(host)
                        });

                matches_request_host
                    || is_private_or_local_host(host)
                    || sec_fetch_site == Some("same-origin")
            });
        ensure!(
            allowed,
            fault(403, "FORBIDDEN", "Origin does not match APP_URL")
        );
    }
    Ok(())
}

/// 恒定时间字节比对，防止基于时序分析的侧信道攻击。
pub(super) fn secure_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}
