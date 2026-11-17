//! 订阅更新与快照比对子系统。
//!
//! 负责定时或手动拉取订阅源、安全校验节点变化、智能保留用户自定义节点重命名，
//! 并在部分源请求失败时保持原有有效节点不丢失。

use crate::{
    network, parser,
    service::{Service, fault, now, required},
};
use anyhow::{Context, Result, ensure};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(crate) mod scheduler;
pub(crate) mod snapshot;
pub(crate) use snapshot::SubscriptionSnapshot;

fn origin(node: &Value) -> &str {
    node["_originName"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| node["name"].as_str())
        .unwrap_or_default()
        .trim()
}
fn source_ids(node: &Value) -> Vec<String> {
    node["_sourceIds"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn content_key(node: &Value, level: usize) -> String {
    fn clean(v: &Value, level: usize, root: bool) -> Value {
        match v {
            Value::Object(o) => {
                let fields: BTreeMap<_, _> = o
                    .iter()
                    .filter(|(k, _)| {
                        !k.starts_with('_')
                            && !(root
                                && (k.as_str() == "name"
                                    || (level >= 1 && k.as_str() == "server")
                                    || (level >= 2 && k.as_str() == "port")
                                    || (level >= 3 && matches!(k.as_str(), "sni" | "servername"))))
                    })
                    .map(|(k, v)| (k.clone(), clean(v, level, false)))
                    .collect();
                serde_json::to_value(fields).unwrap_or(Value::Null)
            }
            Value::Array(a) => a.iter().map(|v| clean(v, level, false)).collect(),
            _ => v.clone(),
        }
    }
    clean(node, level, true).to_string()
}
fn scoped(node: &Value) -> String {
    format!("{}\0{}", origin(node), content_key(node, 0))
}
fn formatted(name: &str, tag: &str, template: &str) -> String {
    if tag.trim().is_empty() {
        return name.trim().to_owned();
    }
    let pattern = if template.trim().is_empty() {
        "[{tag}]{name}"
    } else {
        template.trim()
    };
    let name = pattern
        .replace("{tag}", tag.trim())
        .replace("{name}", name.trim());
    name.trim().to_owned()
}
fn auto_name(node: &Value, source: &Value, last: bool) -> String {
    formatted(
        origin(node),
        source[if last { "lastParsedTag" } else { "tag" }]
            .as_str()
            .unwrap_or_default(),
        source[if last {
            "lastParsedNameTemplate"
        } else {
            "nameTemplate"
        }]
        .as_str()
        .unwrap_or_default(),
    )
}
fn detach(node: &Value, id: &str) -> Option<Value> {
    let mut ids = source_ids(node);
    if !ids.iter().any(|sid| sid == id) {
        return Some(node.clone());
    }
    ids.retain(|sid| sid != id);
    if ids.is_empty() {
        return None;
    }
    let mut node = node.clone();
    node["_sourceIds"] = json!(ids);
    Some(node)
}
fn merge_source(
    stored: Vec<Value>,
    fresh: Vec<Value>,
    source: &Value,
    config: &Value,
) -> (Vec<Value>, HashMap<String, String>) {
    let id = source["id"].as_str().unwrap_or_default();
    let smart = config["smartNodeMatchingEnabled"] != false;
    let treat_new = source["type"] == "url"
        && source["lastParsedContent"].as_str().is_some_and(|s| {
            !s.trim().is_empty()
                && s.trim() != source["content"].as_str().unwrap_or_default().trim()
        });
    let deleted: HashSet<_> = config["deletedNodeNames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .collect();
    let deleted_keys: HashSet<_> = config["deletedNodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            v.get("node").map(|n| {
                format!(
                    "{}\0{}",
                    v["originName"].as_str().unwrap_or_else(|| origin(n)),
                    content_key(n, 0)
                )
            })
        })
        .collect();
    let mut counts = HashMap::new();
    for n in &fresh {
        *counts.entry(origin(n).to_owned()).or_insert(0usize) += 1;
    }
    let mut seen = HashSet::new();
    let fresh: Vec<_> = fresh
        .into_iter()
        .filter(|n| {
            let old = auto_name(n, source, true);
            !deleted_keys.contains(&scoped(n))
                && !(counts.get(origin(n)).copied().unwrap_or(0) <= 1
                    && (deleted.contains(origin(n))
                        || deleted.contains(n["name"].as_str().unwrap_or_default())
                        || deleted.contains(old.as_str())))
                && seen.insert(scoped(n))
        })
        .collect();
    let mut indices: HashMap<String, Vec<usize>> = HashMap::new();
    let mut matches: Vec<HashMap<String, Option<String>>> =
        (0..4).map(|_| HashMap::new()).collect();
    for (i, node) in fresh.iter().enumerate() {
        indices.entry(origin(node).into()).or_default().push(i);
        for (level, map) in matches.iter_mut().enumerate() {
            let key = content_key(node, level);
            map.entry(key)
                .and_modify(|value| {
                    if value.as_deref() != Some(origin(node)) {
                        *value = None;
                    }
                })
                .or_insert_with(|| Some(origin(node).into()));
        }
    }
    let mut consumed = HashSet::new();
    let mut base: Vec<(Value, Option<String>, bool)> = Vec::new();
    let mut had = false;
    for mut node in stored {
        let explicit = node["_originName"]
            .as_str()
            .is_some_and(|v| !v.trim().is_empty());
        let previous_origin = origin(&node).to_owned();
        let name = node["name"].as_str().unwrap_or_default().to_owned();
        let ids = source_ids(&node);
        let inferred = if smart {
            matches
                .iter()
                .enumerate()
                .find_map(|(level, map)| map.get(&content_key(&node, level)).cloned().flatten())
        } else {
            None
        };
        let next_origin = if indices.contains_key(&previous_origin) {
            Some(previous_origin.clone())
        } else if inferred.is_some() {
            inferred
        } else if (smart || !explicit)
            && indices
                .get(&name)
                .is_some_and(|list| list.iter().any(|i| fresh[*i]["type"] == node["type"]))
        {
            Some(name.clone())
        } else if explicit {
            Some(previous_origin.clone())
        } else {
            None
        };
        if ids.iter().any(|sid| sid == id)
            || (ids.is_empty()
                && next_origin
                    .as_ref()
                    .is_some_and(|o| indices.contains_key(o)))
        {
            had = true;
            let index = next_origin
                .as_ref()
                .and_then(|o| indices.get(o))
                .and_then(|list| list.iter().find(|i| !consumed.contains(*i)))
                .copied();
            if let Some(index) = index {
                consumed.insert(index);
                let keep = !treat_new && name != auto_name(&node, source, true);
                let mut merged = fresh[index].clone();
                if let Some(fields) = node.as_object() {
                    for (k, v) in fields {
                        if k.starts_with('_') && !matches!(k.as_str(), "_originName" | "_sourceIds")
                        {
                            merged[k] = v.clone();
                        }
                    }
                }
                merged["name"] = if keep {
                    name.clone().into()
                } else {
                    fresh[index]["name"].clone()
                };
                let mut ids = ids;
                if !ids.iter().any(|sid| sid == id) {
                    ids.push(id.into());
                }
                merged["_sourceIds"] = json!(ids);
                base.push((merged, Some(name), keep));
            } else if let Some(node) = detach(&node, id) {
                base.push((node, Some(name), true));
            }
        } else {
            node["_originName"] = previous_origin.into();
            base.push((node, Some(name), true));
        }
    }
    let mut new = Vec::new();
    for (i, mut node) in fresh.into_iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        if smart {
            if let Some((existing, _, _)) = base
                .iter_mut()
                .find(|(existing, _, _)| scoped(existing) == scoped(&node))
            {
                let mut ids = source_ids(existing);
                if !ids.iter().any(|sid| sid == id) {
                    ids.push(id.into());
                }
                let name = existing["name"].clone();
                if let Some(fields) = existing.as_object() {
                    for (k, v) in fields {
                        if k.starts_with('_') && !matches!(k.as_str(), "_originName" | "_sourceIds")
                        {
                            node[k] = v.clone();
                        }
                    }
                }
                node["name"] = name;
                node["_sourceIds"] = json!(ids);
                *existing = node;
                continue;
            }
        }
        node["_sourceIds"] = json!([id]);
        new.push((node, None, false));
    }
    let index = if had {
        base.iter()
            .rposition(|(v, _, _)| source_ids(v).iter().any(|sid| sid == id))
            .map_or(base.len(), |i| i + 1)
    } else {
        base.len()
    };
    base.splice(index..index, new);
    let mut used = HashSet::new();
    let mut renames = HashMap::new();
    for fixed in [true, false] {
        for (node, old, is_fixed) in &mut base {
            if *is_fixed != fixed {
                continue;
            }
            let name = node["name"].as_str().unwrap_or("未命名节点").to_owned();
            let mut candidate = name.clone();
            let mut n = 2;
            while used.contains(&candidate) {
                candidate = format!("{name} ({n})");
                n += 1;
            }
            used.insert(candidate.clone());
            if let Some(old) = old {
                if *old != candidate {
                    renames.insert(old.clone(), candidate.clone());
                }
            }
            node["name"] = candidate.into();
        }
    }
    (base.into_iter().map(|v| v.0).collect(), renames)
}

fn reconcile(config: &mut Value, nodes: &[Value], renames: &HashMap<String, String>) {
    let names: HashSet<_> = nodes.iter().filter_map(|v| v["name"].as_str()).collect();
    let resolve = |name: &str| -> String {
        let mut current = name.to_owned();
        let mut seen = HashSet::new();
        while let Some(next) = renames.get(&current) {
            if !seen.insert(current.clone()) {
                break;
            }
            current = next.clone();
        }
        current
    };
    if let Some(ports) = config["listenerPorts"].as_object_mut() {
        *ports = ports
            .iter()
            .filter_map(|(name, port)| {
                let name = resolve(name);
                (names.contains(name.as_str())
                    && port.as_u64().is_some_and(|p| (1..=65535).contains(&p)))
                .then(|| (name, port.clone()))
            })
            .collect();
    }
    let relay_groups: HashSet<String> = config["dialerProxyGroups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| g["name"].as_str().map(str::to_owned))
        .collect();
    if let Some(groups) = config["dialerProxyGroups"].as_array_mut() {
        for group in groups {
            for key in ["relayNodes", "targetNodes"] {
                if let Some(list) = group[key].as_array_mut() {
                    let mut seen = HashSet::new();
                    *list = list
                        .iter()
                        .filter_map(|v| {
                            let Some(name) = v.as_str() else {
                                return Some(v.clone());
                            };
                            let name = resolve(name.trim());
                            (seen.insert(name.clone())
                                && (names.contains(name.as_str())
                                    || (key == "relayNodes"
                                        && (name == "DIRECT" || relay_groups.contains(&name)))))
                            .then(|| name.into())
                        })
                        .collect();
                }
            }
        }
    }
    if let Some(groups) = config["proxyGroupAdvanced"].as_object_mut() {
        for (_, group) in groups {
            for key in ["extraMembers", "excludedMembers", "memberOrder"] {
                if let Some(list) = group[key].as_array_mut() {
                    let mut seen = HashSet::new();
                    *list = list
                        .iter()
                        .filter_map(|v| {
                            if v["kind"] != "node" {
                                return Some(v.clone());
                            }
                            let name = resolve(v["name"].as_str().unwrap_or_default().trim());
                            if !names.contains(name.as_str()) || !seen.insert(name.clone()) {
                                return None;
                            }
                            let mut v = v.clone();
                            v["name"] = name.into();
                            Some(v)
                        })
                        .collect();
                }
            }
        }
    }
}

/// 订阅刷新触发模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefreshMode {
    /// 用户通过前端或 API 手动触发。
    Manual,
    /// 由后台定时调度器自动触发。
    Automatic,
}

/// 执行指定订阅的拉取、解析、快照比对与状态更新。
pub(crate) async fn refresh(
    service: &Service,
    owner: &str,
    id: &str,
    mode: RefreshMode,
) -> Result<Value> {
    let (mut row, revision) = service.document("subscription", owner, id).await?;
    let policy = service.source_policy(owner).await?;
    let settings = &service.config;
    let mut sources = row["config"]["sources"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if sources.is_empty() {
        for (index, url) in row["urls"].as_array().into_iter().flatten().enumerate() {
            sources.push(json!({"id":format!("url-{index}"),"type":"url","content":url}));
        }
    }
    let valid: HashSet<_> = sources
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .collect();
    let mut nodes = row["nodes"].as_array().cloned().unwrap_or_default();
    nodes.retain_mut(|node| {
        let ids = source_ids(node);
        let next: Vec<_> = ids
            .iter()
            .filter(|id| valid.contains(*id))
            .cloned()
            .collect();
        if !ids.is_empty() && next.is_empty() {
            return false;
        }
        if !ids.is_empty() {
            node["_sourceIds"] = json!(next);
        }
        if let Some(o) = node.as_object_mut() {
            o.remove("dialer-proxy");
            o.remove("dialer_proxy");
        }
        true
    });
    let mut renames = HashMap::new();
    let mut metadata = json!({});
    let mut stable: HashMap<String, Option<String>> = HashMap::new();
    let mut refreshable = 0;
    let mut refreshed = 0;
    let mut urls = 0;
    let mut statics = 0;
    let mut failed = 0;
    let mut failures = HashMap::new();
    for source in &mut sources {
        let source_id = required(source, "id")?.to_owned();
        if source["type"] == "url" && source["useProxyProviders"] == true {
            nodes = nodes.iter().filter_map(|n| detach(n, &source_id)).collect();
            if let Some(url) = source["userinfoUrl"].as_str() {
                if let Ok(response) = network::fetch(
                    url,
                    source["userinfoUserAgent"]
                        .as_str()
                        .unwrap_or(&settings.network.user_agents[0]),
                    network::RequestPurpose::UserInfo,
                    policy,
                    &settings.network,
                )
                .await
                {
                    let info = crate::info::subscription_info(&json!(response.headers), &[]);
                    merge_info(&mut metadata, &info, &mut stable);
                    source["subscriptionUserInfo"] = info;
                }
            }
            continue;
        }
        refreshable += 1;
        let imported = if source["type"] == "url" {
            let mut request = source.clone();
            request["url"] = source["content"].clone();
            network::import(&request, policy, &settings.network).await
        } else {
            let content = required(source, "content")?.to_owned();
            tokio::task::spawn_blocking(move || {
                parser::parse(&content).map(|parse| json!({"parseResult":parse,"headers":{}}))
            })
            .await?
        };
        let imported = match imported {
            Ok(v)
                if v["parseResult"]["nodes"]
                    .as_array()
                    .is_some_and(|n| !n.is_empty()) =>
            {
                v
            }
            result => {
                failed += 1;
                let error = result
                    .err()
                    .map_or_else(|| "未解析到可用节点".into(), |e| format!("{e:#}"));
                failures.insert(source_id, error);
                continue;
            }
        };
        let parsed = imported["parseResult"]["nodes"]
            .as_array()
            .context("Invalid parse result")?;
        let info = crate::info::subscription_info(&imported["headers"], parsed);
        merge_info(&mut metadata, &info, &mut stable);
        source["subscriptionUserInfo"] = info;
        let prepared = parsed
            .iter()
            .map(|n| {
                let mut n = n.clone();
                let name = origin(&n).to_owned();
                n["_originName"] = name.into();
                n["name"] = auto_name(&n, source, false).into();
                n
            })
            .collect();
        let (merged, map) = merge_source(nodes, prepared, source, &row["config"]);
        for value in renames.values_mut() {
            if let Some(next) = map.get(value) {
                *value = next.clone();
            }
        }
        renames.extend(map);
        nodes = merged;
        refreshed += 1;
        if source["type"] == "url" {
            urls += 1;
        } else {
            statics += 1;
        }
        source["lastParsedContent"] = source["content"].clone();
        source["lastParsedTag"] = source["tag"].clone();
        source["lastParsedNameTemplate"] = source["nameTemplate"].clone();
    }
    if mode == RefreshMode::Automatic {
        automatic_state(
            &mut row,
            &sources,
            &failures,
            (nodes.len() > settings.limits.nodes_per_subscription).then_some(nodes.len()),
            settings,
        );
        if (refreshable > 0 && refreshed == 0)
            || nodes.len() > settings.limits.nodes_per_subscription
        {
            service
                .store
                .update("subscription", owner, &row, revision)
                .await?;
        }
    }
    ensure!(
        refreshable == 0 || refreshed > 0,
        fault(
            502,
            "EXTERNAL_FETCH_FAILED",
            "刷新失败：所有导入源均不可用，已保留旧快照。"
        )
    );
    ensure!(
        nodes.len() <= settings.limits.nodes_per_subscription,
        fault(403, "QUOTA_EXCEEDED", "已超过配置允许的节点数量上限")
    );
    row["config"]["sources"] = json!(sources);
    reconcile(&mut row["config"], &nodes, &renames);
    row["nodes"] = json!(nodes);
    service.generate(&row).await?;
    for (key, value) in stable {
        if let Some(value) = value {
            metadata[&key] = value.into();
        }
    }
    row["subscriptionInfo"] = metadata;
    let timestamp = now();
    row["lastUpdatedAt"] = timestamp.clone().into();
    row["updatedAt"] = timestamp.clone().into();
    row["cacheExpiresAt"] = (Utc::now() + Duration::seconds(settings.refresh.cache_seconds))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        .into();
    if mode == RefreshMode::Manual {
        row["autoUpdateState"] = json!({"externalFailureCount":0,"lastAttemptedAt":timestamp,"lastFailedAt":null,"nodeQuotaFailureCount":0,"lastNodeQuotaExceededAt":null,"lastNodeQuotaActual":null,"lastNodeQuotaLimit":null,"disabledAt":null,"disabledReason":null,"disabledPreviousInterval":null});
    }
    service
        .store
        .update("subscription", owner, &row, revision)
        .await?;
    Ok(
        json!({"success":true,"subscriptionId":id,"nodeCount":nodes.len(),"attemptedUrlFetch":urls>0 || sources.iter().any(|s|s["type"]=="url" && s["useProxyProviders"]!=true),"usedUrlFetch":urls>0,"refreshableSourceCount":refreshable,"refreshedSourceCount":refreshed,"refreshedUrlSourceCount":urls,"refreshedStaticSourceCount":statics,"failedSourceCount":failed,"updatedAt":timestamp}),
    )
}
fn merge_info(target: &mut Value, info: &Value, stable: &mut HashMap<String, Option<String>>) {
    for key in ["upload", "download", "total"] {
        if let Some(value) = info[key].as_f64() {
            target[key] = json!(target[key].as_f64().unwrap_or(0.0) + value);
        }
    }
    if let Some(expire) = info["expire"].as_f64() {
        target["expire"] = json!(
            target["expire"]
                .as_f64()
                .map_or(expire, |old| old.min(expire))
        );
    }
    for key in ["profileWebPageUrl", "planName"] {
        if let Some(value) = info[key].as_str() {
            stable
                .entry(key.into())
                .and_modify(|old| {
                    if old.as_deref() != Some(value) {
                        *old = None;
                    }
                })
                .or_insert_with(|| Some(value.into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refresh_keeps_user_names_deletions_and_failed_source_nodes() {
        let source = json!({"id":"s","type":"url","content":"https://example.com","tag":"new","lastParsedTag":"old"});
        let config = json!({"deletedNodeNames":["removed"]});
        let stored = vec![
            json!({"name":"my-name","_originName":"a","type":"ss","server":"one","port":1,"password":"p","_sourceIds":["s"]}),
            json!({"name":"other","type":"ss","server":"other","port":2,"_sourceIds":["failed"]}),
        ];
        let fresh = vec![
            json!({"name":"[new]a","_originName":"a","type":"ss","server":"two","port":1,"password":"p"}),
            json!({"name":"removed","type":"ss","server":"removed","port":1}),
        ];
        let (next, _) = merge_source(stored, fresh, &source, &config);
        assert_eq!(next.len(), 2);
        assert_eq!(next[0]["name"], "my-name");
        assert_eq!(next[0]["server"], "two");
        assert_eq!(next[1]["name"], "other");
    }
    #[test]
    fn preserves_fixed_name_before_assigning_new_collisions() {
        let source = json!({"id":"s","type":"nodes"});
        let (next, _) = merge_source(
            vec![
                json!({"name":"same","type":"ss","server":"manual","port":443,"cipher":"aes-128-gcm","password":"manual"}),
            ],
            vec![
                json!({"name":"same","_originName":"origin","type":"ss","server":"fresh","port":443,"cipher":"aes-128-gcm","password":"fresh"}),
            ],
            &source,
            &json!({}),
        );
        assert_eq!(next[0]["name"], "same");
        assert_eq!(next[1]["name"], "same (2)");
    }
}

fn automatic_state(
    row: &mut Value,
    sources: &[Value],
    failures: &HashMap<String, String>,
    quota: Option<usize>,
    settings: &crate::config::Config,
) {
    use sha2::{Digest, Sha256};
    let timestamp = now();
    let previous = row["autoUpdateState"].clone();
    let old: Value = previous["failureSourceState"]
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    let mut next = json!({});
    let mut maximum = 0;
    for source in sources {
        let id = source["id"].as_str().unwrap_or_default();
        let Some(error) = failures.get(id) else {
            continue;
        };
        let lower = error.to_lowercase();
        let http = lower
            .split("http ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| {
                s.trim_matches(|c: char| !c.is_ascii_digit())
                    .parse::<u16>()
                    .ok()
            });
        if http.is_some_and(|status| (500..600).contains(&status)) {
            continue;
        }
        let stable = http.is_some_and(|status| (400..500).contains(&status))
            || [
                "timeout",
                "timed out",
                "dns",
                "cert",
                "connection",
                "连接",
                "超时",
                "格式",
                "解析",
                "禁止访问",
                "内网",
                "只支持 http",
                "未导入",
                "yaml",
                "json",
                "parse",
                "syntax",
                "base64",
            ]
            .iter()
            .any(|s| lower.contains(s));
        if !stable {
            continue;
        }
        let digest = Sha256::digest(
            format!(
                "{}\n{}",
                source["type"].as_str().unwrap_or_default().trim(),
                source["content"].as_str().unwrap_or_default().trim()
            )
            .as_bytes(),
        );
        let fingerprint: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        let count = if old[id]["fingerprint"] == fingerprint {
            old[id]["count"].as_u64().unwrap_or(0) + 1
        } else {
            1
        };
        maximum = maximum.max(count);
        next[id] = json!({"count":count,"fingerprint":fingerprint,"reason":"订阅源连续拉取失败","lastFailedAt":timestamp});
    }
    let mut state = json!({"externalFailureCount":maximum,"failureSourceState":if maximum>0 {json!(next.to_string())} else {Value::Null},
        "lastAttemptedAt":timestamp,"lastFailedAt":if maximum>0 {json!(timestamp)} else {Value::Null},
        "nodeQuotaFailureCount":0,"lastNodeQuotaExceededAt":null,"lastNodeQuotaActual":null,"lastNodeQuotaLimit":null,
        "disabledAt":null,"disabledReason":null,"disabledPreviousInterval":null});
    let mut disable = maximum >= settings.refresh.failure_threshold;
    let mut reason = "订阅源连续拉取失败";
    if let Some(actual) = quota {
        let count = previous["nodeQuotaFailureCount"].as_u64().unwrap_or(0) + 1;
        state["nodeQuotaFailureCount"] = count.into();
        state["lastNodeQuotaExceededAt"] = timestamp.clone().into();
        state["lastNodeQuotaActual"] = actual.into();
        state["lastNodeQuotaLimit"] = settings.limits.nodes_per_subscription.into();
        if count >= settings.refresh.failure_threshold {
            disable = true;
            reason = "节点数连续超过配额";
        }
    }
    if disable {
        state["disabledAt"] = timestamp.into();
        state["disabledReason"] = reason.into();
        state["disabledPreviousInterval"] = row["autoUpdateInterval"].clone();
        row["autoUpdateInterval"] = Value::Null;
    }
    row["autoUpdateState"] = state;
}

#[cfg(test)]
mod automatic_tests {
    use super::*;
    #[test]
    fn stable_failures_disable_at_three_but_http_503_does_not() {
        let sources = vec![json!({"id":"s","type":"url","content":"https://example.com/sub"})];
        let failures = HashMap::from([("s".into(), "HTTP 404".into())]);
        let mut row = json!({"autoUpdateInterval":360,"autoUpdateState":{}});
        let settings = crate::config::Config::default();
        for _ in 0..settings.refresh.failure_threshold - 1 {
            automatic_state(&mut row, &sources, &failures, None, &settings);
            assert_eq!(row["autoUpdateInterval"], 360);
        }
        automatic_state(&mut row, &sources, &failures, None, &settings);
        assert!(row["autoUpdateInterval"].is_null());
        let mut row = json!({"autoUpdateInterval":360,"autoUpdateState":{}});
        for _ in 0..4 {
            automatic_state(
                &mut row,
                &sources,
                &HashMap::from([("s".into(), "HTTP 503".into())]),
                None,
                &settings,
            );
        }
        assert_eq!(row["autoUpdateInterval"], 360);
        assert_eq!(row["autoUpdateState"]["externalFailureCount"], 0);
    }
}
