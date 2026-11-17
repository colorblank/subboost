use crate::generator::{
    array, default_generation, info_node, module_name, modules, strings, text, unique,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

const ORDER: &[&str] = &[
    "ad",
    "private",
    "gemini",
    "ai",
    "cn",
    "youtube",
    "google-scholar",
    "education",
    "cloud",
    "google",
    "telegram",
    "github",
    "microsoft",
    "apple",
    "twitter",
    "meta",
    "discord",
    "social-other",
    "netflix",
    "disney",
    "streaming-west",
    "streaming-asia",
    "steam",
    "gaming-pc",
    "gaming-console",
    "dev-tools",
    "storage",
    "payment",
    "crypto",
    "news",
    "shopping",
    "adult",
    "global",
];
pub(crate) const RULE_TYPES: &[&str] = &[
    "DOMAIN",
    "DOMAIN-SUFFIX",
    "DOMAIN-KEYWORD",
    "DOMAIN-WILDCARD",
    "DOMAIN-REGEX",
    "GEOSITE",
    "IP-CIDR",
    "IP-CIDR6",
    "IP-SUFFIX",
    "IP-ASN",
    "GEOIP",
    "SRC-GEOIP",
    "SRC-IP-ASN",
    "SRC-IP-CIDR",
    "SRC-IP-SUFFIX",
    "DST-PORT",
    "SRC-PORT",
    "IN-PORT",
    "IN-TYPE",
    "IN-USER",
    "IN-NAME",
    "REMATCH-NAME",
    "PROCESS-NAME",
    "PROCESS-PATH",
    "PROCESS-NAME-WILDCARD",
    "PROCESS-PATH-WILDCARD",
    "PROCESS-NAME-REGEX",
    "PROCESS-PATH-REGEX",
    "UID",
    "NETWORK",
    "DSCP",
    "AND",
    "OR",
    "NOT",
];
pub(crate) fn no_resolve(typ: &str) -> bool {
    [
        "IP-CIDR",
        "IP-CIDR6",
        "IP-SUFFIX",
        "IP-ASN",
        "GEOIP",
        "RULE-SET",
    ]
    .contains(&typ)
}
fn comma_payload(typ: &str) -> bool {
    [
        "DOMAIN-REGEX",
        "PROCESS-NAME-REGEX",
        "PROCESS-PATH-REGEX",
        "AND",
        "OR",
        "NOT",
    ]
    .contains(&typ)
}
fn logic(typ: &str) -> bool {
    ["AND", "OR", "NOT"].contains(&typ)
}
fn split_level(input: &str) -> Option<Vec<&str>> {
    let mut depth = 0i32;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, c) in input.char_indices() {
        if c == '(' {
            depth += 1;
        }
        if c == ')' {
            depth -= 1;
            if depth < 0 {
                return None;
            }
        }
        if c == ',' && depth == 0 {
            parts.push(input[start..index].trim());
            start = index + 1;
        }
    }
    if depth != 0 {
        return None;
    }
    parts.push(input[start..].trim());
    Some(parts)
}
fn unwrap(input: &str) -> Option<&str> {
    let inner = input.trim().strip_prefix('(')?.strip_suffix(')')?;
    split_level(inner)?;
    Some(inner)
}
pub(crate) fn normalize_rule_value(typ: &str, value: &str) -> Result<String> {
    normalize_value_inner(typ, value, 0)
}
fn normalize_value_inner(typ: &str, value: &str, depth: usize) -> Result<String> {
    ensure!(depth < 64, "逻辑规则嵌套过深");
    let value = value.trim();
    ensure!(!value.is_empty(), "规则值不能为空");
    ensure!(
        !value.contains(['\r', '\n', '\0']),
        "规则值不能包含换行或空字符"
    );
    ensure!(
        comma_payload(typ) || !value.contains(','),
        "该类型的规则值不能包含逗号"
    );
    if !logic(typ) {
        return Ok(value.to_owned());
    }
    let inner = unwrap(value).context("逻辑规则内容需形如 ((类型,值),(类型,值))")?;
    let parts = split_level(inner).context("逻辑规则内容格式错误")?;
    ensure!(
        !parts.is_empty() && parts.iter().all(|p| !p.is_empty()),
        "逻辑规则内容格式错误"
    );
    ensure!(
        typ != "NOT" || parts.len() == 1,
        "NOT 规则只能包含 1 个子条件"
    );
    let mut conditions = Vec::new();
    for part in parts {
        let condition = unwrap(part).context("子条件需用括号包裹")?;
        let (kind, rest) = condition.split_once(',').context("子条件缺少值")?;
        let kind = kind.trim().to_uppercase();
        ensure!(
            RULE_TYPES.contains(&kind.as_str()) || kind == "RULE-SET",
            "子条件类型无效：{kind}"
        );
        let normalized = if comma_payload(&kind) {
            normalize_value_inner(&kind, rest, depth + 1)?
        } else {
            let params = rest.split(',').map(str::trim).collect::<Vec<_>>();
            ensure!(params.iter().all(|p| !p.is_empty()), "子条件值不能为空");
            ensure!(
                params
                    .iter()
                    .skip(1)
                    .all(|p| no_resolve(&kind) && ["no-resolve", "src"].contains(p)),
                "子条件参数无效"
            );
            params.join(",")
        };
        conditions.push(format!("({kind},{normalized})"));
    }
    Ok(format!("({})", conditions.join(",")))
}
fn logic_references(typ: &str, value: &str) -> Vec<String> {
    if !logic(typ) {
        return vec![];
    }
    let Some(inner) = unwrap(value) else {
        return vec![];
    };
    let mut ids = Vec::new();
    for condition in split_level(inner).unwrap_or_default() {
        let Some((kind, rest)) = unwrap(condition).and_then(|c| c.split_once(',')) else {
            continue;
        };
        if kind == "RULE-SET" {
            ids.push(rest.split(',').next().unwrap_or("").to_owned());
        } else {
            ids.extend(logic_references(kind, rest));
        }
    }
    unique(ids)
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    let mut pending = false;
    for c in value.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            if pending && !result.is_empty() {
                result.push('-');
            }
            result.push(c);
            pending = false;
        } else {
            pending = true;
        }
    }
    if result.is_empty() {
        "item".to_owned()
    } else {
        result
    }
}
pub(crate) fn normalized_rules(config: &Value) -> Vec<Value> {
    array(config, "customRules")
        .iter()
        .enumerate()
        .filter_map(|(index, rule)| {
            let typ = text(rule, "type");
            if !RULE_TYPES.contains(&typ) || !valid_target(&rule["target"]) {
                return None;
            }
            let value = normalize_rule_value(typ, text(rule, "value")).ok()?;
            let mut rule = rule.clone();
            rule["value"] = json!(value);
            if text(&rule, "id").is_empty() {
                let target = if let Some(s) = rule["target"].as_str() {
                    s.to_owned()
                } else {
                    format!(
                        "{}:{}",
                        text(&rule["target"], "kind"),
                        text(&rule["target"], "id")
                    )
                };
                rule["id"] = json!(format!(
                    "custom-rule-{}-{}-{}-{}",
                    slug(typ),
                    slug(&value).chars().take(24).collect::<String>(),
                    slug(&target).chars().take(24).collect::<String>(),
                    index + 1
                ));
            }
            if let Some(value) = rule["noResolve"].as_bool() {
                rule["noResolve"] = json!(value && no_resolve(typ));
            }
            Some(rule)
        })
        .collect()
}
pub(crate) fn valid_target(target: &Value) -> bool {
    target.as_str().is_some_and(|s| !s.trim().is_empty())
        || (["module", "custom"].contains(&text(target, "kind")) && !text(target, "id").is_empty())
}
pub(crate) fn valid_ruleset(rule: &Value) -> bool {
    let format = rule["format"].as_str().unwrap_or("mrs");
    let behavior = text(rule, "behavior");
    let path = text(rule, "path").trim_start_matches('/');
    let path_ok = if path.starts_with("http://") || path.starts_with("https://") {
        true
    } else {
        ["geosite/", "geoip/"]
            .iter()
            .any(|p| path.to_lowercase().starts_with(p))
            && path.ends_with(".mrs")
            && !path.contains(['?', '#'])
            && !path.chars().any(char::is_whitespace)
            && path.split('/').count() == 2
    };
    !text(rule, "id").is_empty()
        && path_ok
        && valid_target(&rule["target"])
        && ["mrs", "yaml", "text"].contains(&format)
        && (["domain", "ipcidr"].contains(&behavior)
            || (behavior == "classical" && format != "mrs"))
}
pub(crate) fn normalized_sets(config: &Value) -> Vec<Value> {
    let mut seen = HashSet::new();
    array(config, "customRuleSets")
        .iter()
        .filter(|r| valid_ruleset(r) && seen.insert(text(r, "id").to_owned()))
        .map(|r| {
            let mut r = r.clone();
            r["id"] = json!(text(&r, "id"));
            r["path"] = json!(text(&r, "path").trim_start_matches('/'));
            if text(&r, "name").is_empty() {
                r["name"] = r["id"].clone();
            }
            r
        })
        .collect()
}
fn rule_url(path: &str, base: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_owned()
    } else {
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

struct Targets<'a> {
    config: &'a Value,
    names: Vec<(String, String)>,
    available: HashSet<String>,
    fallback: String,
}
impl<'a> Targets<'a> {
    fn new(nodes: &[Value], groups: &[Value], config: &'a Value) -> Self {
        let names = modules()
            .iter()
            .map(|m| (text(m, "id").to_owned(), module_name(m, config)))
            .collect::<Vec<_>>();
        let mut available: HashSet<String> = groups
            .iter()
            .map(|g| text(g, "name").to_owned())
            .chain(
                nodes
                    .iter()
                    .filter(|n| !info_node(text(n, "name")))
                    .map(|n| text(n, "name").to_owned()),
            )
            .collect();
        available.extend(["DIRECT".to_owned(), "REJECT".to_owned()]);
        let fallback = groups
            .first()
            .map(|g| text(g, "name").to_owned())
            .or_else(|| {
                nodes
                    .iter()
                    .find(|n| !info_node(text(n, "name")))
                    .map(|n| text(n, "name").to_owned())
            })
            .unwrap_or_else(|| "DIRECT".to_owned());
        Self {
            config,
            names,
            available,
            fallback,
        }
    }
    fn normalize(&self, target: &Value) -> Value {
        let Some(name) = target.as_str().map(str::trim) else {
            return target.clone();
        };
        if self.available.contains(name)
            || self.names.iter().any(|(_, n)| n == name)
            || array(self.config, "customProxyGroups")
                .iter()
                .any(|g| text(g, "name") == name)
        {
            return json!(name);
        }
        if let Some((kind, id)) = name.split_once(':') {
            if ["module", "custom"].contains(&kind) && !id.trim().is_empty() {
                return json!({"kind":kind,"id":id.trim()});
            }
        }
        json!(name)
    }
    fn state(&self, target: &Value) -> Option<bool> {
        let target = self.normalize(target);
        let enabled = strings(&self.config["enabledGroups"]);
        if let Some(name) = target.as_str() {
            let modules = self
                .names
                .iter()
                .filter(|(_, n)| n == name)
                .collect::<Vec<_>>();
            let custom = array(self.config, "customProxyGroups")
                .iter()
                .filter(|g| text(g, "name") == name)
                .collect::<Vec<_>>();
            if self.available.contains(name)
                || modules.iter().any(|(id, _)| enabled.contains(id))
                || custom.iter().any(|g| g["enabled"] != false)
            {
                Some(true)
            } else if !modules.is_empty() || !custom.is_empty() {
                Some(false)
            } else {
                None
            }
        } else {
            let id = text(&target, "id");
            match text(&target, "kind") {
                "module" => self
                    .names
                    .iter()
                    .find(|(key, _)| key == id)
                    .map(|_| enabled.iter().any(|key| key == id)),
                "custom" => array(self.config, "customProxyGroups")
                    .iter()
                    .find(|g| text(g, "id") == id)
                    .map(|g| g["enabled"] != false),
                _ => None,
            }
        }
    }
    fn resolve(&self, target: &Value, default: &str) -> String {
        let target = self.normalize(target);
        let name = if let Some(name) = target.as_str() {
            name.to_owned()
        } else {
            let id = text(&target, "id");
            match text(&target, "kind") {
                "module" => self
                    .names
                    .iter()
                    .find(|(key, _)| key == id)
                    .map(|(_, n)| n.clone()),
                "custom" => array(self.config, "customProxyGroups")
                    .iter()
                    .find(|g| text(g, "id") == id && g["enabled"] != false)
                    .map(|g| text(g, "name").to_owned()),
                _ => None,
            }
            .unwrap_or_else(|| default.to_owned())
        };
        if self.available.contains(&name) {
            name
        } else {
            self.fallback.clone()
        }
    }
    fn builtin_active(&self, module: &Value, rule: &Value) -> bool {
        let edit = &self.config["builtinRuleEdits"]
            [format!("module:{}:{}", text(module, "id"), text(rule, "id"))];
        if edit["enabled"] == false {
            return false;
        }
        let source_active = strings(&self.config["enabledGroups"])
            .iter()
            .any(|id| id == text(module, "id"));
        if valid_target(&edit["target"]) {
            self.state(&edit["target"]).unwrap_or(source_active)
        } else {
            source_active
        }
    }
}
#[derive(Clone)]
struct Entry {
    key: String,
    value: String,
    editable: bool,
    active: bool,
}
fn provider(rule: &Value, base: &str) -> Value {
    let id = text(rule, "id");
    let format = rule["format"].as_str().unwrap_or("mrs");
    let extension = if format == "text" { "txt" } else { format };
    json!({"type":"http","behavior":rule["behavior"],"url":rule_url(text(rule,"path"),base),"path":format!("./ruleset/{id}.{extension}"),"interval":86400,"format":format})
}

fn order_entries(entries: &[Entry], order: Vec<String>) -> Vec<String> {
    let all = entries.iter().map(|e| e.key.clone()).collect::<Vec<_>>();
    let editable = entries
        .iter()
        .filter(|e| e.editable)
        .map(|e| e.key.clone())
        .collect::<Vec<_>>();
    let active: HashSet<String> = entries
        .iter()
        .filter(|e| e.active)
        .map(|e| e.key.clone())
        .collect();
    let preset: HashSet<String> = modules()
        .iter()
        .flat_map(|m| {
            array(m, "rules")
                .iter()
                .map(move |r| format!("module:{}:{}", text(m, "id"), text(r, "id")))
        })
        .collect();
    let full = order
        .iter()
        .any(|key| (all.contains(key) || preset.contains(key)) && !editable.contains(key));
    if !full {
        let keys = unique(
            order
                .into_iter()
                .filter(|key| editable.contains(key))
                .chain(editable.iter().cloned()),
        );
        let mut index = 0;
        return all
            .into_iter()
            .map(|key| {
                if editable.contains(&key) {
                    let next = keys[index].clone();
                    index += 1;
                    next
                } else {
                    key
                }
            })
            .filter(|k| active.contains(k))
            .collect();
    }
    let mut next = unique(
        order
            .into_iter()
            .filter(|key| active.contains(key) || preset.contains(key)),
    );
    let mut seen: HashSet<String> = next
        .iter()
        .filter(|key| active.contains(*key))
        .cloned()
        .collect();
    let missing = editable
        .iter()
        .filter(|key| !seen.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        let first = all.iter().position(|k| editable.contains(k)).unwrap_or(0);
        let before = all[..first]
            .iter()
            .rev()
            .find_map(|key| next.iter().position(|k| k == key).map(|i| i + 1));
        let after = all
            .iter()
            .skip(first + missing.len())
            .find_map(|key| next.iter().position(|k| k == key));
        let index = before.or(after).unwrap_or(next.len());
        next.splice(index..index, missing.iter().cloned());
        seen.extend(missing);
    }
    for key in &all {
        if seen.contains(key) {
            continue;
        }
        let rule_id = key
            .strip_prefix("module:")
            .and_then(|s| s.split_once(':'))
            .map(|(_, id)| id);
        let anchor = rule_id.and_then(|id| {
            next.iter().position(|candidate| {
                !active.contains(candidate)
                    && preset.contains(candidate)
                    && candidate
                        .strip_prefix("module:")
                        .and_then(|s| s.split_once(':'))
                        .is_some_and(|(_, other)| other == id)
            })
        });
        if let Some(index) = anchor {
            next.insert(index, key.clone());
        } else {
            next.push(key.clone());
        }
        seen.insert(key.clone());
    }
    next.into_iter()
        .filter(|key| active.contains(key))
        .collect()
}

pub(super) fn generate(
    nodes: &[Value],
    groups: &[Value],
    config: &Value,
) -> (Map<String, Value>, Vec<String>) {
    let targets = Targets::new(nodes, groups, config);
    let enabled = strings(&config["enabledGroups"]);
    let custom_rules = normalized_rules(config);
    let sets = normalized_sets(config);
    let base = text(config, "ruleProviderBaseUrl");
    let base = if base.starts_with("http") {
        base
    } else {
        &default_generation().rule_base_url
    };
    let refs: HashSet<String> = custom_rules
        .iter()
        .filter(|r| targets.state(&r["target"]) != Some(false))
        .flat_map(|r| logic_references(text(r, "type"), text(r, "value")))
        .collect();
    let custom_active: HashSet<String> = sets
        .iter()
        .filter(|r| targets.state(&r["target"]) != Some(false))
        .map(|r| text(r, "id").to_owned())
        .collect();
    let experimental = config["experimentalCnUseCnRuleSet"]
        .as_bool()
        .unwrap_or(true);
    let mut providers = Map::new();
    for module in modules() {
        for rule in array(module, "rules") {
            let id = text(rule, "id");
            if targets.builtin_active(module, rule)
                || (refs.contains(id) && !custom_active.contains(id) && !providers.contains_key(id))
            {
                providers.insert(id.to_owned(), provider(rule, base));
            }
        }
    }
    let cn = json!({"id":"cn","behavior":"domain","path":"geosite/cn.mrs"});
    if ((experimental && enabled.iter().any(|id| id == "cn"))
        || (refs.contains("cn") && !sets.iter().any(|r| r["id"] == "cn")))
        && !providers.contains_key("cn")
    {
        providers.insert("cn".to_owned(), provider(&cn, base));
    }
    for set in &sets {
        let id = text(set, "id");
        if !providers.contains_key(id)
            && (targets.state(&set["target"]) != Some(false) || refs.contains(id))
        {
            providers.insert(id.to_owned(), provider(set, base));
        }
    }
    let editable = custom_rules
        .iter()
        .map(|rule| {
            let typ = text(rule, "type");
            let target = targets.resolve(&rule["target"], "");
            let suffix = if rule["noResolve"] == true && no_resolve(typ) {
                ",no-resolve"
            } else {
                ""
            };
            Entry {
                key: format!("custom-rule:{}", text(rule, "id")),
                value: format!("{typ},{},{target}{suffix}", text(rule, "value")),
                editable: true,
                active: targets.state(&rule["target"]) != Some(false),
            }
        })
        .chain(sets.iter().map(|set| {
            let target = targets.resolve(&set["target"], "");
            let suffix = if set["noResolve"] == true {
                ",no-resolve"
            } else {
                ""
            };
            Entry {
                key: format!("custom-rule-set:{}", text(set, "id")),
                value: format!("RULE-SET,{},{target}{suffix}", text(set, "id")),
                editable: true,
                active: targets.state(&set["target"]) != Some(false),
            }
        }))
        .collect::<Vec<_>>();
    let mut entries = Vec::new();
    let mut emitted = HashSet::new();
    let mut push_module = |module: &Value, rule: &Value, entries: &mut Vec<Entry>| {
        if !targets.builtin_active(module, rule) {
            return;
        }
        let mid = text(module, "id");
        let rid = text(rule, "id");
        let key = format!("module:{mid}:{rid}");
        if !emitted.insert(key.clone()) {
            return;
        }
        let edit = &config["builtinRuleEdits"][&key];
        let default = module_name(module, config);
        let target = targets.resolve(
            if valid_target(&edit["target"]) {
                &edit["target"]
            } else {
                &Value::Null
            },
            &default,
        );
        let nr = if mid == "cn" && rid == "cn-ip" {
            config["cnIpNoResolve"].as_bool().unwrap_or(true)
        } else {
            rule["noResolve"] == true
        };
        let value = format!(
            "RULE-SET,{rid},{target}{}",
            if nr { ",no-resolve" } else { "" }
        );
        entries.push(Entry {
            key,
            value,
            editable: false,
            active: true,
        });
    };
    for id in ORDER.iter().copied().chain(
        modules()
            .iter()
            .map(|m| text(m, "id"))
            .filter(|id| !ORDER.contains(id) && *id != "final"),
    ) {
        if id == "gemini" {
            entries.extend(editable.iter().cloned());
        }
        if id == "apple" {
            if let Some(module) = modules().iter().find(|m| m["id"] == "streaming-west") {
                if let Some(rule) = array(module, "rules")
                    .iter()
                    .find(|r| r["id"] == "apple-tvplus")
                {
                    push_module(module, rule, &mut entries);
                }
            }
        }
        if let Some(module) = modules().iter().find(|m| text(m, "id") == id) {
            for rule in array(module, "rules") {
                push_module(module, rule, &mut entries);
            }
        }
    }
    if experimental && enabled.iter().any(|id| id == "cn") {
        let default = modules()
            .iter()
            .find(|m| m["id"] == "cn")
            .map(|m| module_name(m, config))
            .unwrap_or_default();
        let target = targets.resolve(&json!(default), "");
        entries.push(Entry {
            key: "special:experimental-cn".to_owned(),
            value: format!("RULE-SET,cn,{target}"),
            editable: false,
            active: true,
        });
    }
    let order = order_entries(&entries, strings(&config["ruleOrder"]));
    let mut values = Vec::new();
    for key in &order {
        let Some(entry) = entries.iter().find(|e| e.key == *key) else {
            continue;
        };
        if let Some(rest) = key.strip_prefix("module:") {
            if let Some((module_id, rule_id)) = rest.split_once(':') {
                if !enabled.iter().any(|id| id == module_id) {
                    let custom_key = format!("custom-rule-set:{rule_id}");
                    if order.contains(&custom_key)
                        && entries
                            .iter()
                            .any(|e| e.key == custom_key && e.value == entry.value)
                    {
                        let preset = modules()
                            .iter()
                            .find(|m| text(m, "id") == module_id)
                            .and_then(|m| {
                                array(m, "rules").iter().find(|r| text(r, "id") == rule_id)
                            });
                        let set = sets.iter().find(|s| text(s, "id") == rule_id);
                        if let (Some(preset), Some(set)) = (preset, set) {
                            if preset["behavior"] == set["behavior"]
                                && set["format"].as_str().unwrap_or("mrs") == "mrs"
                                && rule_url(text(set, "path"), base)
                                    == format!("{base}/{}", text(preset, "path"))
                            {
                                continue;
                            }
                        }
                    }
                }
            }
        }
        values.push(entry.value.clone());
    }
    let final_target = if enabled.iter().any(|id| id == "final") {
        let name = modules()
            .iter()
            .find(|m| m["id"] == "final")
            .map(|m| module_name(m, config))
            .unwrap_or_default();
        targets.resolve(&json!(name), "")
    } else {
        targets.fallback.clone()
    };
    values.push(format!("MATCH,{final_target}"));
    (providers, values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::generate_config;
    #[test]
    fn moved_rules_follow_destination_activation() {
        let config = json!({"dnsYaml":"","enabledGroups":["select","cn"],"builtinRuleEdits":{"module:microsoft:onedrive":{"target":{"kind":"module","id":"cn"}}}});
        let value = generate_config(&[], &config).unwrap();
        assert!(array(&value, "rules").contains(&json!("RULE-SET,onedrive,🔒 国内服务")));
        assert!(value["rule-providers"].get("onedrive").is_some());
        let mut disabled = config;
        disabled["enabledGroups"] = json!(["select", "microsoft"]);
        assert!(
            generate_config(&[], &disabled).unwrap()["rule-providers"]
                .get("onedrive")
                .is_none()
        );
    }
    #[test]
    fn nested_logic_retains_provider_without_direct_route() {
        let config = json!({"enabledGroups":["select"],"customRules":[{"id":"logic","type":"AND","value":"((RULE-SET,onedrive),(NETWORK,UDP))","target":"DIRECT"}]});
        let value = generate_config(&[], &config).unwrap();
        assert!(value["rule-providers"].get("onedrive").is_some());
        assert!(
            !array(&value, "rules")
                .iter()
                .filter_map(Value::as_str)
                .any(|r| r.starts_with("RULE-SET,onedrive,"))
        );
        assert!(normalize_rule_value("NOT", "((NETWORK,TCP),(NETWORK,UDP))").is_err());
    }
}
