//! Mihomo (Clash.Meta) YAML 配置生成核心。
//!
//! 根据用户选择的模版或自定义配置，合并代理组定义、节点过滤与链式引用、
//! 规则集路由、DNS 与基础网络参数，最终输出合规的 Mihomo YAML 文件。

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::LazyLock};

mod groups;
pub(crate) mod routing;
pub(crate) mod settings;

#[cfg(test)]
mod regression_tests;

pub(crate) fn default_generation() -> &'static crate::config::Generation {
    static DEFAULTS: LazyLock<crate::config::Generation> =
        LazyLock::new(|| crate::config::Config::default().generation);
    &DEFAULTS
}
pub(crate) fn base_defaults() -> &'static Value {
    static DEFAULTS: LazyLock<Value> = LazyLock::new(|| {
        let text = crate::config::load_file_or_embedded(
            "SUBBOOST_MIHOMO_BASE",
            "config/mihomo/base.json",
            include_str!("../../../config/mihomo/base.json"),
        );
        serde_json::from_str(&text).expect("valid Mihomo base defaults")
    });
    &DEFAULTS
}
pub(crate) fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("").trim()
}
pub(crate) fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub(crate) fn strings(v: &Value) -> Vec<String> {
    unique(
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    )
}
pub(crate) fn unique(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}
pub(crate) fn modules() -> &'static [Value] {
    static DATA: LazyLock<Vec<Value>> = LazyLock::new(|| {
        let text = crate::config::load_file_or_embedded(
            "SUBBOOST_MIHOMO_MODULES",
            "config/mihomo/modules.json",
            include_str!("../../../config/mihomo/modules.json"),
        );
        serde_json::from_str(&text).expect("embedded module data")
    });
    DATA.as_slice()
}
pub(crate) fn template_modules(template: &str) -> Vec<String> {
    match template {
        "minimal" => ["select", "auto", "ad", "private", "cn", "global", "final"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        "full" => modules()
            .iter()
            .filter(|m| !["adult", "gemini", "google-scholar"].contains(&text(m, "id")))
            .map(|m| text(m, "id").to_owned())
            .collect(),
        _ => [
            "select",
            "auto",
            "ad",
            "private",
            "cn",
            "global",
            "ai",
            "youtube",
            "google",
            "microsoft",
            "apple",
            "github",
            "telegram",
            "final",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    }
}
pub(crate) fn module_name(module: &Value, config: &Value) -> String {
    let raw = config["proxyGroupNameOverrides"][text(module, "id")]
        .as_str()
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        return text(module, "name").to_owned();
    }
    let first = raw.split_whitespace().next().unwrap_or("");
    let emoji_prefix = first.len() < raw.len()
        && !first
            .chars()
            .any(|c| c.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&c));
    if emoji_prefix {
        raw.to_owned()
    } else {
        format!("{} {raw}", text(module, "emoji")).trim().to_owned()
    }
}
pub(crate) fn info_node(name: &str) -> bool {
    ["版本太旧", "请更新", "更新软件", "去官网更新"]
        .iter()
        .any(|s| name.contains(s))
        || [
            "剩余流量",
            "剩余订阅流量",
            "距离下次重置剩余",
            "套餐到期",
            "订阅到期",
            "到期时间",
            "过期时间",
        ]
        .iter()
        .any(|s| {
            name.strip_prefix(s)
                .is_some_and(|tail| tail.trim_start().starts_with([':', '：']))
        })
        || ["余额", "会员"].iter().any(|s| {
            name.strip_prefix(s)
                .is_some_and(|tail| tail.trim_start().starts_with('|'))
        })
}
pub(crate) fn valid_port(value: &Value) -> Option<u64> {
    value.as_u64().filter(|p| (1..=65535).contains(p))
}
pub(crate) fn group_type(value: &str) -> bool {
    [
        "select",
        "url-test",
        "fallback",
        "load-balance",
        "direct-first",
        "reject-first",
    ]
    .contains(&value)
}
pub(crate) fn strategy(value: &Value) -> &str {
    match value.as_str() {
        Some(s @ ("consistent-hashing" | "round-robin" | "sticky-sessions")) => s,
        _ => "consistent-hashing",
    }
}
pub(crate) fn test_options(config: &Value) -> (&str, u64) {
    let url = text(config, "testUrl");
    (
        if url.starts_with("http") {
            url
        } else {
            &default_generation().test_url
        },
        config["testInterval"]
            .as_u64()
            .unwrap_or(default_generation().test_interval_seconds),
    )
}
pub(crate) fn active_custom(config: &Value) -> Vec<Value> {
    array(config, "customProxyGroups")
        .iter()
        .filter(|g| {
            !text(g, "id").is_empty()
                && !text(g, "name").is_empty()
                && group_type(text(g, "groupType"))
        })
        .cloned()
        .collect()
}

/// Generate from the persisted subscription JSON contract, not frontend GenerateOptions.
/// 根据已校验的节点列表和订阅配置生成最终的 Mihomo YAML 文本。
pub fn generate(nodes: &[Value], config: &Value) -> Result<String> {
    let value = generate_config(nodes, config)?;
    serde_yaml::to_string(&value).context("无法序列化 Clash YAML")
}

pub fn generate_config(nodes: &[Value], raw: &Value) -> Result<Value> {
    ensure!(raw.is_object() || raw.is_null(), "订阅配置必须是对象");
    let mut config = if raw.is_object() {
        raw.clone()
    } else {
        json!({})
    };
    let mut enabled = strings(&config["enabledGroups"]);
    if enabled.is_empty() {
        enabled = template_modules(text(&config, "template"));
    }
    config["enabledGroups"] = json!(enabled);
    config["customProxyGroups"] = json!(active_custom(&config));
    let mut base = settings::base_config(&config)?;
    let source_providers = settings::source_providers(&config);
    let mut provider_names: Vec<String> = source_providers.keys().cloned().collect();
    provider_names.sort();
    if !source_providers.is_empty() {
        let slot = base.entry("proxy-providers").or_insert(json!({}));
        ensure!(
            slot.is_object(),
            "基础和 DNS 配置中的 proxy-providers 必须是对象，才能与 URL 源 proxy-providers 合并。"
        );
        slot.as_object_mut().unwrap().extend(source_providers);
    }
    let fingerprint = base
        .get("global-client-fingerprint")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let mut output_nodes = Vec::new();
    let mut used_names = HashSet::new();
    let filter = settings::NodeFilter::new(config.get("nodeNameFilter"))?;
    for node in nodes {
        if !node.is_object() || !filter.includes(node)? {
            continue;
        }
        let mut node = node.clone();
        let typ = text(&node, "type");
        if !fingerprint.is_empty()
            && text(&node, "client-fingerprint").is_empty()
            && ["vmess", "vless", "trojan", "anytls"].contains(&typ)
            && (["trojan", "anytls"].contains(&typ)
                || node["tls"] == true
                || (typ == "vless" && node.get("reality-opts").is_some()))
        {
            node["client-fingerprint"] = json!(fingerprint);
        }
        let source_ids = node["_sourceIds"].clone();
        let base_name = match node.get("name") {
            Some(Value::String(s)) => s.trim().to_owned(),
            Some(Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        };
        node["name"] = json!(if base_name.is_empty() {
            "未命名节点"
        } else {
            &base_name
        });
        let Some(mut node) = crate::parser::sanitize_for_generation(&node) else {
            continue;
        };
        if let Some(map) = node.as_object_mut() {
            map.remove("dialer-proxy");
            map.remove("dialer_proxy");
        }
        let base_name = text(&node, "name").to_owned();
        let mut name = base_name.clone();
        let mut index = 2;
        while !used_names.insert(name.clone()) {
            name = format!("{base_name} ({index})");
            index += 1;
        }
        node["name"] = json!(name);
        if source_ids.is_array() {
            node["_sourceIds"] = source_ids;
        }
        output_nodes.push(node);
    }
    let node_names: HashSet<String> = output_nodes
        .iter()
        .map(|n| text(n, "name").to_owned())
        .collect();
    let relay_group_names: HashSet<String> = modules()
        .iter()
        .filter(|m| enabled.iter().any(|id| id == text(m, "id")))
        .map(|m| module_name(m, &config))
        .chain(
            array(&config, "customProxyGroups")
                .iter()
                .filter(|g| g["enabled"] != false)
                .map(|g| text(g, "name").to_owned()),
        )
        .collect();
    let mut dialers = Vec::new();
    for group in array(&config, "dialerProxyGroups") {
        if group["enabled"] == false
            || text(group, "id").is_empty()
            || text(group, "name").is_empty()
            || !group_type(text(group, "type"))
        {
            continue;
        }
        let mut group = group.clone();
        let relays: Vec<String> = strings(&group["relayNodes"])
            .into_iter()
            .filter(|n| n == "DIRECT" || node_names.contains(n) || relay_group_names.contains(n))
            .collect();
        let targets: Vec<String> = if relays.is_empty() {
            vec![]
        } else {
            strings(&group["targetNodes"])
                .into_iter()
                .filter(|n| node_names.contains(n))
                .collect()
        };
        group["relayNodes"] = json!(relays);
        group["targetNodes"] = json!(targets);
        dialers.push(group);
    }
    let valid_dialers: HashSet<String> = node_names
        .iter()
        .cloned()
        .chain(relay_group_names)
        .chain(provider_names.iter().cloned())
        .chain(dialers.iter().map(|d| text(d, "name").to_owned()))
        .chain(["DIRECT".to_owned()])
        .collect();
    for node in &mut output_nodes {
        if let Some(group) = dialers.iter().find(|d| {
            strings(&d["targetNodes"])
                .iter()
                .any(|n| n == text(node, "name"))
        }) {
            node["dialer-proxy"] = json!(text(group, "name"));
        }
        let name = text(node, "dialer-proxy");
        if !name.is_empty() && !valid_dialers.contains(name) {
            node.as_object_mut().unwrap().remove("dialer-proxy");
        }
    }
    let proxy_groups = groups::generate(&output_nodes, &config, &provider_names, &dialers);
    settings::listeners(&mut base, &output_nodes, &proxy_groups, &config, &dialers)?;
    let (providers, rules) = routing::generate(&output_nodes, &proxy_groups, &config);
    for node in &mut output_nodes {
        if let Some(map) = node.as_object_mut() {
            map.retain(|key, _| !key.starts_with('_'));
        }
    }
    base.insert("proxies".to_owned(), json!(output_nodes));
    base.insert("proxy-groups".to_owned(), json!(proxy_groups));
    base.insert("rule-providers".to_owned(), Value::Object(providers));
    base.insert("rules".to_owned(), json!(rules));
    if config["sharedHealthChecks"] == true {
        settings::share_health_checks(&mut base);
    }
    Ok(Value::Object(base))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(name: &str) -> Value {
        json!({"name":name,"type":"ss","server":"example.com","port":443,"cipher":"aes-128-gcm","password":"secret","future-opt":{"value":true}})
    }
    #[test]
    fn strict_empty_base_and_unique_names() {
        let value = generate_config(
            &[node("A"), node("A (2)"), node("A")],
            &json!({"dnsYaml":""}),
        )
        .unwrap();
        assert!(value.get("dns").is_none());
        assert_eq!(value["proxies"][2]["name"], "A (3)");
        assert_eq!(value["proxies"][0]["future-opt"]["value"], true);
        assert!(generate_config(&[], &json!({"dnsYaml":"proxies: []"})).is_err());
    }
    #[test]
    fn missing_base_gets_defaults_but_empty_does_not() {
        assert_eq!(
            generate_config(&[], &json!({})).unwrap()["mixed-port"],
            7897
        );
        let value = generate_config(
            &[],
            &json!({"dnsYaml":"nameserver-policy:\n  '+.example.com': 1.1.1.1"}),
        )
        .unwrap();
        assert_eq!(
            value["dns"]["nameserver-policy"]["+.example.com"],
            "1.1.1.1"
        );
    }
    #[test]
    fn listener_collisions_are_errors_and_disabled_targets_pause() {
        let cfg = json!({"enabledGroups":["select"],"groupListeners":[{"target":{"kind":"module","id":"select"},"port":7897}]});
        assert!(generate_config(&[], &cfg).is_err());
        let cfg = json!({"enabledGroups":["select"],"groupListeners":[{"target":{"kind":"module","id":"auto"},"port":7897}]});
        assert!(generate_config(&[], &cfg).is_ok());
    }
}
