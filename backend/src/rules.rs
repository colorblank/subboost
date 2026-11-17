use crate::generator::{self, array, modules, strings, text};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

const INVALID_CONTENT: &str = "无法识别规则文件，请使用有效的 text 或 YAML 规则集";
static CURATED: LazyLock<Vec<Value>> = LazyLock::new(|| {
    let text = crate::config::load_file_or_embedded(
        "SUBBOOST_RULES_CATALOG",
        "config/rules/catalog.json",
        include_str!("../../config/rules/catalog.json"),
    );
    serde_json::from_str(&text).expect("embedded catalog data")
});
pub(crate) fn featured(kind: &str, name: &str, base: &str) -> Value {
    let path = format!("/{kind}/{name}.mrs");
    let mut rule=CURATED.iter().find(|rule|text(rule,"url").ends_with(&path)).cloned().unwrap_or_else(||json!({"id":if kind=="geoip" {format!("{name}-ip")} else {name.to_owned()},"name":name,"nameZh":name,"category":"other","behavior":if kind=="geoip" {"ipcidr"} else {"domain"},"format":"mrs"}));
    rule["url"] = json!(format!("{}{path}", base.trim_end_matches('/')));
    rule["availabilityStatus"] = json!("available");
    rule
}

fn list_param(params: &Value, primary: &str, alternate: &str) -> Option<Vec<String>> {
    params
        .get(primary)
        .or_else(|| params.get(alternate))
        .map(|value| {
            if let Some(s) = value.as_str() {
                generator::unique(
                    s.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                )
            } else {
                strings(value)
            }
        })
}
pub(crate) fn parents(params: &Value) -> Vec<(String, String)> {
    let requested = list_param(params, "modules", "moduleIds");
    let excluded = list_param(params, "excluded", "excludedRuleKeys").unwrap_or_default();
    let mut parents = vec![];
    for module in modules() {
        let id = text(module, "id");
        if text(module, "category") == "core"
            || requested
                .as_ref()
                .is_some_and(|list| !list.iter().any(|m| m == id))
        {
            continue;
        }
        for rule in array(module, "rules") {
            let rid = text(rule, "id");
            if rule["behavior"] == "domain"
                && text(rule, "path").starts_with("geosite/")
                && !excluded.contains(&format!("{id}:{rid}"))
            {
                parents.push((id.to_owned(), rid.to_owned()));
            }
        }
    }
    parents
}
pub(crate) fn variants(id: &str) -> Vec<(String, &'static str)> {
    let mut bases = vec![id];
    if let Some(base) = id.strip_suffix("-!cn") {
        if !base.is_empty() {
            bases.push(base);
        }
    }
    bases
        .into_iter()
        .flat_map(|base| {
            [
                (format!("{base}-cn"), "dash-cn"),
                (format!("{base}@cn"), "at-cn"),
                (format!("{base}-cn@cn"), "dash-cn-at-cn"),
            ]
        })
        .collect()
}
pub(crate) fn normalize_lines(body: &str, maximum_lines: usize) -> Result<Vec<String>> {
    ensure!(
        body.lines().count() <= maximum_lines,
        "Rule list has too many lines"
    );
    Ok(generator::unique(
        body.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned),
    ))
}
pub(crate) fn compare_cn_sources(
    sources: Vec<(String, String, String, String, Vec<String>)>,
    geolocation: &HashSet<String>,
) -> Vec<Value> {
    let mut items = Vec::new();
    let mut signatures: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
    for (module, parent, id, kind, mut lines) in sources {
        let empty = lines.is_empty();
        let covered = !empty && lines.iter().all(|line| geolocation.contains(line));
        lines.sort();
        let index = items.len();
        if !empty {
            signatures.entry(lines).or_default().push(index);
        }
        items.push(json!({"id":id,"name":id,"behavior":"domain","format":"mrs","path":format!("geosite/{id}.mrs"),"parentRuleId":parent,"parentModuleId":module,"variantKind":kind,"canonicalId":id,"coveredByGeolocationCn":covered,"empty":empty,"actionable":false}));
    }
    let priority = |kind: &str| match kind {
        "dash-cn" => 0,
        "at-cn" => 1,
        _ => 2,
    };
    for group in signatures.values() {
        let canonical = *group
            .iter()
            .min_by_key(|&&i| {
                (
                    priority(text(&items[i], "variantKind")),
                    text(&items[i], "id"),
                )
            })
            .unwrap();
        let id = items[canonical]["id"].clone();
        for &index in group {
            items[index]["canonicalId"] = id.clone();
            if items[index]["id"] != id {
                items[index]["duplicateOf"] = id.clone();
            }
        }
    }
    for item in &mut items {
        item["actionable"] = json!(
            item["empty"] == false
                && item["coveredByGeolocationCn"] == false
                && item.get("duplicateOf").is_none()
        );
    }
    items.sort_by(|a, b| {
        (
            text(a, "parentModuleId"),
            text(a, "parentRuleId"),
            priority(text(a, "variantKind")),
            text(a, "id"),
        )
            .cmp(&(
                text(b, "parentModuleId"),
                text(b, "parentRuleId"),
                priority(text(b, "variantKind")),
                text(b, "id"),
            ))
    });
    items
}

fn ip_version(value: &str) -> Option<u8> {
    value
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| if ip.is_ipv4() { 4 } else { 6 })
}
fn cidr(value: &str, version: Option<u8>) -> bool {
    let Some((address, prefix)) = value.split_once('/') else {
        return false;
    };
    let Some(ip) = ip_version(address) else {
        return false;
    };
    !prefix.is_empty()
        && prefix.len() <= 3
        && prefix.bytes().all(|b| b.is_ascii_digit())
        && prefix
            .parse::<u8>()
            .is_ok_and(|p| p <= if ip == 4 { 32 } else { 128 })
        && version.is_none_or(|v| v == ip)
}
fn domain(value: &str, patterns: bool) -> bool {
    if value.is_empty()
        || value.len() > 253
        || ip_version(value).is_some()
        || value.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return false;
    }
    let domain = if patterns {
        value
            .strip_prefix("+.")
            .or_else(|| value.strip_prefix('.'))
            .unwrap_or(value)
    } else {
        value
    };
    domain.split('.').all(|label| {
        if patterns && label == "*" {
            return true;
        }
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            && label.bytes().next().is_some_and(|b| b != b'-')
            && label.bytes().last().is_some_and(|b| b != b'-')
    })
}
fn port_spec(value: &str) -> bool {
    value.split('/').all(|part| {
        let bounds = part.split('-').collect::<Vec<_>>();
        bounds.len() <= 2
            && bounds.iter().all(|bound| {
                !bound.is_empty()
                    && bound.len() <= 5
                    && bound.bytes().all(|b| b.is_ascii_digit())
                    && bound.parse::<u16>().is_ok()
            })
            && (bounds.len() == 1
                || bounds[0].parse::<u16>().unwrap() <= bounds[1].parse::<u16>().unwrap())
    })
}
fn classical(value: &str) -> bool {
    let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) || parts[1].is_empty() {
        return false;
    }
    let typ = parts[0];
    let pattern = parts[1];
    if parts.len() == 3
        && !(parts[2] == "no-resolve" && ["IP-CIDR", "IP-CIDR6", "GEOIP"].contains(&typ))
    {
        return false;
    }
    match typ {
        "DOMAIN" | "DOMAIN-SUFFIX" => domain(pattern, false),
        "IP-CIDR" => cidr(pattern, Some(4)),
        "IP-CIDR6" => cidr(pattern, Some(6)),
        "GEOIP" | "GEOSITE" => pattern
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_@!-".contains(&b)),
        "DST-PORT" | "SRC-PORT" => port_spec(pattern),
        "DOMAIN-KEYWORD" => !pattern
            .chars()
            .any(|c| c.is_whitespace() || "/:<>[]{}".contains(c)),
        "PROCESS-NAME" => !pattern.contains(['\r', '\n', '\0']),
        _ => false,
    }
}
fn behavior(entries: &[String]) -> Option<&'static str> {
    if entries.iter().all(|v| cidr(v, None)) {
        Some("ipcidr")
    } else if entries.iter().all(|v| domain(v, true)) {
        Some("domain")
    } else if entries.iter().all(|v| classical(v)) {
        Some("classical")
    } else {
        None
    }
}
pub fn detect(content: &str, maximum_entries: usize) -> Result<Value> {
    let policy = crate::parser_policy::policy()?;
    ensure!(
        content.len() <= policy.maximum_input_bytes,
        "规则文件超过解析大小限制"
    );
    ensure!(
        !content
            .chars()
            .any(|c| (c < '\u{20}' && !['\t', '\r', '\n'].contains(&c)) || c == '\u{fffd}'),
        "{INVALID_CONTENT}"
    );
    let content = content
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    ensure!(
        !content.lines().any(|line| line
            .chars()
            .nth(policy.rule_maximum_line_characters)
            .is_some()),
        "规则文件包含过长的行"
    );
    let meaningful = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let (format, entries) = if behavior(&meaningful).is_some() {
        ("text", meaningful)
    } else {
        let mapping = generator::settings::parse_rule_yaml_mapping(&content)
            .map_err(|_| anyhow::anyhow!(INVALID_CONTENT))?;
        ensure!(
            mapping.len() == 1 && mapping.get("payload").is_some_and(Value::is_array),
            "{INVALID_CONTENT}"
        );
        let entries = mapping["payload"].as_array().unwrap();
        ensure!(entries.iter().all(Value::is_string), "{INVALID_CONTENT}");
        (
            "yaml",
            entries
                .iter()
                .map(|v| v.as_str().unwrap().trim().to_owned())
                .collect::<Vec<_>>(),
        )
    };
    ensure!(
        !entries.is_empty()
            && entries.len() <= maximum_entries
            && entries
                .iter()
                .all(|e| !e.is_empty()
                    && e.chars().nth(policy.rule_maximum_line_characters).is_none()),
        "{INVALID_CONTENT}"
    );
    let behavior = behavior(&entries).context(INVALID_CONTENT)?;
    Ok(json!({"format":format,"behavior":behavior}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detection_checks_payload_and_versions() {
        let maximum_entries = crate::config::Config::default().rules.maximum_lines;
        assert_eq!(
            detect("# comment\n+.example.com\n*.example.net", maximum_entries).unwrap(),
            json!({"format":"text","behavior":"domain"})
        );
        assert_eq!(
            detect(
                "payload:\n  - '203.0.113.0/24'\n  - '::1/128'",
                maximum_entries
            )
            .unwrap(),
            json!({"format":"yaml","behavior":"ipcidr"})
        );
        assert!(detect("payload: &loop [*loop]", maximum_entries).is_err());
        assert!(detect("256.2.3.4/32", maximum_entries).is_err());
        assert!(detect("payload:\n  - true", maximum_entries).is_err());
    }
    #[test]
    fn candidates_deduplicate_and_hide_covered_rules() {
        let data = compare_cn_sources(
            vec![
                (
                    "apple".into(),
                    "apple".into(),
                    "apple-cn".into(),
                    "dash-cn".into(),
                    vec!["+.a.com".into()],
                ),
                (
                    "apple".into(),
                    "apple".into(),
                    "apple@cn".into(),
                    "at-cn".into(),
                    vec!["+.a.com".into()],
                ),
            ],
            &HashSet::new(),
        );
        assert_eq!(data[0]["actionable"], true);
        assert_eq!(data[1]["duplicateOf"], "apple-cn");
        assert_eq!(data[1]["actionable"], false);
    }
}
