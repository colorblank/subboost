//! 业务编排服务层（Service）。
//!
//! 协调存储持久化、配置规则、网络出网与订阅刷新，提供全功能操作分发接口。

use crate::config::Config;
use crate::{generator, network, parser, rules, store::Store, templates};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::{fmt, sync::Arc};

/// 具备标准 HTTP 状态码与业务错误码的结构化错误。
#[derive(Debug)]
pub struct Fault {
    /// 对应的 HTTP 响应状态码（如 400, 403, 404, 429）。
    pub status: u16,
    /// 业务错误码字符串（如 "BAD_REQUEST", "NOT_FOUND"）。
    pub code: &'static str,
    /// 人类可读的错误提示消息。
    pub message: String,
}
impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Fault {}

/// 快速构造一个包含状态码和业务代码的 `anyhow::Error`。
pub fn fault(status: u16, code: &'static str, message: impl Into<String>) -> anyhow::Error {
    Fault {
        status,
        code,
        message: message.into(),
    }
    .into()
}

/// 从 JSON 对象中提取必填的非空字符串字段。
pub fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| fault(400, "BAD_REQUEST", format!("{key} is required")))
}

/// 返回当前时间的毫秒级 RFC3339 字符串表示。
pub fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 安全随机令牌长度。
pub const TOKEN_LENGTH: usize = 43;

/// 生成 32 字节密码学安全随机令牌，并转换为 URL-safe Base64。
pub fn token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}
fn state() -> Value {
    json!({"externalFailureCount":0,"lastFailedAt":null,"lastAttemptedAt":null,
    "nodeQuotaFailureCount":0,"lastNodeQuotaExceededAt":null,"lastNodeQuotaActual":null,
    "lastNodeQuotaLimit":null,"disabledAt":null,"disabledReason":null,"disabledPreviousInterval":null})
}

/// SubBoost 核心应用服务。
#[derive(Clone)]
pub struct Service {
    pub(crate) store: Store,
    pub(crate) config: Arc<Config>,
    catalog: Arc<crate::rules::RuleCatalog>,
}
impl Service {
    /// 打开底层加密 SQLite 存储并初始化全局业务服务。
    pub async fn open(config: Config) -> Result<Self> {
        config.validate()?;
        let store = Store::open(
            &config.application.database,
            std::time::Duration::from_secs(config.application.sqlite_busy_timeout_seconds),
        )
        .await?;
        let config = Arc::new(config);
        let catalog = Arc::new(crate::rules::RuleCatalog::new(Arc::clone(&config)));
        Ok(Self {
            store,
            config,
            catalog,
        })
    }
    /// 统一操作执行分发入口，供 HTTP API、CLI 和 MCP 服务调用。
    pub async fn execute(&self, owner: &str, operation: &str, args: Value) -> Result<Value> {
        ensure!(args.is_object(), "Arguments must be a JSON object");
        ensure!(!owner.is_empty(), "Owner required");
        match operation {
            "parse" => {
                tokio::task::spawn_blocking(move || parser::parse(required(&args, "content")?))
                    .await?
            }
            "convert" => {
                let settings = Arc::clone(&self.config);
                tokio::task::spawn_blocking(move || {
                    let parsed = if let Some(content) = args["content"].as_str() {
                        parser::parse(content)?
                    } else {
                        json!({"nodes":parser::validate_nodes(&args["nodes"])? ,"errors":[]})
                    };
                    let nodes = parsed["nodes"].as_array().context("Invalid parse result")?;
                    ensure!(
                        nodes.len() <= settings.limits.nodes_per_subscription,
                        "Node count exceeds configured quota"
                    );
                    let mut config = args.get("config").cloned().unwrap_or_else(|| json!({}));
                    apply_generation_defaults(&mut config, &settings.generation)?;
                    let yaml = generator::generate(nodes, &config)?;
                    Ok(json!({"yaml":yaml,"nodes":nodes,"errors":parsed["errors"]}))
                })
                .await?
            }
            "source.import" => {
                network::import(
                    &args,
                    self.source_policy(owner).await?,
                    &self.config.network,
                )
                .await
            }
            "subscriptions.list" => {
                let records = self.store.list("subscription", Some(owner)).await?;
                Ok(
                    json!({"subscriptions":records.into_iter().map(|(row,_)|self.summary(row)).collect::<Vec<_>>()}),
                )
            }
            "subscriptions.get" => {
                let (row, _) = self
                    .document("subscription", owner, required(&args, "id")?)
                    .await?;
                Ok(json!({"subscription":self.detail(row)}))
            }
            "subscriptions.create" => {
                let row = self.create_subscription(owner, &args).await?;
                Ok(json!({"subscription":self.summary(row)}))
            }
            "subscriptions.update" => {
                let (old, revision) = self
                    .document("subscription", owner, required(&args, "id")?)
                    .await?;
                let row = self.subscription_input(&args, Some(old))?;
                self.store
                    .update("subscription", owner, &row, revision)
                    .await?;
                Ok(json!({"subscription":self.summary(row)}))
            }
            "subscriptions.delete" => {
                let deleted = self
                    .store
                    .delete("subscription", owner, required(&args, "id")?)
                    .await?;
                if !deleted {
                    return Err(fault(404, "NOT_FOUND", "Subscription not found"));
                }
                Ok(json!({"success":true}))
            }
            "subscriptions.reset-link" => {
                let (mut row, revision) = self
                    .document("subscription", owner, required(&args, "id")?)
                    .await?;
                ensure!(
                    required(&args, "expectedToken")? == row["token"].as_str().unwrap_or_default(),
                    fault(409, "CONFLICT", "订阅链接已更改，请刷新页面后重试")
                );
                row["token"] = token().into();
                row["updatedAt"] = now().into();
                self.store
                    .update("subscription", owner, &row, revision)
                    .await?;
                Ok(
                    json!({"subscription":{"id":row["id"],"token":row["token"],"subscriptionUrl":self.yaml_url(row["token"].as_str().unwrap_or_default())}}),
                )
            }
            "subscriptions.refresh" => self.refresh(owner, required(&args, "id")?).await,
            "subscriptions.yaml" => {
                let (row, _) = self
                    .document("subscription", owner, required(&args, "id")?)
                    .await?;
                Ok(json!({"yaml":self.generate(&row).await?}))
            }
            "templates.list" => {
                let kind = args["type"].as_str().unwrap_or("default");
                let mut items = match kind {
                    "default" => templates::builtin_templates(&self.config.generation)
                        .as_array()
                        .cloned()
                        .context("Invalid builtin templates")?,
                    "my" => self
                        .store
                        .list("template", Some(owner))
                        .await?
                        .into_iter()
                        .map(|(v, _)| self.template_summary(v))
                        .collect(),
                    _ => return Err(fault(400, "VALIDATION_ERROR", "Invalid template type")),
                };
                if let Some(ids) = args["ids"].as_array() {
                    if !ids.is_empty() {
                        items.retain(|v| ids.contains(&v["id"]));
                    }
                }
                let count = items.len();
                Ok(
                    json!({"templates":items,"pagination":{"page":1,"limit":count,"total":count,"totalPages":1}}),
                )
            }
            "templates.get" => {
                let id = required(&args, "id")?;
                if let Some(row) = templates::builtin_templates(&self.config.generation)
                    .as_array()
                    .and_then(|a| a.iter().find(|v| v["id"] == id))
                    .cloned()
                {
                    return Ok(json!({"template":row}));
                }
                let (row, _) = self.document("template", owner, id).await?;
                Ok(json!({"template":self.template_detail(row)}))
            }
            "templates.create" => {
                let mut row = self.template_input(&args, None)?;
                row["id"] = uuid::Uuid::new_v4().to_string().into();
                self.store
                    .insert("template", owner, &row, self.config.document_quota(owner))
                    .await?;
                Ok(json!({"template":self.template_summary(row)}))
            }
            "templates.update" => {
                let (old, revision) = self
                    .document("template", owner, required(&args, "id")?)
                    .await?;
                let row = self.template_input(&args, Some(old))?;
                self.store.update("template", owner, &row, revision).await?;
                Ok(json!({"template":self.template_summary(row)}))
            }
            "templates.delete" => {
                ensure!(
                    self.store
                        .delete("template", owner, required(&args, "id")?)
                        .await?,
                    fault(404, "NOT_FOUND", "Template not found")
                );
                Ok(json!({"success":true}))
            }
            "rules.search" => Ok(self.catalog.search(&args).await),
            "rules.cn-candidates" => Ok(self.catalog.cn_candidates(&args).await),
            "rules.detect" => {
                let content = if let Some(url) = args["url"].as_str() {
                    network::fetch(
                        url,
                        &self.config.network.user_agents[0],
                        network::RequestPurpose::Content,
                        self.source_policy(owner).await?,
                        &self.config.network,
                    )
                    .await?
                    .content
                } else {
                    required(&args, "content")?.to_owned()
                };
                let maximum_entries = self.config.rules.maximum_lines;
                tokio::task::spawn_blocking(move || {
                    rules::detect(&content, maximum_entries)
                        .map_err(|error| fault(400, "UNSUPPORTED_RULE_CONTENT", error.to_string()))
                })
                .await?
            }
            "settings.get" => {
                Ok(json!({"allowUnsafeSubscriptionSources":self.allow_private(owner).await?}))
            }
            "settings.update" => {
                ensure!(
                    !owner.starts_with("guest:"),
                    fault(403, "FORBIDDEN", "游客不能开启内网订阅访问")
                );
                let value = args["allowUnsafeSubscriptionSources"]
                    .as_bool()
                    .ok_or_else(|| {
                        fault(
                            400,
                            "BAD_REQUEST",
                            "allowUnsafeSubscriptionSources must be boolean",
                        )
                    })?;
                let row = json!({"id":owner,"allowUnsafeSubscriptionSources":value});
                match self.store.get("settings", owner, owner).await? {
                    Some((_, revision)) => {
                        self.store.update("settings", owner, &row, revision).await?
                    }
                    None => self.store.insert("settings", owner, &row, 1).await?,
                }
                Ok(json!({"allowUnsafeSubscriptionSources":value}))
            }
            "backup.export" => {
                let subscriptions = self
                    .store
                    .list("subscription", Some(owner))
                    .await?
                    .into_iter()
                    .map(|(v, _)| v)
                    .collect::<Vec<_>>();
                let templates = self
                    .store
                    .list("template", Some(owner))
                    .await?
                    .into_iter()
                    .map(|(v, _)| v)
                    .collect::<Vec<_>>();
                Ok(json!({"version":1,"subscriptions":subscriptions,"templates":templates}))
            }
            "backup.import" => self.import_backup(owner, &args).await,
            _ => Err(fault(
                404,
                "NOT_FOUND",
                format!("Unknown operation: {operation}"),
            )),
        }
    }

    pub(crate) async fn document(&self, kind: &str, owner: &str, id: &str) -> Result<(Value, i64)> {
        ensure!(
            id.len() <= self.config.limits.identifier_bytes,
            fault(400, "BAD_REQUEST", "ID too long")
        );
        self.store
            .get(kind, owner, id)
            .await?
            .ok_or_else(|| fault(404, "NOT_FOUND", format!("{kind} not found")))
    }
    pub(crate) async fn allow_private(&self, owner: &str) -> Result<bool> {
        if owner.starts_with("guest:") {
            return Ok(false);
        }
        Ok(self
            .store
            .get("settings", owner, owner)
            .await?
            .is_some_and(|(v, _)| v["allowUnsafeSubscriptionSources"] == true))
    }
    pub(crate) async fn source_policy(&self, owner: &str) -> Result<network::SourcePolicy> {
        Ok(if self.allow_private(owner).await? {
            network::SourcePolicy::AllowPrivate
        } else {
            network::SourcePolicy::PublicOnly
        })
    }
    pub(crate) fn yaml_url(&self, token: &str) -> String {
        format!(
            "{}/api/subscriptions/{token}/config.yaml",
            self.config.application.app_url.trim_end_matches('/')
        )
    }
    fn detail(&self, mut row: Value) -> Value {
        let url = self.yaml_url(row["token"].as_str().unwrap_or_default());
        row["subscriptionUrl"] = url.clone().into();
        row["yamlUrl"] = url.into();
        row["nodeCount"] = row["nodes"].as_array().map_or(0, Vec::len).into();
        row["sourceCount"] = row["config"]["sources"]
            .as_array()
            .map_or(0, Vec::len)
            .into();
        row["smartNodeMatchingEnabled"] =
            (row["config"]["smartNodeMatchingEnabled"] != false).into();
        row.as_object_mut().map(|o| o.remove("owner"));
        row
    }
    fn summary(&self, row: Value) -> Value {
        let mut row = self.detail(row);
        if let Some(object) = row.as_object_mut() {
            for key in ["urls", "nodes", "config", "subscriptionInfo"] {
                object.remove(key);
            }
        }
        row
    }
    fn template_detail(&self, mut row: Value) -> Value {
        row["kind"] = "config".into();
        if !row["description"].is_string() {
            row["description"] = "".into();
        }
        row["downloads"] = row["downloads"]
            .as_u64()
            .or(row["usageCount"].as_u64())
            .unwrap_or(0)
            .into();
        row["engagementCount"] = row["engagementCount"]
            .as_u64()
            .or(row["favoriteCount"].as_u64())
            .unwrap_or(0)
            .into();
        if !row["tags"].is_array() {
            row["tags"] = json!([]);
        }
        row
    }
    fn template_summary(&self, row: Value) -> Value {
        let mut row = self.template_detail(row);
        row.as_object_mut().map(|o| o.remove("config"));
        row
    }
    fn template_input(&self, args: &Value, old: Option<Value>) -> Result<Value> {
        let mut row = old.unwrap_or_else(||json!({"createdAt":now(),"isBuiltin":false,"isPublic":false,"isDefault":false,"isOwner":true,"ownerName":"本地","usageCount":0,"favoriteCount":0,"isFavorited":false}));
        if args.get("name").is_some() {
            row["name"] = required(args, "name")?.into();
        }
        required(&row, "name")?;
        ensure!(
            row["name"]
                .as_str()
                .is_some_and(|s| s.len() <= self.config.limits.name_bytes),
            fault(400, "BAD_REQUEST", "Template name too long")
        );
        if let Some(description) = args.get("description") {
            ensure!(
                description.is_string() || description.is_null(),
                "description must be a string"
            );
            row["description"] = description.clone();
        }
        if let Some(config) = args.get("config") {
            row["config"] = templates::validate(config)?;
        }
        ensure!(
            row["config"].is_object(),
            fault(400, "BAD_REQUEST", "Template config required")
        );
        row["updatedAt"] = now().into();
        Ok(row)
    }
    fn subscription_input(&self, args: &Value, old: Option<Value>) -> Result<Value> {
        let mut row = old.unwrap_or_else(||json!({"id":uuid::Uuid::new_v4().to_string(),"token":token(),"name":"","urls":[],"nodes":[],"config":{},"subscriptionInfo":{},"autoUpdateInterval":null,"isPrimary":false,"cacheExpiresAt":null,"lastAccessedAt":null,"lastUpdatedAt":null,"createdAt":now(),"autoUpdateState":state()}));
        if let Some(name) = args["name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            row["name"] = name.into();
        }
        required(&row, "name")?;
        ensure!(
            row["name"]
                .as_str()
                .is_some_and(|s| s.len() <= self.config.limits.name_bytes),
            fault(400, "BAD_REQUEST", "Subscription name too long")
        );
        if let Some(urls) = args.get("urls") {
            let urls = urls.as_array().context("urls must be an array")?;
            ensure!(
                urls.len() <= self.config.limits.import_sources,
                "Too many subscription URLs"
            );
            row["urls"] = urls
                .iter()
                .map(|value| {
                    let url = value.as_str().context("URL must be a string")?.trim();
                    ensure!(
                        url.len() <= self.config.network.maximum_url_bytes,
                        "Subscription URL too long"
                    );
                    network::validate_url(url)?;
                    Ok(Value::String(url.into()))
                })
                .collect::<Result<Vec<_>>>()?
                .into();
        }
        if let Some(nodes) = args.get("nodes") {
            let nodes = parser::validate_nodes(nodes)?;
            ensure!(
                nodes.len() <= self.config.limits.nodes_per_subscription,
                "Node count exceeds configured quota"
            );
            row["nodes"] = nodes.into();
        }
        ensure!(
            row["urls"].as_array().is_some_and(|v| !v.is_empty())
                || row["nodes"].as_array().is_some_and(|v| !v.is_empty()),
            fault(400, "BAD_REQUEST", "At least one URL or node is required")
        );
        if let Some(config) = args.get("config") {
            ensure!(config.is_object(), "config must be an object");
            row["config"] = config.clone();
        }
        if let Some(smart) = args.get("smartNodeMatchingEnabled") {
            ensure!(
                smart.is_boolean(),
                "smartNodeMatchingEnabled must be boolean"
            );
            row["config"]["smartNodeMatchingEnabled"] = smart.clone();
        }
        if !row["config"]["smartNodeMatchingEnabled"].is_boolean() {
            row["config"]["smartNodeMatchingEnabled"] = true.into();
        }
        normalize_sources(&mut row["config"], &self.config)?;
        if let Some(value) = args.get("autoUpdateInterval") {
            row["autoUpdateInterval"] =
                if value.is_null() || value == "" || value.as_i64().is_some_and(|n| n <= 0) {
                    Value::Null
                } else {
                    let seconds = value
                        .as_u64()
                        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                        .context("自动更新间隔必须是整数（秒）")?;
                    ensure!(
                        (self.config.refresh.minimum_interval_seconds
                            ..=self.config.refresh.maximum_interval_seconds)
                            .contains(&seconds),
                        "自动更新间隔超出配置允许范围"
                    );
                    seconds.into()
                };
            if !row["autoUpdateInterval"].is_null() {
                row["autoUpdateState"] = state();
            }
        }
        if let Some(info) = args.get("subscriptionInfo") {
            ensure!(
                info.is_null() || info.is_object(),
                "subscriptionInfo must be an object or null"
            );
            row["subscriptionInfo"] = normalize_info(info);
        }
        row["updatedAt"] = now().into();
        Ok(row)
    }
    async fn create_subscription(&self, owner: &str, args: &Value) -> Result<Value> {
        let row = self.subscription_input(args, None)?;
        self.store
            .insert(
                "subscription",
                owner,
                &row,
                self.config.document_quota(owner),
            )
            .await?;
        Ok(row)
    }
    pub(crate) async fn generate(&self, row: &Value) -> Result<String> {
        let nodes = row["nodes"].as_array().cloned().unwrap_or_default();
        let mut config = row["config"].clone();
        apply_generation_defaults(&mut config, &self.config.generation)?;
        tokio::task::spawn_blocking(move || generator::generate(&nodes, &config)).await?
    }
    pub async fn public_yaml(&self, token: &str) -> Result<(String, Value)> {
        let (owner, mut row) = self
            .store
            .by_token(token)
            .await?
            .ok_or_else(|| fault(404, "NOT_FOUND", "Subscription not found"))?;
        let expired = row["cacheExpiresAt"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_none_or(|date| date.with_timezone(&Utc) < Utc::now());
        if expired && row["urls"].as_array().is_some_and(|a| !a.is_empty()) {
            // A failed refresh must not make a previously working subscription disappear.
            if self.refresh(&owner, required(&row, "id")?).await.is_ok() {
                row = self
                    .document("subscription", &owner, required(&row, "id")?)
                    .await?
                    .0;
            }
        }
        let yaml = self.generate(&row).await?;
        let (mut current, revision) = self
            .document("subscription", &owner, required(&row, "id")?)
            .await?;
        current["lastAccessedAt"] = now().into();
        self.store
            .update("subscription", &owner, &current, revision)
            .await?;
        Ok((yaml, row))
    }
    async fn import_backup(&self, owner: &str, args: &Value) -> Result<Value> {
        ensure!(args["version"] == 1, "Unsupported backup version");
        let subscriptions = args["subscriptions"]
            .as_array()
            .context("subscriptions must be an array")?;
        let templates = args["templates"]
            .as_array()
            .context("templates must be an array")?;
        let maximum = self.config.document_quota(owner);
        ensure!(
            subscriptions.len() <= maximum && templates.len() <= maximum,
            "Backup exceeds configured quota"
        );
        // Validate the whole archive before any write; IDs and links are regenerated on import.
        let subscriptions = subscriptions
            .iter()
            .map(|v| self.subscription_input(v, None))
            .collect::<Result<Vec<_>>>()?;
        let templates = templates
            .iter()
            .map(|v| {
                let mut row = self.template_input(v, None)?;
                row["id"] = uuid::Uuid::new_v4().to_string().into();
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?;
        self.store
            .import_atomic(owner, &subscriptions, &templates, maximum)
            .await?;
        Ok(
            json!({"success":true,"subscriptionCount":subscriptions.len(),"templateCount":templates.len()}),
        )
    }
    pub async fn refresh(&self, owner: &str, id: &str) -> Result<Value> {
        crate::refresh::refresh(self, owner, id, crate::refresh::RefreshMode::Manual).await
    }
    pub async fn update_due(&self) -> Result<Value> {
        let mut updated = 0;
        let mut failed = 0;
        let mut skipped = 0;
        for (mut row, revision) in self.store.list("subscription", None).await? {
            let Some(interval) = row["autoUpdateInterval"].as_i64().filter(|n| *n > 0) else {
                continue;
            };
            let previous = row["autoUpdateState"]["lastAttemptedAt"]
                .as_str()
                .or_else(|| row["lastUpdatedAt"].as_str())
                .or_else(|| row["createdAt"].as_str());
            if previous
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_some_and(|date| {
                    date.with_timezone(&Utc) + Duration::seconds(interval) > Utc::now()
                })
            {
                skipped += 1;
                continue;
            }
            let id = required(&row, "id")?.to_owned();
            let owner = self
                .store
                .owner_of(&id)
                .await?
                .context("Subscription owner not found")?;
            row["autoUpdateState"]["lastAttemptedAt"] = now().into();
            if self
                .store
                .update("subscription", &owner, &row, revision)
                .await
                .is_err()
            {
                skipped += 1;
                continue;
            }
            match crate::refresh::refresh(self, &owner, &id, crate::refresh::RefreshMode::Automatic)
                .await
            {
                Ok(_) => updated += 1,
                Err(error) => {
                    failed += 1;
                    eprintln!("Automatic refresh {id}: {error}");
                }
            }
        }
        Ok(json!({"success":true,"updated":updated,"failed":failed,"skipped":skipped}))
    }
}

fn normalize_sources(config: &mut Value, settings: &Config) -> Result<()> {
    let Some(raw) = config.get_mut("sources") else {
        return Ok(());
    };
    let sources = raw.as_array_mut().context("sources must be an array")?;
    ensure!(
        sources.len() <= settings.limits.import_sources,
        "Too many sources"
    );
    let mut output = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for source in sources.iter() {
        ensure!(source.is_object(), "Source must be an object");
        let kind = required(source, "type")?;
        ensure!(
            matches!(kind, "url" | "yaml" | "nodes"),
            "Invalid source type"
        );
        let content = required(source, "content")?;
        let contents: Vec<_> = if kind == "url" {
            content
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            vec![content]
        };
        for (i, content) in contents.into_iter().enumerate() {
            if kind == "url" {
                ensure!(
                    content.len() <= settings.network.maximum_url_bytes,
                    "Subscription URL too long"
                );
                network::validate_url(content)?;
            }
            let mut source = source.clone();
            let id = source["id"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
                .filter(|_| i == 0)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            ensure!(
                id.len() <= settings.limits.identifier_bytes && ids.insert(id.clone()),
                "Duplicate or invalid source ID"
            );
            source["id"] = id.into();
            source["content"] = content.into();
            output.push(source);
        }
    }
    *raw = output.into();
    Ok(())
}
pub(crate) fn normalize_info(value: &Value) -> Value {
    let mut info = json!({});
    for key in ["upload", "download", "total", "expire"] {
        if let Some(n) = value[key]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0 && (key != "expire" || *n > 946684800.0))
        {
            info[key] = json!(n);
        }
    }
    for key in ["profileWebPageUrl", "planName"] {
        if let Some(text) = value[key].as_str() {
            let text: String = text
                .replace(['\r', '\n'], " ")
                .trim()
                .chars()
                .take(if key == "planName" { 200 } else { 1024 })
                .collect();
            if !text.is_empty()
                && (key != "profileWebPageUrl" || network::validate_url(&text).is_ok())
            {
                info[key] = text.into();
            }
        }
    }
    info
}

fn apply_generation_defaults(
    config: &mut Value,
    defaults: &crate::config::Generation,
) -> Result<()> {
    let config = config
        .as_object_mut()
        .context("Configuration must be an object")?;
    config
        .entry("testUrl")
        .or_insert_with(|| json!(defaults.test_url));
    config
        .entry("testInterval")
        .or_insert_with(|| json!(defaults.test_interval_seconds));
    config
        .entry("ruleProviderBaseUrl")
        .or_insert_with(|| json!(defaults.rule_base_url));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TOKEN_LENGTH, token};

    #[test]
    fn token_length_matches_contract() {
        let sample = token();
        assert_eq!(sample.len(), TOKEN_LENGTH);
    }
}
