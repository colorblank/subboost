use serde_json::{Value, json};

pub(super) struct Tool {
    pub(super) name: &'static str,
    pub(super) description: &'static str,
    pub(super) operation: &'static str,
    pub(super) schema: Value,
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn tool(
    name: &'static str,
    description: &'static str,
    operation: &'static str,
    schema: Value,
) -> Tool {
    Tool {
        name,
        description,
        operation,
        schema,
    }
}

pub(super) fn tools(configuration: &crate::config::Rules) -> Vec<Tool> {
    let text = json!({"type": "string"});
    let identifier = json!({"type": "string", "minLength": 1});
    let config = json!({"type": "object", "description": "Saved subscription configuration; same config object used by the web editor."});
    let nodes = json!({"type": "array", "items": {"type": "object"}, "description": "Mihomo proxy nodes. Endpoint and protocol validation is performed by the service."});
    let strings = json!({"type": "array", "items": {"type": "string"}});
    let id = object(json!({"id": identifier}), &["id"]);
    let empty = object(json!({}), &[]);
    let mut subscription_properties = json!({
        "name": {"type": "string", "minLength": 1},
        "urls": strings,
        "nodes": nodes,
        "config": config,
        "autoUpdateInterval": {"type": ["integer", "null"], "description": "Automatic refresh interval in seconds; null disables it."},
        "subscriptionInfo": {"type": ["object", "null"]},
        "smartNodeMatchingEnabled": {"type": "boolean"}
    });
    let create_subscription = object(subscription_properties.clone(), &["name"]);
    subscription_properties["id"] = identifier.clone();
    let update_subscription = object(subscription_properties, &["id"]);
    vec![
        tool(
            "parse",
            "Parse subscription links, base64, JSON or YAML; returns nodes and diagnostics.",
            "parse",
            object(json!({"content": text}), &["content"]),
        ),
        tool(
            "convert",
            "Convert subscription content or nodes into Mihomo YAML using saved subscription configuration.",
            "convert",
            object(
                json!({"content": text, "nodes": nodes, "config": config}),
                &[],
            ),
        ),
        tool(
            "subscriptions_list",
            "List subscriptions owned by this local CLI/MCP installation, not browser guests.",
            "subscriptions.list",
            empty.clone(),
        ),
        tool(
            "subscriptions_get",
            "Read a local subscription, including its nodes and configuration.",
            "subscriptions.get",
            id.clone(),
        ),
        tool(
            "subscriptions_create",
            "Create a local subscription from source URLs, nodes and configuration.",
            "subscriptions.create",
            create_subscription,
        ),
        tool(
            "subscriptions_update",
            "Update supplied fields of a local subscription.",
            "subscriptions.update",
            update_subscription,
        ),
        tool(
            "subscriptions_delete",
            "Delete a local subscription and revoke its public link.",
            "subscriptions.delete",
            id.clone(),
        ),
        tool(
            "subscriptions_refresh",
            "Fetch configured sources and refresh a local subscription's nodes.",
            "subscriptions.refresh",
            id.clone(),
        ),
        tool(
            "subscriptions_reset_link",
            "Rotate a subscription's public link only when expectedToken matches the current token.",
            "subscriptions.reset-link",
            object(
                json!({"id": identifier, "expectedToken": identifier}),
                &["id", "expectedToken"],
            ),
        ),
        tool(
            "subscriptions_yaml",
            "Generate Mihomo YAML for a local saved subscription.",
            "subscriptions.yaml",
            id.clone(),
        ),
        tool(
            "templates_list",
            "List builtin and local templates, optionally filtered by type or IDs.",
            "templates.list",
            object(json!({"type": text, "ids": strings}), &[]),
        ),
        tool(
            "templates_get",
            "Read a builtin or locally saved template.",
            "templates.get",
            id.clone(),
        ),
        tool(
            "templates_create",
            "Create a local template from a saved subscription configuration.",
            "templates.create",
            object(
                json!({"name": identifier, "description": text, "config": config}),
                &["name", "config"],
            ),
        ),
        tool(
            "templates_update",
            "Update supplied fields of a local template; builtin templates are read-only.",
            "templates.update",
            object(
                json!({"id": identifier, "name": identifier, "description": text, "config": config}),
                &["id"],
            ),
        ),
        tool(
            "templates_delete",
            "Delete a local template; builtin templates are read-only.",
            "templates.delete",
            id,
        ),
        tool(
            "source_import",
            "Fetch and parse a subscription URL using local source-security settings. Optional userinfo URL supplies usage metadata.",
            "source.import",
            object(
                json!({"url": identifier, "userinfoUrl": text, "userinfoUserAgent": text}),
                &["url"],
            ),
        ),
        tool(
            "rules_search",
            "Search the rule catalog by keyword and type.",
            "rules.search",
            object(
                json!({"keyword": text, "type": {"type": "string", "enum": ["all", "geosite", "geoip"]}, "page": {"type": "integer", "minimum": 1, "maximum": configuration.maximum_page}, "size": {"type": "integer", "minimum": 1, "maximum": configuration.maximum_page_size}}),
                &[],
            ),
        ),
        tool(
            "rules_cn_candidates",
            "Find CN rule variants for enabled modules while respecting excluded rule keys.",
            "rules.cn-candidates",
            object(
                json!({"moduleIds": strings, "excludedRuleKeys": strings, "debug": {"type": "boolean"}}),
                &[],
            ),
        ),
        tool(
            "rules_detect",
            "Detect the format and behavior of rule-list content.",
            "rules.detect",
            object(json!({"content": text}), &["content"]),
        ),
        tool(
            "settings_get",
            "Read local subscription-source security settings.",
            "settings.get",
            empty.clone(),
        ),
        tool(
            "settings_update",
            "Change the local source-security policy. Enabling unsafe sources permits private-network subscription URLs.",
            "settings.update",
            object(
                json!({"allowUnsafeSubscriptionSources": {"type": "boolean"}}),
                &["allowUnsafeSubscriptionSources"],
            ),
        ),
        tool(
            "backup_export",
            "Export this local owner's subscriptions and templates. The result contains source credentials and public link tokens; keep it private.",
            "backup.export",
            empty,
        ),
        tool(
            "backup_import",
            "Import a version-1 local backup of subscriptions and templates.",
            "backup.import",
            object(
                json!({"version": {"type": "integer", "enum": [1]}, "subscriptions": {"type": "array", "items": {"type": "object"}}, "templates": {"type": "array", "items": {"type": "object"}}}),
                &["version", "subscriptions", "templates"],
            ),
        ),
    ]
}
