//! 解析器安全策略与资源限额子模块。
//!
//! 从 `config/parser-policy.json` 或环境变量 `SUBBOOST_PARSER_POLICY` 加载解析预算，
//! 防范过大输入、YAML 递归膨胀炸弹、超长错误信息以及恶意正则 ReDoS 拒绝服务攻击。

use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;

/// 解析器资源安全限制与预算配置。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParserPolicy {
    /// 允许处理的最大原始输入字节数。
    pub maximum_input_bytes: usize,
    /// 格式化/标量转换允许的最大输出字节数。
    pub maximum_output_bytes: usize,
    /// 单个订阅允许解析的最大节点数。
    pub maximum_nodes: usize,
    /// 文本解析允许扫描的最大行数。
    pub maximum_lines: usize,
    /// 允许收集的最大解析错误条数。
    pub maximum_errors: usize,
    /// 单条错误消息允许的最大字符数。
    pub maximum_error_characters: usize,
    /// YAML 解析的最大嵌套深度。
    pub yaml_maximum_depth: usize,
    /// YAML 解析允许的最大 DOM 节点总数。
    pub yaml_maximum_nodes: usize,
    /// YAML 文本标量累计最大字节数。
    pub yaml_maximum_text_bytes: usize,
    /// YAML 允许处理的最大 Alias 别名合并条目数（防 Billion Laughs 炸弹）。
    pub yaml_maximum_merge_entries: usize,
    /// 编译正则表达式允许的最大内存限制字节数。
    pub regex_maximum_bytes: usize,
    /// 正则表达式回溯上限。
    pub regex_backtrack_limit: usize,
    /// 单行规则的最大字符数上限。
    pub rule_maximum_line_characters: usize,
}

impl ParserPolicy {
    fn load() -> Result<Self> {
        let text = crate::config::load_file_or_embedded(
            "SUBBOOST_PARSER_POLICY",
            "config/parser-policy.json",
            include_str!("../../../config/parser-policy.json"),
        );
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
