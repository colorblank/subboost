use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::service::Service;

const PROTOCOL_VERSION: &str = "2024-11-05";

struct Tool {
    name: &'static str,
    description: &'static str,
    operation: &'static str,
    schema: Value,
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

fn tools(configuration: &crate::config::Rules) -> Vec<Tool> {
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

pub async fn serve(service: Service, owner: &str) -> Result<()> {
    serve_io(
        &service,
        owner,
        BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    )
    .await
}

async fn serve_io(
    service: &Service,
    owner: &str,
    mut input: impl AsyncBufRead + Unpin,
    mut output: impl AsyncWrite + Unpin,
) -> Result<()> {
    let tools = tools(&service.config.rules);
    let mut initialized = false;
    while let Some(line) = read_line(&mut input, service.config.limits.input_bytes).await? {
        let response = match line {
            InputLine::Oversized => Some(error(
                Value::Null,
                -32700,
                "Input line exceeds the configured size limit",
            )),
            InputLine::Bytes(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Err(_) => Some(error(Value::Null, -32700, "Parse error")),
                Ok(Value::Array(batch)) if !batch.is_empty() => {
                    let mut responses = Vec::new();
                    for request in batch {
                        if let Some(response) =
                            dispatch(service, owner, &tools, &mut initialized, request).await
                        {
                            responses.push(response);
                        }
                    }
                    if responses.is_empty() {
                        None
                    } else {
                        Some(Value::Array(responses))
                    }
                }
                Ok(request) => dispatch(service, owner, &tools, &mut initialized, request).await,
            },
        };
        if let Some(response) = response {
            output.write_all(&serde_json::to_vec(&response)?).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
    }
    Ok(())
}

async fn dispatch(
    service: &Service,
    owner: &str,
    tools: &[Tool],
    initialized: &mut bool,
    request: Value,
) -> Option<Value> {
    let Value::Object(mut request) = request else {
        return Some(error(Value::Null, -32600, "Invalid Request"));
    };
    let id = request.remove("id");
    let method = request.remove("method");
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || !method.as_ref().is_some_and(Value::is_string)
        || id
            .as_ref()
            .is_some_and(|id| !id.is_null() && !id.is_string() && !id.is_number())
    {
        return Some(error(Value::Null, -32600, "Invalid Request"));
    }
    // Notifications never receive responses or trigger request-only tool side effects.
    let id = id?;
    let method = method.as_ref().and_then(Value::as_str).unwrap_or_default();
    let params = request.remove("params").unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Some(error(id, -32602, "Params must be an object"));
    }
    let result = match method {
        "initialize" => {
            if *initialized {
                return Some(error(id, -32600, "Session is already initialized"));
            }
            if !params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
                || !params.get("capabilities").is_some_and(Value::is_object)
                || !params.get("clientInfo").is_some_and(|client| {
                    client.get("name").is_some_and(Value::is_string)
                        && client.get("version").is_some_and(Value::is_string)
                })
            {
                return Some(error(
                    id,
                    -32602,
                    "initialize requires protocolVersion, capabilities and clientInfo {name,version}",
                ));
            }
            *initialized = true;
            json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": {"tools": {"listChanged": false}}, "serverInfo": {"name": "subboost", "version": env!("CARGO_PKG_VERSION")}, "instructions": "Tools use the trusted local SQLite owner, separate from browser guests. Subscription and backup results may contain credentials; do not publish them."})
        }
        "ping" => json!({}),
        "tools/list" | "tools/call" if !*initialized => {
            return Some(error(id, -32002, "Initialize the MCP session first"));
        }
        "tools/list" => {
            if params.get("cursor").is_some_and(|cursor| !cursor.is_null()) {
                return Some(error(
                    id,
                    -32602,
                    "This tool list is not paginated; omit cursor",
                ));
            }
            json!({"tools": tools.iter().map(|tool| json!({"name": tool.name, "description": tool.description, "inputSchema": tool.schema})).collect::<Vec<_>>()})
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, -32602, "tools/call requires a tool name"));
            };
            let Some(tool) = tools.iter().find(|tool| tool.name == name) else {
                return Some(error(id, -32602, "Unknown tool"));
            };
            let mut params = params;
            let arguments = params
                .as_object_mut()
                .and_then(|params| params.remove("arguments"))
                .unwrap_or_else(|| json!({}));
            if !arguments.is_object() {
                return Some(error(id, -32602, "Tool arguments must be an object"));
            }
            match validate(&arguments, &tool.schema, "arguments") {
                Err(message) => tool_result(message, true),
                Ok(()) => match service.execute(owner, tool.operation, arguments).await {
                    Ok(value) => tool_result(
                        value.to_string(),
                        value.get("error").is_some_and(Value::is_string),
                    ),
                    Err(error) => tool_result(format!("{error:#}"), true),
                },
            }
        }
        _ => return Some(error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_result(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn validate(value: &Value, schema: &Value, path: &str) -> std::result::Result<(), String> {
    let matches = |kind: &str| match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    };
    if let Some(kind) = schema.get("type") {
        let valid = if let Some(kind) = kind.as_str() {
            matches(kind)
        } else {
            kind.as_array()
                .is_some_and(|kinds| kinds.iter().filter_map(Value::as_str).any(matches))
        };
        if !valid {
            return Err(format!("{path} must have type {kind}"));
        }
    }
    if let Some(variants) = schema.get("enum").and_then(Value::as_array) {
        if !variants.contains(value) {
            return Err(format!("{path} must be one of {variants:?}"));
        }
    }
    if let Some(text) = value.as_str() {
        if schema
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|min| text.chars().count() < min as usize)
        {
            return Err(format!("{path} must not be empty"));
        }
    }
    if let Some(number) = value.as_i64() {
        if schema
            .get("minimum")
            .and_then(Value::as_i64)
            .is_some_and(|min| number < min)
            || schema
                .get("maximum")
                .and_then(Value::as_i64)
                .is_some_and(|max| number > max)
        {
            return Err(format!("{path} is out of range"));
        }
    } else if value.is_u64()
        && schema
            .get("maximum")
            .and_then(Value::as_u64)
            .is_some_and(|max| value.as_u64().is_some_and(|number| number > max))
    {
        return Err(format!("{path} is out of range"));
    }
    if let Some(fields) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required.iter().filter_map(Value::as_str) {
                if !fields.contains_key(field) {
                    return Err(format!("{path}.{field} is required"));
                }
            }
        }
        for (name, value) in fields {
            if let Some(property) = schema.get("properties").and_then(|fields| fields.get(name)) {
                validate(value, property, &format!("{path}.{name}"))?;
            } else if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                return Err(format!("{path}.{name} is not a supported argument"));
            }
        }
    }
    if let (Some(values), Some(item_schema)) = (value.as_array(), schema.get("items")) {
        for (index, value) in values.iter().enumerate() {
            validate(value, item_schema, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

enum InputLine {
    Bytes(Vec<u8>),
    Oversized,
}

async fn read_line(
    input: &mut (impl AsyncBufRead + Unpin),
    limit: usize,
) -> std::io::Result<Option<InputLine>> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let bytes = input.fill_buf().await?;
        if bytes.is_empty() {
            return Ok(if oversized {
                Some(InputLine::Oversized)
            } else if line.is_empty() {
                None
            } else {
                Some(InputLine::Bytes(line))
            });
        }
        let end = bytes.iter().position(|byte| *byte == b'\n');
        let count = end.unwrap_or(bytes.len());
        if !oversized {
            if count > limit.saturating_sub(line.len()) {
                line.clear();
                oversized = true;
            } else {
                line.extend_from_slice(&bytes[..count]);
            }
        }
        input.consume(count + usize::from(end.is_some()));
        if end.is_some() {
            return Ok(Some(if oversized {
                InputLine::Oversized
            } else {
                InputLine::Bytes(line)
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_validate_required_types_ranges_and_unknown_fields() {
        let tools = tools(&crate::config::Config::default().rules);
        for name in ["subscriptions_create", "templates_create", "source_import"] {
            let tool = tools.iter().find(|tool| tool.name == name).unwrap();
            assert!(validate(&json!({}), &tool.schema, "arguments").is_err());
        }
        let search = &tools
            .iter()
            .find(|tool| tool.name == "rules_search")
            .unwrap()
            .schema;
        assert!(validate(&json!({"size": 51}), search, "arguments").is_err());
        assert!(validate(&json!({"size": u64::MAX}), search, "arguments").is_err());
        assert!(validate(&json!({"keyword": 1}), search, "arguments").is_err());
        assert!(validate(&json!({"owner": "guest"}), search, "arguments").is_err());
        assert!(validate(&json!({"keyword": "cn", "size": 10}), search, "arguments").is_ok());
    }

    #[tokio::test]
    async fn bounds_and_drains_lines_and_handles_final_line_without_newline() {
        let mut input = BufReader::with_capacity(2, &b"12345\nok\nlast"[..]);
        assert!(matches!(
            read_line(&mut input, 4).await.unwrap(),
            Some(InputLine::Oversized)
        ));
        assert!(
            matches!(read_line(&mut input, 4).await.unwrap(), Some(InputLine::Bytes(bytes)) if bytes == b"ok")
        );
        assert!(
            matches!(read_line(&mut input, 4).await.unwrap(), Some(InputLine::Bytes(bytes)) if bytes == b"last")
        );
        assert!(read_line(&mut input, 4).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn stdio_negotiates_and_returns_protocol_and_tool_errors_without_notification_responses()
    {
        let directory = tempfile::tempdir().unwrap();
        let service = Service::open({
            let mut configuration = crate::config::Config::default();
            configuration.application.database =
                (&directory.path().join("test.sqlite3")).to_path_buf();
            configuration.application.app_url = ("http://localhost:3001").to_owned();
            configuration
        })
        .await
        .unwrap();
        let messages = [
            "not json".to_owned(),
            json!({"jsonrpc":"2.0", "id":0, "method":"tools/list"}).to_string(),
            json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-06-18", "capabilities":{}, "clientInfo":{"name":"test", "version":"1"}}}).to_string(),
            json!({"jsonrpc":"2.0", "method":"notifications/initialized"}).to_string(),
            json!({"jsonrpc":"2.0", "id":2, "method":"tools/list"}).to_string(),
            json!({"jsonrpc":"2.0", "id":3, "method":"tools/call", "params":{"name":"subscriptions_get", "arguments":{}}}).to_string(),
            json!({"jsonrpc":"2.0", "id":4, "method":"tools/call", "params":{"name":"subscriptions_get", "arguments":{"id":"missing"}}}).to_string(),
            json!({"jsonrpc":"2.0", "id":5, "method":"unknown"}).to_string(),
            json!({"jsonrpc":"2.0", "id":6, "method":"tools/call", "params":[]}).to_string(),
            json!({"jsonrpc":"2.0", "method":"unknown"}).to_string(),
            json!({"jsonrpc":"2.0", "id":7, "method":"ping"}).to_string(),
            json!({"jsonrpc":"2.0", "id":8, "method":"tools/call", "params":{"name":"subscriptions_create", "arguments":{"name":"Local only", "nodes":[{"name":"Direct","type":"direct"}]}}}).to_string(),
            json!({"jsonrpc":"2.0", "id":9, "method":"tools/call", "params":{"name":"subscriptions_list"}}).to_string(),
            json!({"jsonrpc":"2.0", "method":"tools/call", "params":{"name":"subscriptions_create", "arguments":{"name":"Ignored notification"}}}).to_string(),
        ].join("\n");
        let mut output = Vec::new();
        serve_io(
            &service,
            "local",
            BufReader::new(messages.as_bytes()),
            &mut output,
        )
        .await
        .unwrap();
        let responses: Vec<Value> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(responses.len(), 11);
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert_eq!(responses[1]["error"]["code"], -32002);
        assert_eq!(responses[2]["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(
            responses[3]["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "source_import")
        );
        assert_eq!(responses[4]["result"]["isError"], true);
        assert_eq!(responses[5]["result"]["isError"], true);
        assert_eq!(responses[6]["error"]["code"], -32601);
        assert_eq!(responses[7]["error"]["code"], -32602);
        assert_eq!(responses[8]["result"], json!({}));
        assert_eq!(responses[9]["result"]["isError"], false);
        let list: Value = serde_json::from_str(
            responses[10]["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(list["subscriptions"].as_array().unwrap().len(), 1);
        let guests = service
            .execute("guest-test", "subscriptions.list", json!({}))
            .await
            .unwrap();
        assert!(guests["subscriptions"].as_array().unwrap().is_empty());
        let local = service
            .execute("local", "subscriptions.list", json!({}))
            .await
            .unwrap();
        assert_eq!(local["subscriptions"].as_array().unwrap().len(), 1);
        for invalid in [
            json!({"jsonrpc":"1.0", "id":10, "method":"ping"}),
            json!({"jsonrpc":"2.0", "id":{}, "method":"ping"}),
            json!({"jsonrpc":"2.0", "id":10}),
            json!([]),
        ] {
            let response = dispatch(
                &service,
                "local",
                &tools(&service.config.rules),
                &mut true,
                invalid,
            )
            .await
            .unwrap();
            assert_eq!(response["error"]["code"], -32600);
            assert!(response["id"].is_null());
        }
    }
}
