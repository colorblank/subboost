use crate::generator::{
    self, array, base_defaults, group_type, module_name, modules, strings, test_options, text,
    valid_port,
};
use anyhow::{Context, Result, bail, ensure};
use regex::RegexBuilder;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::LazyLock;

pub(crate) fn valid_test_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        ["http", "https"].contains(&u.scheme()) && u.username().is_empty() && u.password().is_none()
    })
}
pub(crate) fn normalize_advanced(value: &Value) -> Value {
    let mut result = Map::new();
    for key in ["sourceIds", "regions"] {
        let mut values = strings(&value[key]);
        if key == "regions" {
            values = values
                .into_iter()
                .map(|s| s.to_lowercase())
                .filter(|s| crate::generator::groups::is_valid_region(s))
                .collect();
        }
        if !values.is_empty() {
            result.insert(key.to_owned(), json!(values));
        }
    }
    for key in ["includeRegex", "excludeRegex"] {
        if !text(value, key).is_empty() {
            result.insert(key.to_owned(), json!(text(value, key)));
        }
    }
    if generator::settings::valid_test_url(text(value, "testUrl")) {
        result.insert("testUrl".to_owned(), json!(text(value, "testUrl")));
    }
    if group_type(text(value, "groupType")) {
        result.insert("groupType".to_owned(), json!(text(value, "groupType")));
        if value["groupType"] == "load-balance" {
            result.insert(
                "strategy".to_owned(),
                json!(generator::strategy(&value["strategy"])),
            );
        }
    }
    for key in ["extraMembers", "excludedMembers", "memberOrder"] {
        let mut seen = std::collections::HashSet::new();
        let refs = array(value, key)
            .iter()
            .filter_map(|r| {
                let kind = text(r, "kind");
                let reference = match kind {
                    "direct" | "reject" => json!({"kind":kind}),
                    "node" if !text(r, "name").is_empty() => {
                        json!({"kind":kind,"name":text(r,"name")})
                    }
                    "module" | "custom" if !text(r, "id").is_empty() => {
                        json!({"kind":kind,"id":text(r,"id")})
                    }
                    _ => return None,
                };
                if seen.insert(reference.to_string()) {
                    Some(reference)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !refs.is_empty() {
            result.insert(key.to_owned(), json!(refs));
        }
    }
    Value::Object(result)
}

pub(crate) fn parse_yaml_mapping(input: &str) -> Result<Map<String, Value>> {
    let value = crate::parser::parse_yaml_value(input).context("基础和 DNS 配置 YAML 解析失败")?;
    value.as_object().cloned().context("基础和 DNS 配置必须是 YAML 对象（顶层 key/value 映射），不能是字符串、数组、数字、布尔值或 null。")
}

/// Rule files deliberately reject aliases even though base configuration supports them.
pub(crate) fn parse_rule_yaml_mapping(input: &str) -> Result<Map<String, Value>> {
    let max_bytes =
        crate::parser_policy::policy().map_or(10 * 1024 * 1024, |p| p.yaml_maximum_text_bytes);
    ensure!(input.len() <= max_bytes, "YAML 超过大小限制");
    let mut quote = None;
    let mut escaped = false;
    let mut previous = ' ';
    let mut comment = false;
    let mut depth: usize = 0;
    for c in input.chars() {
        if c == '\n' {
            comment = false;
        }
        if comment {
            continue;
        }
        if let Some(q) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if q == '"' && c == '\\' {
                escaped = true;
                continue;
            }
            if c == q {
                quote = None;
            }
        } else {
            if c == '#' && previous.is_whitespace() {
                comment = true;
                continue;
            }
            if (c == '\'' || c == '"') && (previous.is_whitespace() || "[{,:-".contains(previous)) {
                quote = Some(c);
            }
            if (c == '&' || c == '*') && (previous.is_whitespace() || "[{,:".contains(previous)) {
                bail!("YAML 不支持锚点或别名，请展开配置后重试");
            }
            if c == '[' || c == '{' {
                depth += 1;
                ensure!(depth <= 100, "YAML 嵌套过深");
            }
            if c == ']' || c == '}' {
                depth = depth.saturating_sub(1);
            }
        }
        previous = c;
    }
    parse_yaml_mapping(input)
}

pub(super) fn base_config(config: &Value) -> Result<Map<String, Value>> {
    if let Some(yaml) = config["dnsYaml"].as_str() {
        let mut base = if yaml.trim().is_empty() {
            Map::new()
        } else {
            parse_yaml_mapping(yaml)?
        };
        let conflicts = ["proxies", "proxy-groups", "rule-providers", "rules"]
            .into_iter()
            .filter(|key| base.contains_key(*key))
            .collect::<Vec<_>>();
        ensure!(
            conflicts.is_empty(),
            "基础和 DNS 配置不能包含 {}；这些段由 SubBoost 根据节点、代理组和规则生成。",
            conflicts.join(", ")
        );
        let policy = base
            .get("nameserver-policy")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| {
                        if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                            Some((k.clone(), json!(s)))
                        } else if v.is_array() {
                            let values = strings(v);
                            if values.is_empty() {
                                None
                            } else {
                                Some((k.clone(), json!(values)))
                            }
                        } else {
                            None
                        }
                    })
                    .collect::<Map<String, Value>>()
            })
            .unwrap_or_default();
        if !policy.is_empty() {
            base.remove("nameserver-policy");
            let dns = base.entry("dns").or_insert(json!({}));
            ensure!(
                dns.is_object(),
                "基础和 DNS 配置中的 dns 必须是对象，才能合并顶层 nameserver-policy。"
            );
            dns.as_object_mut()
                .unwrap()
                .entry("nameserver-policy")
                .or_insert(Value::Object(policy));
        }
        return Ok(base);
    }
    static DNS: LazyLock<Value> = LazyLock::new(|| {
        let text = crate::config::load_file_or_embedded(
            "SUBBOOST_MIHOMO_DNS",
            "config/mihomo/dns.json",
            include_str!("../../../config/mihomo/dns.json"),
        );
        serde_json::from_str(&text).expect("embedded DNS data")
    });
    let mut base = base_defaults()
        .as_object()
        .expect("base defaults are an object")
        .clone();
    if let Some(port) = valid_port(&config["mixedPort"]) {
        base.insert("mixed-port".into(), port.into());
    }
    if let Some(allow_lan) = config["allowLan"].as_bool() {
        base.insert("allow-lan".into(), allow_lan.into());
    }
    base.insert("dns".into(), DNS.clone());
    Ok(base)
}

pub(super) fn source_providers(config: &Value) -> Map<String, Value> {
    let mut providers = Map::new();
    let mut fallback = 0;
    let (test_url, interval) = test_options(config);
    for source in array(config, "sources") {
        if source["type"] != "url" || source["useProxyProviders"] != true {
            continue;
        }
        let raw = text(source, "content");
        let Ok(mut url) = url::Url::parse(raw) else {
            continue;
        };
        if !["http", "https"].contains(&url.scheme()) {
            continue;
        }
        if url.query().is_none() {
            let path = url.path().to_owned();
            if let Some((clean, dirty)) = path
                .split_once('&')
                .filter(|(clean, dirty)| !clean.is_empty() && !dirty.is_empty())
            {
                url.set_path(clean);
                for (key, value) in url::form_urlencoded::parse(dirty.as_bytes()) {
                    url.query_pairs_mut().append_pair(&key, &value);
                }
            }
        }
        let normalized = url.to_string();
        let id = if text(source, "id").is_empty() {
            fallback += 1;
            fallback.to_string()
        } else {
            text(source, "id").to_owned()
        };
        let safe = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let name = format!("url_{safe}");
        providers.entry(name.clone()).or_insert_with(|| json!({"type":"http","url":normalized,"interval":3600,"path":format!("./proxy_providers/{name}.yaml"),"health-check":{"enable":true,"url":test_url,"interval":interval}}));
    }
    providers
}

fn safe_regex(pattern: &str) -> bool {
    let mut groups = vec![false];
    let mut closed_repetition = false;
    let mut escaped = false;
    let mut character_class = false;
    let mut previous = '\0';
    let mut repetitions = 0;
    for c in pattern.chars() {
        if escaped {
            escaped = false;
            previous = c;
            closed_repetition = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '[' {
            character_class = true;
        }
        if character_class {
            if c == ']' {
                character_class = false;
            }
            continue;
        }
        match c {
            '(' => {
                groups.push(false);
                closed_repetition = false;
            }
            ')' => {
                closed_repetition = groups.pop().unwrap_or(false);
                if let Some(parent) = groups.last_mut() {
                    *parent |= closed_repetition;
                }
            }
            '*' | '+' | '?' | '{'
                if !(c == '?' && (previous == '(' || "*+?}".contains(previous))) =>
            {
                repetitions += 1;
                if closed_repetition || repetitions > 25 {
                    return false;
                }
                if let Some(group) = groups.last_mut() {
                    *group = true;
                }
                closed_repetition = false;
            }
            _ => {
                closed_repetition = false;
            }
        }
        previous = c;
    }
    true
}

pub(super) enum FilterRegex {
    Linear(regex::Regex),
    Extended(fancy_regex::Regex),
}
impl FilterRegex {
    pub(super) fn compile(pattern: &str) -> Result<Self> {
        let policy = crate::parser_policy::policy()?;
        if let Ok(re) = RegexBuilder::new(pattern)
            .case_insensitive(true)
            .size_limit(policy.regex_maximum_bytes)
            .build()
        {
            return Ok(Self::Linear(re));
        }
        let re = fancy_regex::RegexBuilder::new(&format!("(?i){pattern}"))
            .backtrack_limit(policy.regex_backtrack_limit)
            .build()
            .context("节点名称过滤配置无效：正则语法无效")?;
        Ok(Self::Extended(re))
    }
    pub(super) fn matches(&self, text: &str) -> Result<bool> {
        match self {
            Self::Linear(re) => Ok(re.is_match(text)),
            Self::Extended(re) => re
                .is_match(text)
                .context("节点名称过滤正则超过安全计算限制"),
        }
    }
}
pub(super) struct NodeFilter {
    enabled: bool,
    includes: Vec<FilterRegex>,
    excludes: Vec<FilterRegex>,
}
impl NodeFilter {
    pub fn new(value: Option<&Value>) -> Result<Self> {
        let Some(value) = value else {
            return Ok(Self {
                enabled: false,
                includes: vec![],
                excludes: vec![],
            });
        };
        ensure!(
            value.is_object() && value["enabled"].is_boolean(),
            "节点名称过滤配置无效：enabled 必须是布尔值"
        );
        for key in ["includeEnabled", "excludeEnabled"] {
            ensure!(
                value.get(key).is_none_or(Value::is_boolean),
                "节点名称过滤配置无效：{key} 必须是布尔值"
            );
        }
        let parse = |key: &str, optional: bool| -> Result<Vec<FilterRegex>> {
            if optional && value.get(key).is_none() {
                return Ok(vec![]);
            }
            let values = value[key]
                .as_array()
                .context("节点名称过滤配置无效：规则必须是字符串数组")?;
            ensure!(
                values.iter().all(Value::is_string),
                "节点名称过滤配置无效：规则必须是文本"
            );
            let patterns = strings(&value[key]);
            ensure!(
                patterns.len() <= 20,
                "节点名称过滤配置无效：最多允许 20 条规则"
            );
            patterns
                .iter()
                .map(|pattern| {
                    ensure!(
                        pattern.chars().count() <= 200,
                        "节点名称过滤配置无效：每条规则最多 200 个字符"
                    );
                    ensure!(
                        safe_regex(pattern),
                        "节点名称过滤配置无效：正则可能导致运行时间过长"
                    );
                    FilterRegex::compile(pattern)
                })
                .collect()
        };
        let mut includes = parse("includeRegexes", true)?;
        let mut excludes = parse("excludeRegexes", false)?;
        if value["includeEnabled"] == false {
            includes.clear();
        }
        if value["excludeEnabled"] == false {
            excludes.clear();
        }
        Ok(Self {
            enabled: value["enabled"] == true,
            includes,
            excludes,
        })
    }
    pub fn includes(&self, node: &Value) -> Result<bool> {
        if !self.enabled {
            return Ok(true);
        }
        let origin = if text(node, "_originName").is_empty() {
            text(node, "name")
        } else {
            text(node, "_originName")
        };
        let mut included = self.includes.is_empty();
        for pattern in &self.includes {
            if pattern.matches(origin)? {
                included = true;
                break;
            }
        }
        if !included {
            return Ok(false);
        }
        for pattern in &self.excludes {
            if pattern.matches(origin)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(super) fn listeners(
    base: &mut Map<String, Value>,
    nodes: &[Value],
    groups: &[Value],
    config: &Value,
    dialers: &[Value],
) -> Result<()> {
    ensure!(
        base.get("listeners").is_none_or(Value::is_array),
        "基础和 DNS 配置中的 listeners 必须是数组，才能与节点监听端口合并。"
    );
    let mut listeners = base
        .get("listeners")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut node_ports = HashSet::new();
    let mut index = 0;
    for node in nodes {
        if let Some(port) = valid_port(&config["listenerPorts"][text(node, "name")]) {
            if !node_ports.insert(port) {
                continue;
            }
            listeners.push(json!({"name":format!("mixed{index}"),"type":"mixed","port":port,"proxy":text(node,"name")}));
            index += 1;
        }
    }
    let mut used_ports: HashSet<u64> = listeners
        .iter()
        .filter_map(|l| valid_port(&l["port"]))
        .collect();
    if let Some(port) = base.get("mixed-port").and_then(valid_port) {
        used_ports.insert(port);
    }
    let mut names: HashSet<String> = listeners
        .iter()
        .map(|l| text(l, "name").to_owned())
        .collect();
    let mut seen_targets = HashSet::new();
    let mut group_index = 0;
    for binding in array(config, "groupListeners") {
        if valid_port(&binding["port"]).is_none() {
            continue;
        }
        let target = &binding["target"];
        let id = text(target, "id");
        if id.is_empty() {
            continue;
        }
        let name = match text(target, "kind") {
            "module" => modules()
                .iter()
                .find(|m| text(m, "id") == id)
                .map(|m| module_name(m, config)),
            "custom" => array(config, "customProxyGroups")
                .iter()
                .find(|g| text(g, "id") == id)
                .map(|g| text(g, "name").to_owned()),
            "dialer" => array(config, "dialerProxyGroups")
                .iter()
                .find(|g| text(g, "id") == id)
                .map(|g| text(g, "name").to_owned()),
            _ => continue,
        }
        .context(
            "分组监听的目标策略组已被删除，请在对应策略组的高级设置中移除或修改该监听配置。",
        )?;
        if binding["enabled"] == false || !groups.iter().any(|g| text(g, "name") == name) {
            continue;
        }
        if text(target, "kind") == "custom"
            && array(config, "customProxyGroups")
                .iter()
                .any(|g| text(g, "id") == id && g["enabled"] == false)
        {
            continue;
        }
        if text(target, "kind") == "dialer" && !dialers.iter().any(|g| text(g, "id") == id) {
            continue;
        }
        if !seen_targets.insert(format!("{}:{id}", text(target, "kind"))) {
            continue;
        }
        let port = valid_port(&binding["port"]).unwrap();
        ensure!(
            used_ports.insert(port),
            "策略组「{name}」的监听端口 {port} 与已有监听端口冲突，请修改后再生成。"
        );
        let listener_name = loop {
            let name = format!("group-mixed-{group_index}");
            group_index += 1;
            if names.insert(name.clone()) {
                break name;
            }
        };
        listeners.push(json!({"name":listener_name,"type":"mixed","listen":if binding["allowLan"] == true {"0.0.0.0"} else {"127.0.0.1"},"port":port,"proxy":name,"udp":true}));
    }
    if !listeners.is_empty() {
        base.insert("listeners".to_owned(), json!(listeners));
    }
    Ok(())
}

pub(super) fn share_health_checks(config: &mut Map<String, Value>) {
    let nodes = config
        .get("proxies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(groups) = config.get("proxy-groups").and_then(Value::as_array) else {
        return;
    };
    let mut cohorts: Vec<(Value, Vec<String>, Vec<usize>)> = vec![];
    for (index, group) in groups.iter().enumerate() {
        if group["type"] != "url-test"
            || group["lazy"] != false
            || !group["url"].is_string()
            || [
                "use",
                "filter",
                "exclude-filter",
                "exclude-type",
                "include-all",
                "include-all-proxies",
                "include-all-providers",
            ]
            .iter()
            .any(|k| group.get(*k).is_some())
        {
            continue;
        }
        let members = strings(&group["proxies"]);
        if members.is_empty()
            || members.len() != array(group, "proxies").len()
            || members
                .iter()
                .any(|n| !nodes.iter().any(|node| text(node, "name") == n))
        {
            continue;
        }
        let Some(interval) = group["interval"].as_u64() else {
            continue;
        };
        let mut health = json!({"enable":true,"url":group["url"],"interval":if interval==0 {300} else {interval},"timeout":group.get("timeout").cloned().unwrap_or(json!(5000)),"lazy":false});
        if let Some(status) = group.get("expected-status") {
            health["expected-status"] = status.clone();
        }
        if let Some(cohort) = cohorts.iter_mut().find(|c| c.0 == health && c.1 == members) {
            cohort.2.push(index);
        } else {
            cohorts.push((health, members, vec![index]));
        }
    }
    let mut providers = config
        .get("proxy-providers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut used: HashSet<String> = providers
        .keys()
        .cloned()
        .chain(nodes.iter().map(|n| text(n, "name").to_owned()))
        .chain(groups.iter().map(|g| text(g, "name").to_owned()))
        .collect();
    let mut replacements = groups.clone();
    let mut counter = 1;
    let mut changed = false;
    for (health, members, indices) in cohorts {
        if indices.len() < 2 {
            continue;
        }
        changed = true;
        let name = loop {
            let name = format!("subboost_shared_{counter}");
            counter += 1;
            if used.insert(name.clone()) {
                break name;
            }
        };
        let payload: Vec<Value> = members
            .iter()
            .filter_map(|name| nodes.iter().find(|n| text(n, "name") == name).cloned())
            .collect();
        providers.insert(
            name.clone(),
            json!({"type":"inline","payload":payload,"health-check":health}),
        );
        for index in indices {
            replacements[index]
                .as_object_mut()
                .unwrap()
                .remove("proxies");
            replacements[index]["use"] = json!([name]);
        }
    }
    if changed {
        config.insert("proxy-providers".to_owned(), Value::Object(providers));
        config.insert("proxy-groups".to_owned(), json!(replacements));
    }
}
