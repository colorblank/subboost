//! Model Context Protocol (MCP 2024-11-05) 协议服务端实现。
//!
//! 通过标准输入/输出（stdin/stdout）提供基于换行分隔 JSON-RPC 的工具调用接口，
//! 支持大模型直接调用 SubBoost 的订阅解析、转换、存储管理与规则查询功能。

use anyhow::Result;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::service::Service;

#[cfg(test)]
mod tests;
mod tools;

use tools::{Tool, tools};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// 启动基于标准输入输出的 MCP 2024-11-05 服务端。
pub async fn serve(service: Service, owner: &str) -> Result<()> {
    serve_io(
        &service,
        owner,
        BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    )
    .await
}

/// 在给定的异步输入与输出流上运行 JSON-RPC 消息处理循环。
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
