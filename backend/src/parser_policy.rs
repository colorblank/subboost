use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParserPolicy {
    pub maximum_input_bytes: usize,
    pub maximum_output_bytes: usize,
    pub maximum_nodes: usize,
    pub maximum_lines: usize,
    pub maximum_errors: usize,
    pub maximum_error_characters: usize,
    pub yaml_maximum_depth: usize,
    pub yaml_maximum_nodes: usize,
    pub yaml_maximum_text_bytes: usize,
    pub yaml_maximum_merge_entries: usize,
    pub regex_maximum_bytes: usize,
    pub regex_backtrack_limit: usize,
    pub rule_maximum_line_characters: usize,
}

impl ParserPolicy {
    fn load() -> Result<Self> {
        let explicit = std::env::var_os("SUBBOOST_PARSER_POLICY");
        let path = explicit
            .as_ref()
            .map_or_else(|| PathBuf::from("config/parser-policy.json"), PathBuf::from);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if explicit.is_none() && error.kind() == std::io::ErrorKind::NotFound => {
                include_str!("../../config/parser-policy.json").to_owned()
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Reading parser policy {}", path.display()));
            }
        };
        let policy: Self = serde_json::from_str(&text).context("Invalid parser policy JSON")?;
        ensure!(
            [
                policy.maximum_input_bytes,
                policy.maximum_output_bytes,
                policy.maximum_nodes,
                policy.maximum_lines,
                policy.maximum_errors,
                policy.yaml_maximum_depth,
                policy.yaml_maximum_nodes,
                policy.yaml_maximum_text_bytes,
                policy.yaml_maximum_merge_entries,
                policy.regex_maximum_bytes,
                policy.regex_backtrack_limit,
                policy.rule_maximum_line_characters
            ]
            .into_iter()
            .all(|limit| limit > 0)
                && policy.maximum_error_characters > 1,
            "Parser policy budgets must be positive"
        );
        Ok(policy)
    }
}

/// A process-wide immutable safety policy, loaded once before parsing any untrusted content.
pub(crate) fn policy() -> Result<&'static ParserPolicy> {
    static POLICY: LazyLock<std::result::Result<ParserPolicy, String>> =
        LazyLock::new(|| ParserPolicy::load().map_err(|error| format!("{error:#}")));
    POLICY.as_ref().map_err(|error| anyhow!("{error}"))
}
