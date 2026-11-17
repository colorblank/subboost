use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::config::Config;
use crate::network::{self, RequestPurpose, SourcePolicy};
use crate::rules;

struct RemoteIndex {
    geosite: Vec<String>,
    geoip: Vec<String>,
    fetched: u64,
    expires: u64,
}

struct Discovery {
    items: Vec<Value>,
    fetched: u64,
    expires: u64,
}

#[derive(Default)]
struct CatalogCache {
    index: Option<Arc<RemoteIndex>>,
    discoveries: HashMap<String, Arc<Discovery>>,
}

/// On-demand remote metadata with per-instance caching and single-flight fetches.
pub(crate) struct RuleCatalog {
    config: Arc<Config>,
    cache: Mutex<CatalogCache>,
    index_gate: Mutex<()>,
    discovery_gate: Mutex<()>,
}

impl RuleCatalog {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            cache: Mutex::default(),
            index_gate: Mutex::new(()),
            discovery_gate: Mutex::new(()),
        }
    }

    async fn fetch(&self, url: &str, purpose: RequestPurpose) -> Result<String> {
        let network = &self.config.network;
        Ok(network::fetch(
            url,
            &network.user_agents[0],
            purpose,
            SourcePolicy::PublicOnly,
            network,
        )
        .await?
        .content)
    }

    async fn remote_index(&self) -> Result<Arc<RemoteIndex>> {
        let _gate = self.index_gate.lock().await;
        let cached = self.cache.lock().await.index.clone();
        if let Some(index) = cached.as_ref().filter(|index| now() < index.expires) {
            return Ok(Arc::clone(index));
        }
        match self.fetch_index().await {
            Ok(index) => {
                let index = Arc::new(index);
                self.cache.lock().await.index = Some(Arc::clone(&index));
                Ok(index)
            }
            Err(error) => cached.ok_or(error),
        }
    }

    async fn fetch_index(&self) -> Result<RemoteIndex> {
        let base = self.config.network.github_api_base.trim_end_matches('/');
        let root: Value = serde_json::from_str(
            &self
                .fetch(&format!("{base}/meta"), RequestPurpose::GitHubMetadata)
                .await?,
        )?;
        let sha = root["tree"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["type"] == "tree" && entry["path"] == "geo")
            .and_then(|entry| entry["sha"].as_str())
            .unwrap_or("meta");
        ensure!(
            !sha.is_empty() && sha.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "Invalid Git tree SHA"
        );
        let tree: Value = serde_json::from_str(
            &self
                .fetch(
                    &format!("{base}/{sha}?recursive=1"),
                    RequestPurpose::GitHubMetadata,
                )
                .await?,
        )?;
        ensure!(
            tree["truncated"] != true && tree["tree"].is_array(),
            "GitHub tree response truncated or invalid"
        );
        let names = |kind: &str| {
            let prefix = format!("{kind}/");
            let root_prefix = format!("geo/{kind}/");
            let mut names: Vec<_> = tree["tree"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| entry["type"] == "blob")
                .filter_map(|entry| {
                    let path = entry["path"].as_str()?;
                    let file = path
                        .strip_prefix(&prefix)
                        .or_else(|| path.strip_prefix(&root_prefix))?;
                    let name = file.strip_suffix(".mrs")?;
                    (!name.is_empty() && !name.contains('/')).then(|| name.to_owned())
                })
                .collect();
            names.sort();
            names.dedup();
            names
        };
        let fetched = now();
        Ok(RemoteIndex {
            geosite: names("geosite"),
            geoip: names("geoip"),
            fetched,
            expires: fetched
                .saturating_add(self.config.network.rule_cache_seconds.saturating_mul(1000)),
        })
    }

    pub async fn search(&self, parameters: &Value) -> Value {
        let keyword = parameters["keyword"].as_str().unwrap_or_default();
        let kind = match parameters["type"].as_str() {
            Some(kind @ ("geosite" | "geoip")) => kind,
            _ => "all",
        };
        let page = positive(parameters, "page", 1).min(self.config.rules.maximum_page);
        let size = positive(parameters, "size", self.config.rules.page_size)
            .min(self.config.rules.maximum_page_size);
        let index = match self.remote_index().await {
            Ok(index) => index,
            Err(_) => {
                return json!({"items":[],"keyword":keyword,"type":kind,"page":page,"size":size,"totalMatched":0,"totalRules":0,"source":"unavailable","error":"规则库暂不可用，请稍后重试","code":"RULE_INDEX_UNAVAILABLE"});
            }
        };
        let query = keyword.trim().to_lowercase();
        let mut matched = Vec::new();
        if !query.is_empty() {
            for (catalog_kind, names) in [("geosite", &index.geosite), ("geoip", &index.geoip)] {
                if kind != "all" && kind != catalog_kind {
                    continue;
                }
                for name in names {
                    let rule =
                        rules::featured(catalog_kind, name, &self.config.generation.rule_base_url);
                    let haystack = format!(
                        "{} {} {}",
                        rule["id"].as_str().unwrap_or_default(),
                        rule["name"].as_str().unwrap_or_default(),
                        rule["nameZh"].as_str().unwrap_or_default()
                    );
                    if haystack.to_lowercase().contains(&query) {
                        matched.push(rule);
                    }
                }
            }
        }
        let start = page.saturating_sub(1).saturating_mul(size) as usize;
        let items: Vec<_> = matched.iter().skip(start).take(size as usize).collect();
        json!({"items":items,"keyword":keyword,"type":kind,"page":page,"size":size,"totalMatched":matched.len(),"totalRules":index.geosite.len()+index.geoip.len(),"fetchedAt":index.fetched,"expiresAt":index.expires,"source":if now() >= index.expires {"stale"} else {"remote"}})
    }

    async fn discover(&self, parents: Vec<(String, String)>) -> Result<Arc<Discovery>> {
        let key = parents
            .iter()
            .map(|(module, rule)| format!("{module}:{rule}"))
            .collect::<Vec<_>>()
            .join("|");
        let _gate = self.discovery_gate.lock().await;
        let previous = self.cache.lock().await.discoveries.get(&key).cloned();
        if let Some(value) = previous.as_ref().filter(|value| now() < value.expires) {
            return Ok(Arc::clone(value));
        }
        match self.fetch_discovery(parents).await {
            Ok(value) => {
                let value = Arc::new(value);
                let mut cache = self.cache.lock().await;
                if cache.discoveries.len() >= self.config.rules.maximum_discoveries {
                    let oldest = cache
                        .discoveries
                        .iter()
                        .min_by_key(|(_, value)| value.fetched)
                        .map(|(key, _)| key.clone());
                    if let Some(oldest) = oldest {
                        cache.discoveries.remove(&oldest);
                    }
                }
                cache.discoveries.insert(key, Arc::clone(&value));
                Ok(value)
            }
            Err(error) => previous.ok_or(error),
        }
    }

    async fn fetch_discovery(&self, parents: Vec<(String, String)>) -> Result<Discovery> {
        let index = self.remote_index().await?;
        let mut sources = Vec::new();
        for (module, parent) in parents {
            for (id, kind) in rules::variants(&parent) {
                if index.geosite.contains(&id) {
                    sources.push((module.clone(), parent.clone(), id, kind.to_owned()));
                }
            }
        }
        let mut identifiers = crate::generator::unique(
            sources.iter().map(|source| source.2.clone()).chain(
                index
                    .geosite
                    .iter()
                    .any(|id| id == "geolocation-cn")
                    .then(|| "geolocation-cn".to_owned()),
            ),
        );
        let mut lists = HashMap::new();
        let mut charged = 0;
        while !identifiers.is_empty() {
            let chunk: Vec<_> = identifiers
                .drain(..identifiers.len().min(self.config.rules.parallel_requests))
                .collect();
            let mut requests = tokio::task::JoinSet::new();
            for id in chunk {
                let config = Arc::clone(&self.config);
                requests.spawn(async move {
                    let encoded: String =
                        url::form_urlencoded::byte_serialize(id.as_bytes()).collect();
                    let url = format!(
                        "{}/geosite/{encoded}.list",
                        config.generation.rule_base_url.trim_end_matches('/')
                    );
                    let response = network::fetch(
                        &url,
                        &config.network.user_agents[0],
                        RequestPurpose::Content,
                        SourcePolicy::PublicOnly,
                        &config.network,
                    )
                    .await?;
                    let lines =
                        rules::normalize_lines(&response.content, config.rules.maximum_lines)?;
                    Ok::<_, anyhow::Error>((id, lines, response.content.len()))
                });
            }
            while let Some(response) = requests.join_next().await {
                let (id, lines, bytes) = response.context("Rule request task failed")??;
                charged += bytes + lines.iter().map(String::len).sum::<usize>();
                ensure!(
                    charged <= self.config.rules.maximum_discovery_bytes,
                    "Rule discovery is too large"
                );
                lists.insert(id, lines);
            }
        }
        let geolocation = lists
            .remove("geolocation-cn")
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<_>>();
        let mut charged = geolocation.iter().map(String::len).sum::<usize>();
        let mut fetched = Vec::with_capacity(sources.len());
        for (module, parent, id, kind) in sources {
            let lines = lists.get(&id).cloned().unwrap_or_default();
            charged += lines.iter().map(String::len).sum::<usize>();
            ensure!(
                charged <= self.config.rules.maximum_discovery_bytes,
                "Rule discovery is too large"
            );
            fetched.push((module, parent, id, kind, lines));
        }
        let fetched_at = now().min(index.fetched);
        Ok(Discovery {
            items: rules::compare_cn_sources(fetched, &geolocation),
            fetched: fetched_at,
            expires: fetched_at
                .saturating_add(self.config.network.rule_cache_seconds.saturating_mul(1000))
                .min(index.expires),
        })
    }

    pub async fn cn_candidates(&self, parameters: &Value) -> Value {
        let parents = rules::parents(parameters);
        let debug = parameters["debug"] == true || parameters["debug"] == "1";
        let discovery = if parents.is_empty() {
            let fetched = now();
            Ok(Arc::new(Discovery {
                items: Vec::new(),
                fetched,
                expires: fetched
                    .saturating_add(self.config.network.rule_cache_seconds.saturating_mul(1000)),
            }))
        } else {
            self.discover(parents).await
        };
        match discovery {
            Ok(discovery) => {
                let items: Vec<_> = discovery
                    .items
                    .iter()
                    .filter(|item| item["actionable"] == true)
                    .collect();
                let mut result = json!({"items":items,"source":if now()>=discovery.expires {"stale"} else {"remote"},"cache":{"fetchedAt":discovery.fetched,"expiresAt":discovery.expires,"ttlMs":self.config.network.rule_cache_seconds*1000}});
                if debug {
                    result["allItems"] = json!(discovery.items);
                }
                result
            }
            Err(_) => {
                json!({"items":[],"source":"unavailable","error":"规则库暂不可用，请稍后重试","code":"RULE_INDEX_UNAVAILABLE"})
            }
        }
    }
}

fn now() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

fn positive(parameters: &Value, name: &str, default: u64) -> u64 {
    parameters[name]
        .as_u64()
        .or_else(|| {
            parameters[name]
                .as_str()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(default)
        .max(1)
}
