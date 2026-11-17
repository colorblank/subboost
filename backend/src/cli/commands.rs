use std::{net::SocketAddr, path::PathBuf};

use clap::Subcommand;

use super::args::{Input, JsonArgs};

#[derive(Subcommand)]
pub(super) enum Command {
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

#[derive(Subcommand)]
pub(super) enum SubscriptionCommand {
    List,
    Get {
        id: String,
    },
    /// Provide {name,urls,nodes,config,autoUpdateInterval} JSON.
    Create(JsonArgs),
    /// Provide {id,name,urls,nodes,config,autoUpdateInterval} JSON.
    Update(JsonArgs),
    Delete {
        id: String,
    },
    Refresh {
        id: String,
    },
    ResetLink {
        id: String,
        #[arg(long)]
        expected_token: String,
    },
    /// Output plain Mihomo YAML for this subscription.
    Yaml {
        id: String,
    },
}

#[derive(Subcommand)]
pub(super) enum TemplateCommand {
    /// List templates, optionally filtered by {ids:string[]} JSON.
    List {
        #[arg(long, value_name = "JSON")]
        json: Option<String>,
    },
    Get {
        id: String,
    },
    /// Provide {name,description,config} JSON.
    Create(JsonArgs),
    /// Provide {id,name,description,config} JSON.
    Update(JsonArgs),
    Delete {
        id: String,
    },
}

#[derive(Subcommand)]
pub(super) enum SourceCommand {
    /// Import using {url,policy,nodes,content} JSON.
    Import(JsonArgs),
}

#[derive(Subcommand)]
pub(super) enum RulesCommand {
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
pub(super) enum SettingsCommand {
    Get,
    /// Change settings with {allowUnsafeSubscriptionSources:boolean}.
    Update(JsonArgs),
}

#[derive(Subcommand)]
pub(super) enum BackupCommand {
    /// Export JSON containing credentials and public link tokens; keep it private.
    Export,
    /// Import a version-1 object using --json inline, @FILE, or @-.
    Import(JsonArgs),
}
