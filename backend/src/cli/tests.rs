use clap::Parser;

use crate::config::Config;

use super::{
    Cli,
    args::{read_bounded, read_json},
};

#[test]
fn parses_documented_commands_and_rejects_conflicting_input() {
    for args in [
        vec!["subboost", "mcp"],
        vec!["subboost", "convert", "--json", "{\"nodes\":[]}"],
        vec![
            "subboost",
            "subscription",
            "create",
            "--json",
            "{\"name\":\"Demo\"}",
        ],
        vec![
            "subboost",
            "subscription",
            "reset-link",
            "id",
            "--expected-token",
            "token",
        ],
        vec![
            "subboost",
            "template",
            "update",
            "--json",
            "{\"id\":\"id\",\"name\":\"Demo\"}",
        ],
        vec!["subboost", "backup", "export"],
        vec!["subboost", "backup", "import", "--json", "@backup.json"],
        vec!["subboost", "migrate-legacy", "legacy.json"],
    ] {
        assert!(Cli::try_parse_from(args).is_ok());
    }
    assert!(Cli::try_parse_from(["subboost", "convert", "input.txt", "--json", "{}"]).is_err());
    assert!(
        Cli::try_parse_from(["subboost", "convert", "--config", "{}", "--json", "{}"]).is_err()
    );
}

#[tokio::test]
async fn validates_json_object_and_utf8_inputs() {
    let maximum_input_bytes = Config::default().limits.input_bytes;
    assert!(read_json("[]", maximum_input_bytes).await.is_err());
    assert!(read_json("{bad}", maximum_input_bytes).await.is_err());
    assert!(read_json("@", maximum_input_bytes).await.is_err());
    assert_eq!(
        read_json("{\"name\":\"Demo\"}", maximum_input_bytes)
            .await
            .expect("valid JSON")["name"],
        "Demo"
    );
    assert!(
        read_bounded(&[0xff][..], maximum_input_bytes)
            .await
            .is_err()
    );
    assert_eq!(
        read_bounded(&b"hello"[..], maximum_input_bytes)
            .await
            .expect("valid UTF-8"),
        "hello"
    );
    assert!(read_bounded(&b"oversized"[..], 3).await.is_err());
}
