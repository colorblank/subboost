use std::{borrow::Cow, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::{api, config::Config, legacy::LegacyImport, mcp, service::Service};

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

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP backend (no frontend process is started).
    Serve {
        #[arg(long, env = "SUBBOOST_LISTEN")]
        listen: Option<SocketAddr>,
    },
    /// Parse a subscription file or stdin into a JSON parse result.
    Parse(Input),
    /// Convert subscription content or node JSON into Mihomo YAML.
    #[command(
        after_help = "Examples:\n  subboost convert subscription.txt\n  cat subscription.txt | subboost convert --config '@config.json'\n  subboost convert --json '{\"nodes\":[{\"name\":\"Direct\",\"type\":\"direct\"}],\"config\":{}}'\nUse --output-json to retain parse diagnostics with the YAML."
    )]
    Convert {
        /// Input file, or - for stdin. Omit when using --json.
        #[arg(value_name = "FILE", conflicts_with = "json")]
        file: Option<PathBuf>,
        /// Saved subscription configuration: inline JSON object or @FILE.
        #[arg(long, value_name = "JSON", conflicts_with = "json")]
        config: Option<String>,
        /// Conversion object with content or nodes, and optional config.
        #[arg(long, value_name = "JSON")]
        json: Option<String>,
        /// Emit {yaml,nodes,errors} JSON instead of plain YAML.
        #[arg(long)]
        output_json: bool,
    },
    /// Manage subscriptions owned by this local CLI/MCP installation.
    #[command(alias = "subscriptions")]
    Subscription {
        #[command(subcommand)]
        command: SubscriptionCommand,
    },
    /// Manage builtin and locally saved templates.
    #[command(alias = "templates")]
    Template {
        #[command(subcommand)]
        command: TemplateCommand,
    },
    /// Import a remote source using the shared source security policy.
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },
    /// Search rule catalogs and inspect rule-list content.
    Rules {
        #[command(subcommand)]
        command: RulesCommand,
    },
    /// Read or change local source-security settings.
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// Export or import local subscriptions and templates as versioned JSON.
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
    /// Atomically import a PostgreSQL export made with scripts/export-legacy.sql.
    #[command(
        long_about = "Import legacy PostgreSQL data offline into SQLite. FILE must come from scripts/export-legacy.sql. Set ENCRYPTION_KEY to the old installation's master key; it is never printed. Existing IDs, tokens, ownership, password hashes and refresh state are preserved. Any conflict aborts the whole import. Legacy sessions and job leases are intentionally not imported."
    )]
    MigrateLegacy {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Run MCP 2024-11-05 over newline-delimited JSON-RPC on stdin/stdout.
    #[command(
        long_about = "Run a native MCP stdio server using the same database and trusted local owner as CLI commands, never a browser guest. Supports initialize, ping, tools/list and tools/call. Accepts newer clients by negotiating protocol 2024-11-05. Input line limits come from limits.input_bytes in the settings file. Stdout contains only JSON-RPC messages; close stdin to exit."
    )]
    Mcp,
}

#[derive(Args)]
struct Input {
    /// UTF-8 input file, or - for stdin (default).
    #[arg(default_value = "-")]
    file: PathBuf,
}

#[derive(Args)]
struct JsonArgs {
    /// Object as inline JSON, @FILE, or @- for stdin.
    #[arg(long, value_name = "JSON")]
    json: String,
}

#[derive(Subcommand)]
enum SubscriptionCommand {
    List,
    Get {
        id: String,
    },
    /// Create with {name,urls?,nodes?,config?,autoUpdateInterval?,subscriptionInfo?}.
    Create(JsonArgs),
    /// Update with {id,name?,urls?,nodes?,config?,autoUpdateInterval?,subscriptionInfo?}.
    Update(JsonArgs),
    Delete {
        id: String,
    },
    /// Re-fetch configured sources and refresh saved nodes.
    Refresh {
        id: String,
    },
    /// Rotate the public link only if its current token still matches.
    ResetLink {
        id: String,
        #[arg(long)]
        expected_token: String,
    },
    /// Return the saved subscription as plain YAML.
    Yaml {
        id: String,
    },
}

#[derive(Subcommand)]
enum TemplateCommand {
    /// Optional JSON filter: {type?,ids?}.
    List {
        #[arg(long, value_name = "JSON")]
        json: Option<String>,
    },
    Get {
        id: String,
    },
    /// Create with {name,description?,config}.
    Create(JsonArgs),
    /// Update a local template with {id,name?,description?,config?}.
    Update(JsonArgs),
    Delete {
        id: String,
    },
}

#[derive(Subcommand)]
enum SourceCommand {
    /// Import with {url,userinfoUrl?,userinfoUserAgent?}; returns content and parse result.
    Import(JsonArgs),
}

#[derive(Subcommand)]
enum RulesCommand {
    /// Search using optional {keyword,type,page,size} JSON.
    Search {
        #[arg(long, value_name = "JSON")]
        json: Option<String>,
    },
    /// Discover CN variants using optional {moduleIds,excludedRuleKeys,debug} JSON.
    CnCandidates {
        #[arg(long, value_name = "JSON")]
        json: Option<String>,
    },
    /// Detect format and behavior of rule-list content from a file or stdin.
    Detect(Input),
}

#[derive(Subcommand)]
enum SettingsCommand {
    Get,
    /// Change settings with {allowUnsafeSubscriptionSources:boolean}.
    Update(JsonArgs),
}

#[derive(Subcommand)]
enum BackupCommand {
    /// Export JSON containing credentials and public link tokens; keep it private.
    Export,
    /// Import a version-1 object using --json inline, @FILE, or @-.
    Import(JsonArgs),
}

pub async fn run() -> Result<()> {
    run_cli(Cli::parse()).await
}

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

async fn write_output(output: &str) -> Result<()> {
    let mut stdout = tokio::io::stdout();
    stdout.write_all(output.as_bytes()).await?;
    if !output.ends_with('\n') {
        stdout.write_all(b"\n").await?;
    }
    stdout.flush().await?;
    Ok(())
}

async fn optional_json(value: Option<String>, maximum_input_bytes: usize) -> Result<Value> {
    match value {
        Some(value) => read_json(&value, maximum_input_bytes).await,
        None => Ok(json!({})),
    }
}

async fn read_json(value: &str, maximum_input_bytes: usize) -> Result<Value> {
    let text = match value.strip_prefix('@') {
        Some("") => bail!("@FILE requires a file path"),
        Some(path) => {
            Cow::Owned(read_input(std::path::Path::new(path), maximum_input_bytes).await?)
        }
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

async fn read_input(path: &std::path::Path, maximum_input_bytes: usize) -> Result<String> {
    if path == std::path::Path::new("-") {
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

async fn read_bounded(
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
