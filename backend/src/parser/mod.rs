//! 订阅与协议节点解析引擎。
//!
//! 支持自动识别 Base64 订阅包装、标准/非标分享链接、Clash/Mihomo YAML 配置、
//! Surge 托管配置以及 SIP002/SIP008、SSD 等多种格式，并在解析过程中应用严格的
//! 资源上限保护与协议清洗验证。

use anyhow::{Result, anyhow, bail};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

mod legacy_yaml;
mod links;
mod platform;
pub(crate) mod policy;
#[cfg(test)]
mod tests;
mod validation;
mod yaml;

pub(crate) use policy::{ParserPolicy, policy};
pub use validation::sanitize_for_generation;

/// 解析 YAML 文本为 JSON Value，应用防深度嵌套与实体爆炸的安全预算。
pub fn parse_yaml_value(content: &str) -> Result<Value> {
    check_input(content)?;
    yaml::load(content)
}

const STRING_FIELDS: &[&str] = &[
    "password",
    "username",
    "psk",
    "auth-str",
    "token",
    "private-key-passphrase",
    "obfs-password",
];

fn limited(reason: &str) -> anyhow::Error {
    anyhow!("订阅解析受限: {reason}")
}
fn check_items(count: usize) -> Result<()> {
    if count > policy::policy()?.maximum_nodes {
        return Err(limited("节点或分段数量超过上限"));
    }
    Ok(())
}
fn check_input(content: &str) -> Result<()> {
    let policy = policy::policy()?;
    if content.len() > policy.maximum_input_bytes {
        return Err(limited("输入内容超过大小上限"));
    }
    let mut lines = 1;
    let mut previous = 0;
    for byte in content.bytes() {
        if byte == b'\r' || (byte == b'\n' && previous != b'\r') {
            lines += 1;
            if lines > policy.maximum_lines {
                return Err(limited("输入行数超过上限"));
            }
        }
        previous = byte;
    }
    Ok(())
}
fn decode_base64(raw: &str) -> Result<String> {
    let compact: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(&compact) {
            return String::from_utf8(bytes).map_err(|_| anyhow!("Base64 内容不是有效 UTF-8 文本"));
        }
    }
    bail!("无效的 Base64 编码")
}
fn decode_percent(raw: &str, form: bool) -> String {
    let mut bytes = Vec::with_capacity(raw.len());
    let input = raw.as_bytes();
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            if let (Some(a), Some(b)) = (
                (input[i + 1] as char).to_digit(16),
                (input[i + 2] as char).to_digit(16),
            ) {
                bytes.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        bytes.push(if form && input[i] == b'+' {
            b' '
        } else {
            input[i]
        });
        i += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| raw.to_owned())
}
fn text<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get(key).and_then(Value::as_str).unwrap_or("")
}
fn boolish(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(v) => Some(*v),
        Value::Number(n) if n.as_u64() == Some(1) => Some(true),
        Value::Number(n) if n.as_u64() == Some(0) => Some(false),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}
fn integer(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.trim().bytes().all(|c| c.is_ascii_digit()))
            .and_then(|s| s.trim().parse().ok())
    })
}
fn port(value: &Value) -> Result<u16> {
    integer(value)
        .filter(|n| (1..=65535).contains(n))
        .map(|n| n as u16)
        .ok_or_else(|| anyhow!("端口必须是 1 到 65535 的整数"))
}
fn list(raw: &str) -> Vec<Value> {
    raw.split([',', '|'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| json!(s))
        .collect()
}
fn port_spec(raw: &str) -> Result<(String, u16)> {
    let mut out = Vec::new();
    let mut first = None;
    for part in raw.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            bail!("无效的端口范围");
        }
        let mut range = part.split('-');
        let start = port(&json!(range.next().unwrap_or_default()))?;
        let end = range
            .next()
            .map(|p| port(&json!(p.trim())))
            .transpose()?
            .unwrap_or(start);
        if end < start || range.next().is_some() {
            bail!("无效的端口范围");
        }
        first.get_or_insert(start);
        out.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
    }
    Ok((
        out.join(","),
        first.ok_or_else(|| anyhow!("无效的端口范围"))?,
    ))
}

struct Parsed {
    policy: &'static ParserPolicy,
    nodes: Vec<Value>,
    errors: Vec<String>,
    bytes: usize,
}
impl Parsed {
    fn new() -> Result<Self> {
        Ok(Self {
            policy: policy::policy()?,
            nodes: Vec::new(),
            errors: Vec::new(),
            bytes: 0,
        })
    }
    fn error(&mut self, message: impl AsRef<str>) -> Result<()> {
        if self.errors.len() >= self.policy.maximum_errors {
            return Err(limited("解析错误数量超过上限"));
        }
        let message = message.as_ref();
        let maximum = self.policy.maximum_error_characters;
        let bounded = if message.chars().nth(maximum).is_some() {
            format!("{}…", message.chars().take(maximum - 1).collect::<String>())
        } else {
            message.to_owned()
        };
        self.errors.push(bounded);
        Ok(())
    }
    fn push(&mut self, mut node: Value) -> Result<()> {
        check_items(self.nodes.len() + 1)?;
        validation::normalize(&mut node, validation::ValidationMode::Imported)?;
        self.bytes += serde_json::to_vec(&node)?.len();
        if self.bytes > self.policy.maximum_output_bytes {
            return Err(limited("展开后的内容超过大小上限"));
        }
        self.nodes.push(node);
        Ok(())
    }
    fn node_result(&mut self, segment: &str, result: Result<Option<Value>>) -> Result<()> {
        let result = result.and_then(|node| match node {
            Some(node) => self.push(node),
            None => Ok(()),
        });
        if let Err(error) = result {
            if error.to_string().starts_with("订阅解析受限:") {
                return Err(error);
            }
            self.error(format!(
                "解析失败: {}... - {error}",
                segment.chars().take(50).collect::<String>()
            ))?;
        }
        Ok(())
    }
    fn finish(mut self) -> Value {
        let mut seen = HashSet::new();
        let mut placeholder = false;
        self.nodes.retain(|node| {
            if validation::placeholder(node) {
                placeholder = true;
                return false;
            }
            let mut identity = node.clone();
            if !validation::info_name(text(node, "name")) {
                identity.as_object_mut().map(|o| o.remove("name"));
            }
            validation::strip_control(&mut identity);
            seen.insert(validation::stable_key(&identity))
        });
        if placeholder && self.errors.len() < self.policy.maximum_errors {
            self.errors
                .push("检测到客户端更新提示占位节点，已自动忽略".into());
        }
        let mut unique = HashSet::new();
        self.errors
            .retain(|e| !e.trim().is_empty() && unique.insert(e.clone()));
        json!({"totalParsed": self.nodes.len(), "totalFailed": self.errors.len(), "nodes": self.nodes, "errors": self.errors})
    }
}

/// 解析订阅原始文本，自动探测 Base64、URI 分享链接、YAML 或 JSON 结构，并返回节点列表与解析错误。
pub fn parse(content: &str) -> Result<Value> {
    let mut parsed = Parsed::new()?;
    match parse_inner(content, &mut parsed) {
        Ok(()) => Ok(parsed.finish()),
        Err(error) => {
            let mut failed = Parsed::new()?;
            failed.error(error.to_string())?;
            Ok(failed.finish())
        }
    }
}

fn parse_inner(content: &str, parsed: &mut Parsed) -> Result<()> {
    check_input(content)?;
    let mut decoded = None;
    let mut content = content.trim_start_matches('\u{feff}').trim();
    if content.is_empty() {
        return Ok(());
    }
    // A subscription wrapper is decoded once; protocol payloads are decoded by their parser.
    if !content.contains("://")
        && !content.contains(':')
        && !content.contains('[')
        && !content.contains('{')
    {
        if let Ok(value) = decode_base64(content) {
            if value.contains("://")
                || value.contains("proxies:")
                || value.trim_start().starts_with(['[', '{'])
            {
                decoded = Some(value);
            }
        }
    }
    if let Some(value) = &decoded {
        check_input(value)?;
        content = value.trim();
    }
    let lower = content
        .chars()
        .take(300)
        .collect::<String>()
        .to_ascii_lowercase();
    if lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || lower.contains("<body")
        || lower.contains("<head")
    {
        parsed.error("检测到 HTML 内容，可能是错误页或拦截页，请检查订阅链接")?;
        return Ok(());
    }
    if content
        .get(..6)
        .is_some_and(|s| s.eq_ignore_ascii_case("ssd://"))
    {
        return parse_ssd(&decode_base64(&content[6..])?, parsed);
    }
    if content.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(content) {
            if let Some(servers) = value.get("Server") {
                let servers = servers
                    .as_array()
                    .ok_or_else(|| anyhow!("Netch 配置缺少 Server 列表"))?;
                check_items(servers.len())?;
                for entry in servers {
                    let value = if let Some(raw) = entry.as_str() {
                        serde_json::from_str(raw)?
                    } else {
                        entry.clone()
                    };
                    if value.is_null() {
                        continue;
                    }
                    parsed.node_result("Netch", links::netch(&value).map(Some))?;
                }
                return Ok(());
            }
        }
    }
    let platform_header = content.lines().any(|line| {
        let line = line.trim().to_ascii_lowercase();
        ["[proxy]", "[server_local]", "[server_remote]"].contains(&line.as_str())
            || line.starts_with("[wireguard ")
    });
    if platform_header {
        return platform::parse_config(content, parsed);
    }
    let first = content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with(['#', ';', '%']))
        .unwrap_or("");
    let yaml_like = !first.contains("://")
        && (first.starts_with(['{', '[', '-', '?', '!', '&'])
            || first.contains(": ")
            || first.ends_with(':'));
    if yaml_like
        || content
            .lines()
            .any(|l| l.trim_start().starts_with("proxies:"))
    {
        return yaml::parse(content, parsed);
    }
    let mut count = 0;
    for line in content
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with(['#', ';']))
    {
        if !line.contains("://") && line.contains('=') {
            count += 1;
            check_items(count)?;
            parsed.node_result(line, platform::parse_line(line, &Default::default()))?;
            continue;
        }
        let split = line.contains('|')
            && line
                .split('|')
                .filter(|s| !s.trim().is_empty())
                .all(|s| s.trim().contains("://"));
        if split {
            for segment in line.split('|').map(str::trim).filter(|s| !s.is_empty()) {
                count += 1;
                check_items(count)?;
                parsed.node_result(segment, links::parse_link(segment))?;
            }
        } else {
            count += 1;
            check_items(count)?;
            parsed.node_result(line, links::parse_link(line))?;
        }
    }
    Ok(())
}

fn parse_ssd(raw: &str, parsed: &mut Parsed) -> Result<()> {
    let data: Value = serde_json::from_str(raw)?;
    let servers: Vec<&Value> = match data.get("servers") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => bail!("SSD 订阅中未找到可转换的服务器条目"),
    };
    check_items(servers.len())?;
    for (i, server) in servers.into_iter().enumerate() {
        if !server.is_object() {
            continue;
        }
        let mut node = json!({"type":"ss", "server":text(server,"server"), "udp":true});
        for (target, source) in [
            ("port", "port"),
            ("cipher", "encryption"),
            ("password", "password"),
        ] {
            node[target] = server
                .get(source)
                .or_else(|| data.get(source))
                .cloned()
                .unwrap_or(Value::Null);
        }
        node["name"] = json!(if !text(server, "remarks").is_empty() {
            decode_percent(text(server, "remarks"), false)
        } else if !text(&data, "airport").is_empty() {
            format!("{}-{}", text(&data, "airport"), i + 1)
        } else {
            format!("SSD-{}", i + 1)
        });
        if !text(server, "plugin").is_empty() {
            links::ss_plugin(
                &mut node,
                &format!(
                    "{};{}",
                    text(server, "plugin"),
                    text(server, "plugin_options")
                ),
            )?;
        }
        parsed.node_result("SSD", Ok(Some(node)))?;
    }
    if parsed.nodes.is_empty() {
        parsed.error("SSD 订阅中未找到可转换的服务器条目")?;
    }
    Ok(())
}

/// 在持久化存储前全量验证节点列表，规范化字段格式并清除控制字符。
pub fn validate_nodes(value: &Value) -> Result<Vec<Value>> {
    let items = value
        .as_array()
        .ok_or_else(|| anyhow!("节点列表必须是数组"))?;
    let policy = policy::policy()?;
    if items.len() > policy.maximum_nodes {
        bail!("Node count exceeds parser safety policy");
    }
    let mut nodes = Vec::with_capacity(items.len());
    let mut bytes = 0;
    for (index, item) in items.iter().enumerate() {
        let mut node = item.clone();
        validation::normalize(&mut node, validation::ValidationMode::Persisted)
            .map_err(|error| anyhow!("节点 #{} {error}", index + 1))?;
        validation::strip_control(&mut node);
        bytes += serde_json::to_vec(&node)?.len();
        if bytes > policy.maximum_output_bytes {
            return Err(limited("节点内容超过大小上限"));
        }
        nodes.push(node);
    }
    Ok(nodes)
}
