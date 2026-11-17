//! 命令行工具（CLI）子模块。
//!
//! 提供纯 Rust 实现的直接命令行接口，支持免 HTTP 服务的本地 SQLite 交互、
//! 订阅解析与转换、模版管理、规则检索、数据导入导出以及 MCP 服务启动。

use std::{borrow::Cow, path::PathBuf};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};

use crate::{api, config::Config, legacy::LegacyImport, mcp, service::Service};

mod args;
mod commands;
#[cfg(test)]
mod tests;

use args::{optional_json, read_input, read_json, write_output};
use commands::{
    BackupCommand, Command, RulesCommand, SettingsCommand, SourceCommand, SubscriptionCommand,
    TemplateCommand,
};

/// SubBoost 命令行根参数解析结构。
#[derive(Parser)]
#[command(
    name = "subboost",
    version,
    about = "Local subscription conversion, storage and MCP tools",
    long_about = "Pure Rust subscription tools. Commands use local SQLite directly, without an HTTP server. JSON results go to stdout; errors go to stderr. Browser guests have separate ownership. Runtime policy comes from --settings TOML; JSON arguments accept an inline object, @FILE, or @- for stdin."
)]
struct Cli {
    /// Runtime policy file; defaults are packaged from config/subboost.toml.
    #[arg(long, global = true, env = "SUBBOOST_CONFIG")]
    settings: Option<PathBuf>,
    /// SQLite database shared by CLI, MCP and the HTTP backend.
    #[arg(long, global = true, env = "SUBBOOST_DATABASE")]
    database: Option<PathBuf>,
    /// Public frontend URL used to construct subscription links.
    #[arg(long, global = true, env = "APP_URL")]
    app_url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

/// 解析命令行参数并执行对应的子命令逻辑。
pub async fn run() -> Result<()> {
    run_cli(Cli::parse()).await
}

/// 根据解析出的 CLI 结构配置运行环境并分发执行。
async fn run_cli(cli: Cli) -> Result<()> {
    let mut configuration = match cli.settings {
        Some(path) => Config::load(&path)?,
        None => Config::load_default()?,
    };
    if let Some(database) = cli.database {
        configuration.application.database = database;
    }
    if let Some(app_url) = cli.app_url {
        configuration.application.app_url = app_url;
    }
    configuration.validate()?;
    let maximum_input_bytes = configuration.limits.input_bytes;
    let local_owner = configuration.application.local_owner.clone();
    let (operation, arguments, yaml) = match cli.command {
        Command::Serve { listen } => {
            let listen = listen.unwrap_or(configuration.application.listen);
            return api::serve(Service::open(configuration).await?, listen).await;
        }
        Command::Mcp => return mcp::serve(Service::open(configuration).await?, &local_owner).await,
        Command::MigrateLegacy { file } => {
            let key =
                std::env::var("ENCRYPTION_KEY").context("Legacy ENCRYPTION_KEY is required")?;
            let export = read_input(&file, maximum_input_bytes)
                .await
                .context("Reading legacy export")?;
            let migration = LegacyImport::prepare(&export, &key)?;
            let service = Service::open(configuration).await?;
            let result = migration.import(&service.store).await?;
            return write_output(&serde_json::to_string(&result)?).await;
        }
        Command::Parse(input) => (
            "parse",
            json!({"content": read_input(&input.file, maximum_input_bytes).await?}),
            false,
        ),
        Command::Convert {
            file,
            config,
            json,
            output_json,
        } => {
            let arguments = if let Some(json) = json {
                read_json(&json, maximum_input_bytes).await?
            } else {
                // Read explicit config first so two inputs cannot accidentally consume stdin.
                ensure!(
                    config.as_deref() != Some("@-")
                        || file
                            .as_deref()
                            .is_some_and(|path| path != std::path::Path::new("-")),
                    "stdin cannot supply both configuration and subscription content"
                );
                let config = match config {
                    Some(config) => read_json(&config, maximum_input_bytes).await?,
                    None => json!({}),
                };
                json!({"content": read_input(file.as_deref().unwrap_or(std::path::Path::new("-")), maximum_input_bytes).await?, "config": config})
            };
            ("convert", arguments, !output_json)
        }
        Command::Subscription { command } => match command {
            SubscriptionCommand::List => ("subscriptions.list", json!({}), false),
            SubscriptionCommand::Get { id } => ("subscriptions.get", json!({"id": id}), false),
            SubscriptionCommand::Create(args) => (
                "subscriptions.create",
                read_json(&args.json, maximum_input_bytes).await?,
                false,
            ),
            SubscriptionCommand::Update(args) => (
                "subscriptions.update",
                read_json(&args.json, maximum_input_bytes).await?,
                false,
            ),
            SubscriptionCommand::Delete { id } => {
                ("subscriptions.delete", json!({"id": id}), false)
            }
            SubscriptionCommand::Refresh { id } => {
                ("subscriptions.refresh", json!({"id": id}), false)
            }
            SubscriptionCommand::ResetLink { id, expected_token } => (
                "subscriptions.reset-link",
                json!({"id": id, "expectedToken": expected_token}),
                false,
            ),
            SubscriptionCommand::Yaml { id } => ("subscriptions.yaml", json!({"id": id}), true),
        },
        Command::Template { command } => match command {
            TemplateCommand::List { json } => (
                "templates.list",
                optional_json(json, maximum_input_bytes).await?,
                false,
            ),
            TemplateCommand::Get { id } => ("templates.get", json!({"id": id}), false),
            TemplateCommand::Create(args) => (
                "templates.create",
                read_json(&args.json, maximum_input_bytes).await?,
                false,
            ),
            TemplateCommand::Update(args) => (
                "templates.update",
                read_json(&args.json, maximum_input_bytes).await?,
                false,
            ),
            TemplateCommand::Delete { id } => ("templates.delete", json!({"id": id}), false),
        },
        Command::Source {
            command: SourceCommand::Import(args),
        } => (
            "source.import",
            read_json(&args.json, maximum_input_bytes).await?,
            false,
        ),
        Command::Rules { command } => match command {
            RulesCommand::Search { json } => (
                "rules.search",
                optional_json(json, maximum_input_bytes).await?,
                false,
            ),
            RulesCommand::CnCandidates { json } => (
                "rules.cn-candidates",
                optional_json(json, maximum_input_bytes).await?,
                false,
            ),
            RulesCommand::Detect(input) => (
                "rules.detect",
                json!({"content": read_input(&input.file, maximum_input_bytes).await?}),
                false,
            ),
        },
        Command::Settings { command } => match command {
            SettingsCommand::Get => ("settings.get", json!({}), false),
            SettingsCommand::Update(args) => (
                "settings.update",
                read_json(&args.json, maximum_input_bytes).await?,
                false,
            ),
        },
        Command::Backup { command } => match command {
            BackupCommand::Export => ("backup.export", json!({}), false),
            BackupCommand::Import(args) => (
                "backup.import",
                read_json(&args.json, maximum_input_bytes).await?,
                false,
            ),
        },
    };
    let service = Service::open(configuration).await?;
    let result = service.execute(&local_owner, operation, arguments).await?;
    let output = if yaml {
        Cow::Borrowed(
            result
                .get("yaml")
                .and_then(Value::as_str)
                .context("service returned no YAML")?,
        )
    } else {
        Cow::Owned(serde_json::to_string(&result)?)
    };
    write_output(&output).await
}
