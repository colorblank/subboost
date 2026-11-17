use crate::generator::settings::normalize_advanced as advanced;
use crate::generator::{
    self, array, group_type, modules, routing, strings, template_modules, text,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};

const SCHEMA: &str = "subboost-template-config/v1";
pub fn builtin_templates(defaults: &crate::config::Generation) -> Value {
    Value::Array([
        ("minimal","精简版","基础代理组 + 国内外分流，适合轻度用户"),
        ("standard","标准版","完整代理组 + 常用规则，满足大部分需求"),
        ("full","完整版","全部功能 + 扩展规则集，适合高级用户"),
    ].into_iter().map(|(typ,name,description)| {
        let ids = template_modules(typ);
        let rule_count: usize = modules().iter().filter(|m|ids.iter().any(|id|id==text(m,"id"))).map(|m|array(m,"rules").len()).sum();
        json!({"id":format!("builtin-{typ}"),"name":name,"description":description,"downloads":0,"engagementCount":0,"createdAt":"2026-06-01T00:00:00.000Z","tags":["内置"],"isOfficial":true,"isPublic":true,"proxyGroupCount":ids.len(),"ruleCount":rule_count,"kind":"config","config":default_config(typ,defaults)})
    }).collect())
}
pub fn default_config(typ: &str, defaults: &crate::config::Generation) -> Value {
    json!({"schema":SCHEMA,"template":typ,"enabledProxyGroups":template_modules(typ),"hiddenProxyGroups":[],"customProxyGroups":[],"proxyGroupAdvanced":{},"proxyGroupAdvancedModeEnabled":false,"customRuleSets":[],"builtinRuleEdits":{},"customRules":[],"ruleOrder":[],"cnIpNoResolve":true,"experimentalCnUseCnRuleSet":true,"dialerProxyGroups":[],"proxyGroupNameOverrides":{},"dnsYaml":"","mixedPort":generator::base_defaults()["mixed-port"],"allowLan":generator::base_defaults()["allow-lan"],"testUrl":defaults.test_url,"testInterval":defaults.test_interval_seconds,"ruleProviderBaseUrl":defaults.rule_base_url,"sharedHealthChecks":false})
}
fn required_string<'a>(value: &'a Value, key: &str, empty: bool) -> Result<&'a str> {
    let value = value[key]
        .as_str()
        .with_context(|| format!("{key} 必须是字符串"))?;
    ensure!(empty || !value.trim().is_empty(), "{key} 不能为空");
    Ok(if empty { value } else { value.trim() })
}
fn string_array(value: &Value, key: &str, required: bool) -> Result<Vec<String>> {
    if !required && value.get(key).is_none() {
        return Ok(vec![]);
    }
    let array = value[key]
        .as_array()
        .with_context(|| format!("{key} 必须是数组"))?;
    ensure!(array.iter().all(Value::is_string), "{key} 只能包含字符串");
    Ok(strings(&value[key]))
}
fn check_optional_bool(value: &Value, key: &str) -> Result<()> {
    ensure!(
        value.get(key).is_none_or(Value::is_boolean),
        "{key} 必须是布尔值"
    );
    Ok(())
}
fn check_target(value: &Value, key: &str) -> Result<()> {
    ensure!(
        routing::valid_target(&value[key]),
        "{key} 必须是字符串或有效代理组引用"
    );
    Ok(())
}
fn normalize_target(value: &Value) -> Value {
    if let Some(s) = value.as_str() {
        json!(s.trim())
    } else {
        json!({"kind":text(value,"kind"),"id":text(value,"id")})
    }
}
fn strategy_valid(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| ["consistent-hashing", "round-robin", "sticky-sessions"].contains(&s))
}

/// Return the normalized template, or an actionable validation error.
pub fn validate(value: &Value) -> Result<Value> {
    ensure!(value.is_object(), "模板配置必须是对象");
    ensure!(value["schema"] == SCHEMA, "模板配置 schema 无效");
    for key in [
        "moduleRuleOverrides",
        "moduleRuleExclusions",
        "allRulesOrderEditingEnabled",
        "filteredProxyGroups",
    ] {
        ensure!(value.get(key).is_none(), "模板配置包含已移除字段: {key}");
    }
    ensure!(
        ["minimal", "standard", "full"].contains(&text(value, "template")),
        "模板类型无效"
    );
    let enabled = string_array(value, "enabledProxyGroups", true)?;
    let hidden = string_array(value, "hiddenProxyGroups", false)?;
    ensure!(!enabled.is_empty(), "至少需要一个代理组");
    for key in enabled.iter().chain(hidden.iter()) {
        ensure!(
            modules().iter().any(|m| text(m, "id") == key),
            "enabledProxyGroups 或 hiddenProxyGroups 包含未知代理组"
        );
    }
    ensure!(
        enabled.iter().any(|id| !hidden.contains(id)),
        "至少需要一个可见代理组"
    );
    for key in ["customProxyGroups", "customRules", "dialerProxyGroups"] {
        ensure!(value[key].is_array(), "{key} 必须是数组");
    }
    for key in [
        "sharedHealthChecks",
        "cnIpNoResolve",
        "experimentalCnUseCnRuleSet",
        "proxyGroupAdvancedModeEnabled",
    ] {
        check_optional_bool(value, key)?;
    }
    let dns = required_string(value, "dnsYaml", true)?;
    let port =
        generator::valid_port(&value["mixedPort"]).context("mixedPort 必须在 1 到 65535 之间")?;
    ensure!(value["allowLan"].is_boolean(), "allowLan 必须是布尔值");
    let interval = value["testInterval"]
        .as_u64()
        .filter(|i| *i > 0)
        .context("testInterval 必须是正整数")?;
    for key in ["testUrl", "ruleProviderBaseUrl"] {
        let url = required_string(value, key, false)?;
        ensure!(
            url.to_lowercase().starts_with("http://") || url.to_lowercase().starts_with("https://"),
            "{key} 必须是 http(s) URL"
        );
    }
    let mut custom = Vec::new();
    for (index, group) in array(value, "customProxyGroups").iter().enumerate() {
        ensure!(group.is_object(), "customProxyGroups 只能包含对象");
        ensure!(
            group.get("rules").is_none(),
            "模板配置包含已移除字段: customProxyGroups[{index}].rules"
        );
        let id = required_string(group, "id", false)?;
        let name = required_string(group, "name", false)?;
        let emoji = required_string(group, "emoji", true)?;
        let typ = text(group, "groupType");
        ensure!(group_type(typ), "customProxyGroups.groupType 无效");
        ensure!(
            group.get("strategy").is_none() || strategy_valid(&group["strategy"]),
            "customProxyGroups.strategy 无效"
        );
        for key in ["enabled", "includeInGroupMembers"] {
            check_optional_bool(group, key)?;
        }
        ensure!(
            group.get("memberSource").is_none() || group["memberSource"] == "filtered-nodes",
            "customProxyGroups.memberSource 无效"
        );
        let mut normalized = json!({"id":id,"name":name,"emoji":emoji,"groupType":typ,"advanced":advanced(&group["advanced"])});
        for key in ["enabled", "memberSource", "includeInGroupMembers"] {
            if let Some(v) = group.get(key) {
                normalized[key] = v.clone();
            }
        }
        if !text(group, "description").is_empty() {
            normalized["description"] = json!(text(group, "description"));
        }
        if typ == "load-balance" {
            normalized["strategy"] = json!(generator::strategy(&group["strategy"]));
        }
        custom.push(normalized);
    }
    let mut dialers = Vec::new();
    for group in array(value, "dialerProxyGroups") {
        ensure!(group.is_object(), "dialerProxyGroups 只能包含对象");
        let id = required_string(group, "id", false)?;
        let name = required_string(group, "name", false)?;
        let typ = text(group, "type");
        ensure!(group_type(typ), "dialerProxyGroups.type 无效");
        ensure!(
            group.get("strategy").is_none() || strategy_valid(&group["strategy"]),
            "dialerProxyGroups.strategy 无效"
        );
        check_optional_bool(group, "enabled")?;
        let relay = string_array(group, "relayNodes", true)?;
        let targets = string_array(group, "targetNodes", true)?;
        let mut normalized =
            json!({"id":id,"name":name,"type":typ,"relayNodes":relay,"targetNodes":targets});
        if let Some(enabled) = group.get("enabled") {
            normalized["enabled"] = enabled.clone();
        }
        if group.get("testUrl").is_some() {
            let url = required_string(group, "testUrl", false)?;
            ensure!(
                url.to_lowercase().starts_with("http://")
                    || url.to_lowercase().starts_with("https://"),
                "dialerProxyGroups.testUrl 必须是 http(s) URL"
            );
            normalized["testUrl"] = json!(url);
        }
        if typ == "load-balance" {
            normalized["strategy"] = json!(generator::strategy(&group["strategy"]));
        }
        dialers.push(normalized);
    }
    for rule in array(value, "customRules") {
        ensure!(rule.is_object(), "customRules 只能包含对象");
        ensure!(
            routing::RULE_TYPES.contains(&text(rule, "type")),
            "customRules 包含无效类型"
        );
        required_string(rule, "value", false)?;
        routing::normalize_rule_value(text(rule, "type"), text(rule, "value"))?;
        check_target(rule, "target")?;
        check_optional_bool(rule, "noResolve")?;
    }
    if let Some(sets) = value.get("customRuleSets") {
        ensure!(sets.is_array(), "customRuleSets 必须是数组");
        for rule in array(value, "customRuleSets") {
            ensure!(rule.is_object(), "customRuleSets 只能包含对象");
            required_string(rule, "id", false)?;
            required_string(rule, "name", false)?;
            required_string(rule, "path", false)?;
            check_target(rule, "target")?;
            check_optional_bool(rule, "noResolve")?;
            ensure!(
                routing::valid_ruleset(rule),
                "customRuleSets.format、behavior 或 path 无效"
            );
        }
    }
    let mut edits = Map::new();
    if let Some(raw) = value.get("builtinRuleEdits") {
        let raw = raw.as_object().context("builtinRuleEdits 必须是对象")?;
        for (key, edit) in raw {
            let key = key.trim();
            ensure!(
                modules()
                    .iter()
                    .any(|m| array(m, "rules").iter().any(|r| format!(
                        "module:{}:{}",
                        text(m, "id"),
                        text(r, "id")
                    ) == key)),
                "builtinRuleEdits 包含未知内置规则"
            );
            ensure!(edit.is_object(), "builtinRuleEdits 的值必须是对象");
            let mut normalized = Map::new();
            if edit.get("target").is_some() {
                check_target(edit, "target")?;
                normalized.insert("target".to_owned(), normalize_target(&edit["target"]));
            }
            if edit.get("enabled").is_some() {
                ensure!(
                    edit["enabled"] == false,
                    "builtinRuleEdits.enabled 只能是 false"
                );
                normalized.insert("enabled".to_owned(), json!(false));
            }
            if !normalized.is_empty() {
                edits.insert(key.to_owned(), Value::Object(normalized));
            }
        }
    }
    let mut overrides = Map::new();
    if let Some(raw) = value.get("proxyGroupNameOverrides") {
        for (key, v) in raw
            .as_object()
            .context("proxyGroupNameOverrides 必须是对象")?
        {
            let v = v
                .as_str()
                .context("proxyGroupNameOverrides 的值必须是字符串")?
                .trim();
            if !key.trim().is_empty() && !v.is_empty() {
                overrides.insert(key.trim().to_owned(), json!(v));
            }
        }
    }
    let mut advanced_groups = Map::new();
    if let Some(raw) = value.get("proxyGroupAdvanced") {
        for (key, v) in raw.as_object().context("proxyGroupAdvanced 必须是对象")? {
            ensure!(
                modules().iter().any(|m| text(m, "id") == key.trim()),
                "proxyGroupAdvanced 包含未知代理组"
            );
            advanced_groups.insert(key.trim().to_owned(), advanced(v));
        }
    }
    let order = string_array(value, "ruleOrder", false)?;
    let rules = routing::normalized_rules(value);
    let sets = routing::normalized_sets(value);
    let editable = rules
        .iter()
        .map(|r| format!("custom-rule:{}", text(r, "id")))
        .chain(
            sets.iter()
                .map(|r| format!("custom-rule-set:{}", text(r, "id"))),
        )
        .collect::<Vec<_>>();
    let preset = modules()
        .iter()
        .flat_map(|m| {
            array(m, "rules")
                .iter()
                .map(move |r| format!("module:{}:{}", text(m, "id"), text(r, "id")))
        })
        .collect::<Vec<_>>();
    let full = order
        .iter()
        .any(|key| preset.contains(key) || key == "special:experimental-cn");
    let normalized_order = if order.is_empty() {
        vec![]
    } else if full {
        order
            .into_iter()
            .filter(|key| {
                preset.contains(key)
                    || editable.contains(key)
                    || (key == "special:experimental-cn"
                        && enabled.iter().any(|id| id == "cn")
                        && !hidden.iter().any(|id| id == "cn")
                        && value["experimentalCnUseCnRuleSet"] != false)
            })
            .collect()
    } else {
        let mut order = order;
        order.retain(|key| editable.contains(key));
        order.extend(editable);
        generator::unique(order)
    };
    let advanced_enabled = value["proxyGroupAdvancedModeEnabled"]
        .as_bool()
        .unwrap_or_else(|| !custom.is_empty() || !advanced_groups.is_empty());
    let mut result = json!({"schema":SCHEMA,"template":text(value,"template"),"enabledProxyGroups":enabled,"hiddenProxyGroups":hidden,"customProxyGroups":custom,"proxyGroupAdvanced":advanced_groups,"proxyGroupAdvancedModeEnabled":advanced_enabled,"customRuleSets":sets,"builtinRuleEdits":edits,"customRules":rules,"ruleOrder":normalized_order,"dialerProxyGroups":dialers,"proxyGroupNameOverrides":overrides,"dnsYaml":dns,"mixedPort":port,"allowLan":value["allowLan"],"testUrl":text(value,"testUrl"),"testInterval":interval,"sharedHealthChecks":value["sharedHealthChecks"].as_bool().unwrap_or(false),"ruleProviderBaseUrl":text(value,"ruleProviderBaseUrl")});
    for key in ["cnIpNoResolve", "experimentalCnUseCnRuleSet"] {
        if let Some(v) = value.get(key) {
            result[key] = v.clone();
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_roundtrip_and_removed_fields() {
        let defaults = &crate::config::Config::default().generation;
        for entry in builtin_templates(defaults)
            .as_array()
            .expect("builtin array")
        {
            assert!(validate(&entry["config"]).is_ok());
        }
        let mut config = default_config("standard", defaults);
        config["moduleRuleOverrides"] = json!({});
        assert!(validate(&config).is_err());
    }
    #[test]
    fn hidden_all_and_invalid_provider_formats_fail() {
        let defaults = &crate::config::Config::default().generation;
        let mut config = default_config("minimal", defaults);
        config["hiddenProxyGroups"] = config["enabledProxyGroups"].clone();
        assert!(validate(&config).is_err());
        let mut config = default_config("minimal", defaults);
        config["customRuleSets"] = json!([{"id":"test","name":"test","behavior":"classical","format":"mrs","path":"geosite/test.mrs","target":"DIRECT"}]);
        assert!(validate(&config).is_err());
    }
}
