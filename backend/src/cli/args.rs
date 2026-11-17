//! 命令行参数解析与 I/O 辅助函数。

use std::{borrow::Cow, path::Path};

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

/// 通用输入源参数，支持指定文件路径或 `-` 表示标准输入。
#[derive(Args)]
pub(super) struct Input {
    /// UTF-8 input file, or - for stdin (default).
    #[arg(default_value = "-")]
    pub(super) file: std::path::PathBuf,
}

/// JSON 格式参数，支持行内文本、`@FILE` 引用或 `@-` 标准输入。
#[derive(Args)]
pub(super) struct JsonArgs {
    /// Object as inline JSON, @FILE, or @- for stdin.
    #[arg(long, value_name = "JSON")]
    pub(super) json: String,
}

/// 向标准输出写入结果字符串，并确保包含尾部换行符。
pub(super) async fn write_output(output: &str) -> Result<()> {
    let mut stdout = tokio::io::stdout();
    stdout.write_all(output.as_bytes()).await?;
    if !output.ends_with('\n') {
        stdout.write_all(b"\n").await?;
    }
    stdout.flush().await?;
    Ok(())
}

/// 解析可选的 JSON 参数，若未指定则返回空的 JSON 对象 `{}`。
pub(super) async fn optional_json(
    value: Option<String>,
    maximum_input_bytes: usize,
) -> Result<Value> {
    match value {
        Some(value) => read_json(&value, maximum_input_bytes).await,
        None => Ok(json!({})),
    }
}

/// 读取并解析 JSON 参数，支持行内 JSON、`@FILE` 或 `@-` 输入。
pub(super) async fn read_json(value: &str, maximum_input_bytes: usize) -> Result<Value> {
    let text = match value.strip_prefix('@') {
        Some("") => bail!("@FILE requires a file path"),
        Some(path) => Cow::Owned(read_input(Path::new(path), maximum_input_bytes).await?),
        None => {
            ensure!(
                value.len() <= maximum_input_bytes,
                "JSON exceeds the configured input limit"
            );
            Cow::Borrowed(value)
        }
    };
    let value: Value = serde_json::from_str(&text).context("invalid JSON argument")?;
    ensure!(value.is_object(), "JSON argument must be an object");
    Ok(value)
}

pub(super) async fn read_input(path: &Path, maximum_input_bytes: usize) -> Result<String> {
    if path == Path::new("-") {
        read_bounded(tokio::io::stdin(), maximum_input_bytes).await
    } else {
        let file = tokio::fs::File::open(path)
            .await
            .with_context(|| format!("cannot open {}", path.display()))?;
        read_bounded(file, maximum_input_bytes)
            .await
            .with_context(|| format!("cannot read {}", path.display()))
    }
}

pub(super) async fn read_bounded(
    reader: impl AsyncRead + Unpin,
    maximum_input_bytes: usize,
) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(maximum_input_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        bytes.len() <= maximum_input_bytes,
        "Input exceeds the configured size limit"
    );
    String::from_utf8(bytes).context("input must be UTF-8")
}
