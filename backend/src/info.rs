//! 订阅用量信息与到期时间解析工具。
//!
//! 负责从 HTTP `subscription-userinfo` 响应头或机场占位提示节点名称中，
//! 正则提取已用上传/下载流量、总配额、到期时间以及官网/套餐信息。

use chrono::{DateTime, NaiveDate};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;

fn expire(value: f64) -> Option<i64> {
    let value = value.floor();
    if value > 946684800.0 && value <= 253402300799.0 {
        Some(value as i64)
    } else if value > 946684800000.0 && value <= 253402300799000.0 {
        Some((value / 1000.0).floor() as i64)
    } else {
        None
    }
}
fn date(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|v| v.timestamp())
        .or_else(|| {
            NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
                .ok()?
                .and_hms_opt(12, 0, 0)
                .map(|v| v.and_utc().timestamp())
        })
        .filter(|v| expire(*v as f64).is_some())
}
fn labeled<'a>(name: &'a str, labels: &[&str]) -> Option<&'a str> {
    labels.iter().find_map(|label| {
        let index = name.find(label)?;
        let suffix = name[index + label.len()..].trim_start();
        suffix
            .strip_prefix(':')
            .or_else(|| suffix.strip_prefix('：'))
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })
}
fn bytes(value: &str) -> Option<f64> {
    static BYTES: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*(B|KB|MB|GB|TB|PB)\b").expect("static bytes expression")
    });
    let capture = BYTES.captures(value)?;
    let value = capture[1].parse::<f64>().ok()?;
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let exponent = units
        .iter()
        .position(|s| s.eq_ignore_ascii_case(&capture[2]))?;
    let value = (value * 1024_f64.powi(exponent as i32)).round();
    value.is_finite().then_some(value)
}
/// 从 HTTP 头或节点列表中提取结构化的订阅用量信息（upload, download, total, expire 等）。
pub fn subscription_info(headers: &Value, nodes: &[Value]) -> Value {
    let mut info = json!({});
    if let Some(header) = headers["subscription-userinfo"].as_str() {
        for field in header.split(';') {
            if let Some((key, value)) = field.split_once('=') {
                let key = key.trim();
                if !matches!(key, "upload" | "download" | "total" | "expire") {
                    continue;
                }
                let decoded: String = url::form_urlencoded::parse(
                    format!("value={}", value.trim().replace('+', "%2B")).as_bytes(),
                )
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
                let numeric = decoded
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0);
                if key == "expire" {
                    if let Some(value) = numeric.and_then(expire).or_else(|| date(&decoded)) {
                        info[key] = value.into();
                    }
                } else if let Some(value) = numeric {
                    info[key] = json!(value.floor());
                }
            }
        }
    }
    let mut remaining = None;
    let mut used = None;
    let mut total = None;
    let mut expiry = None;
    for node in nodes {
        let name = node["name"].as_str().unwrap_or_default().trim();
        if remaining.is_none() {
            remaining = labeled(name, &["剩余流量", "剩余订阅流量"]).and_then(bytes);
        }
        if used.is_none() {
            used =
                labeled(name, &["已用流量", "已使用流量", "累计已用", "累计使用"]).and_then(bytes);
        }
        if total.is_none() {
            total = labeled(name, &["总流量", "套餐流量", "流量上限", "总量"]).and_then(bytes);
        }
        if expiry.is_none() {
            expiry =
                labeled(name, &["套餐到期", "订阅到期", "到期时间", "过期时间"]).and_then(date);
        }
    }
    let header_total = info["total"].as_f64();
    let header_used =
        info["upload"].as_f64().unwrap_or(0.0) + info["download"].as_f64().unwrap_or(0.0);
    let meaningful = header_total.is_some_and(|v| v > 1024.0) || header_used > 1024.0;
    let invalid = header_total
        .is_some_and(|v| v > 0.0 && ((v <= 1024.0 && header_used <= 1024.0) || header_used > v));
    let hints = [remaining, used, total]
        .iter()
        .flatten()
        .any(|v| *v > 1024.0);
    if invalid || (hints && !meaningful) {
        if let Some(fields) = info.as_object_mut() {
            for key in ["upload", "download", "total"] {
                fields.remove(key);
            }
        }
        if hints {
            let capacity = total.or_else(|| header_total.filter(|v| *v > 1024.0));
            if let Some(used) = used.or_else(|| Some((capacity? - remaining?).max(0.0))) {
                info["upload"] = json!(used);
                info["download"] = json!(0);
            }
            if let Some(capacity) = capacity {
                if used.is_some() || remaining.is_some() {
                    info["total"] = json!(capacity);
                }
            }
        }
    }
    if info["expire"].is_null() {
        if let Some(value) = expiry {
            info["expire"] = value.into();
        }
    }
    for (key, header) in [
        ("profileWebPageUrl", "profile-web-page-url"),
        ("planName", "plan-name"),
    ] {
        if let Some(value) = headers[header].as_str() {
            let value: String = value
                .replace(['\r', '\n'], " ")
                .trim()
                .chars()
                .take(if key == "planName" { 200 } else { 1024 })
                .collect();
            if !value.is_empty()
                && (key != "profileWebPageUrl" || crate::network::validate_url(&value).is_ok())
            {
                info[key] = value.into();
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_millisecond_expiry_and_repairs_placeholder_traffic_from_nodes() {
        let info = subscription_info(
            &json!({"subscription-userinfo":"upload=1; download=1; total=3; expire=1893456000000"}),
            &[
                json!({"name":"套餐流量：100 GB"}),
                json!({"name":"剩余流量：25 GB"}),
            ],
        );
        assert_eq!(info["expire"], 1893456000i64);
        assert_eq!(info["total"], json!(100.0 * 1024.0_f64.powi(3)));
        assert_eq!(info["upload"], json!(75.0 * 1024.0_f64.powi(3)));
    }
}
