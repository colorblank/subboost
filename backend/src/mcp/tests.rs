use serde_json::{Value, json};
use tokio::io::BufReader;

use crate::service::Service;

use super::{InputLine, PROTOCOL_VERSION, dispatch, read_line, serve_io, tools::tools, validate};

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
async fn stdio_negotiates_and_returns_protocol_and_tool_errors_without_notification_responses() {
    let directory = tempfile::tempdir().unwrap();
    let service = Service::open({
        let mut configuration = crate::config::Config::default();
        configuration.application.database = directory.path().join("test.sqlite3");
        configuration.application.app_url = "http://localhost:3001".to_owned();
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
