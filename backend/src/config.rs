use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

/// Deployment policy, kept separate from user-authored Mihomo configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub application: Application,
    pub refresh: Refresh,
    pub network: Network,
    pub limits: Limits,
    pub authentication: Authentication,
    pub rate_limits: RateLimits,
    pub generation: Generation,
    pub rules: Rules,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Application {
    pub database: PathBuf,
    pub app_url: String,
    pub listen: SocketAddr,
    pub local_owner: String,
    pub sqlite_busy_timeout_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refresh {
    pub enabled: bool,
    pub poll_seconds: u64,
    pub cache_seconds: i64,
    pub default_interval_seconds: u64,
    pub step_interval_seconds: u64,
    pub minimum_interval_seconds: u64,
    pub maximum_interval_seconds: u64,
    pub failure_threshold: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Network {
    pub request_timeout_seconds: u64,
    pub connect_timeout_seconds: u64,
    pub maximum_response_bytes: usize,
    pub maximum_redirects: usize,
    pub maximum_profile_requests: usize,
    pub snapshot_comparisons: usize,
    pub negotiation_timeout_seconds: u64,
    pub maximum_url_bytes: usize,
    pub user_agents: Vec<String>,
    pub github_api_base: String,
    pub latest_release_url: String,
    pub rule_cache_seconds: u64,
    pub doh_url: String,
    pub doh_addresses: Vec<SocketAddr>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub guest_documents: usize,
    pub administrator_documents: usize,
    pub nodes_per_subscription: usize,
    pub import_sources: usize,
    pub http_body_bytes: usize,
    pub small_http_body_bytes: usize,
    pub input_bytes: usize,
    pub body_timeout_seconds: u64,
    pub identifier_bytes: usize,
    pub name_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authentication {
    pub guest_cookie: String,
    pub administrator_cookie: String,
    pub guest_session_seconds: i64,
    pub administrator_session_seconds: i64,
    pub minimum_password_characters: usize,
    pub maximum_password_bytes: usize,
    pub parallel_logins: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimits {
    pub requests: u32,
    pub request_window_seconds: u64,
    pub expensive_operations: u32,
    pub operation_window_seconds: u64,
    pub logins: u32,
    pub login_window_seconds: u64,
    pub subscription_downloads: u32,
    pub download_window_seconds: u64,
    pub maximum_clients: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    pub rule_base_url: String,
    pub test_url: String,
    pub test_interval_seconds: u64,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if path.is_dir() {
            eprintln!(
                "warning: settings path {} is a directory (likely created by a Docker file mount when the host file was missing); falling back to built-in default configuration",
                path.display()
            );
            return Ok(Self::default());
        }
        if !path.exists()
            && path.file_name().and_then(|name| name.to_str()) == Some("subboost.toml")
            && path.parent().is_some_and(Path::is_dir)
        {
            let defaults = crate::include_config_str!("subboost.toml");
            if let Err(error) = std::fs::write(path, defaults) {
                eprintln!(
                    "warning: could not initialize default settings at {}: {error}; using built-in defaults",
                    path.display()
                );
            }
            return Self::parse(defaults).with_context(|| format!("Settings {}", path.display()));
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Reading settings {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("Settings {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).context("Invalid TOML settings")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let app =
            url::Url::parse(&self.application.app_url).context("Invalid application.app_url")?;
        ensure!(
            matches!(app.scheme(), "http" | "https")
                && app.host_str().is_some()
                && app.username().is_empty()
                && app.password().is_none()
                && app.query().is_none()
                && app.fragment().is_none(),
            "application.app_url must be an HTTP(S) origin or base path"
        );
        ensure!(
            !self.application.database.as_os_str().is_empty(),
            "application.database is required"
        );
        ensure!(
            self.application.sqlite_busy_timeout_seconds > 0,
            "SQLite busy timeout must be positive"
        );
        ensure!(
            !self.application.local_owner.is_empty()
                && !self.application.local_owner.starts_with("guest:"),
            "Invalid application.local_owner"
        );
        ensure!(
            self.refresh.poll_seconds > 0
                && self.refresh.cache_seconds > 0
                && self.refresh.failure_threshold > 0
                && self.refresh.minimum_interval_seconds > 0
                && self.refresh.maximum_interval_seconds >= self.refresh.minimum_interval_seconds,
            "Refresh durations and failure threshold must be positive and ordered"
        );
        ensure!(
            self.network.request_timeout_seconds > 0
                && self.network.connect_timeout_seconds > 0
                && self.network.negotiation_timeout_seconds > 0
                && self.network.rule_cache_seconds > 0
                && self.network.maximum_response_bytes > 0
                && self.network.maximum_url_bytes > 0
                && self.network.maximum_profile_requests > 0
                && !self.network.user_agents.is_empty(),
            "Network budgets must be positive"
        );
        ensure!(
            self.network
                .user_agents
                .iter()
                .all(|value| !value.is_empty()
                    && reqwest::header::HeaderValue::from_str(value).is_ok()),
            "Invalid network.user_agents"
        );
        for (name, value) in [
            ("github_api_base", &self.network.github_api_base),
            ("latest_release_url", &self.network.latest_release_url),
            ("doh_url", &self.network.doh_url),
            ("rule_base_url", &self.generation.rule_base_url),
            ("test_url", &self.generation.test_url),
        ] {
            let url = url::Url::parse(value).with_context(|| format!("Invalid {name}"))?;
            ensure!(
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
                "{name} must be HTTP(S)"
            );
        }
        ensure!(
            self.network
                .doh_addresses
                .iter()
                .all(|address| crate::network::is_public_ip(address.ip())),
            "DoH addresses must be public"
        );
        ensure!(
            self.limits.guest_documents > 0
                && self.limits.administrator_documents > 0
                && self.limits.nodes_per_subscription > 0
                && self.limits.import_sources > 0
                && self.limits.http_body_bytes > 0
                && self.limits.small_http_body_bytes > 0
                && self.limits.input_bytes > 0
                && self.limits.input_bytes < usize::MAX
                && self.limits.body_timeout_seconds > 0
                && self.limits.identifier_bytes > 0
                && self.limits.name_bytes > 0,
            "Application limits must be positive"
        );
        ensure!(
            self.authentication.guest_session_seconds > 0
                && self.authentication.administrator_session_seconds > 0
                && self.authentication.minimum_password_characters > 0
                && self.authentication.parallel_logins > 0,
            "Authentication durations and limits must be positive"
        );
        for name in [
            &self.authentication.guest_cookie,
            &self.authentication.administrator_cookie,
        ] {
            ensure!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
                "Invalid authentication cookie name"
            );
        }
        ensure!(
            self.authentication.guest_cookie != self.authentication.administrator_cookie,
            "Guest and administrator cookie names must differ"
        );
        ensure!(
            self.rate_limits.requests > 0
                && self.rate_limits.expensive_operations > 0
                && self.rate_limits.logins > 0
                && self.rate_limits.subscription_downloads > 0
                && self.rate_limits.request_window_seconds > 0
                && self.rate_limits.operation_window_seconds > 0
                && self.rate_limits.login_window_seconds > 0
                && self.rate_limits.download_window_seconds > 0
                && self.rate_limits.maximum_clients > 0
                && self.generation.test_interval_seconds > 0,
            "Rate and generation limits must be positive"
        );
        ensure!(
            (self.refresh.minimum_interval_seconds..=self.refresh.maximum_interval_seconds)
                .contains(&self.refresh.default_interval_seconds)
                && self.refresh.step_interval_seconds > 0,
            "Default refresh interval must be within the permitted range"
        );
        ensure!(
            self.authentication.maximum_password_bytes
                >= self.authentication.minimum_password_characters
                && self.network.snapshot_comparisons > 0,
            "Invalid password or snapshot budget"
        );
        ensure!(
            self.rules.page_size > 0
                && self.rules.page_size <= self.rules.maximum_page_size
                && self.rules.maximum_page > 0
                && self.rules.maximum_lines > 0
                && self.rules.maximum_discoveries > 0
                && self.rules.parallel_requests > 0
                && self.rules.maximum_discovery_bytes > 0,
            "Rule limits must be positive and ordered"
        );
        Ok(())
    }

    /// Load the editable repository policy when present; standalone binaries retain its packaged defaults.
    pub fn load_default() -> Result<Self> {
        let explicit = std::env::var_os("SUBBOOST_CONFIG");
        if let Some(explicit_path) = explicit {
            let path = PathBuf::from(explicit_path);
            return Self::load(&path);
        }

        if let Some(path) = resolve_asset_path("subboost.toml") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                return Self::parse(&text);
            }
        }

        Ok(Self::default())
    }

    pub fn document_quota(&self, owner: &str) -> usize {
        if owner.starts_with("guest:") {
            self.limits.guest_documents
        } else {
            self.limits.administrator_documents
        }
    }
}

/// 编译期静态嵌入 `backend/config/` 下的配置资源文件。
#[macro_export]
macro_rules! include_config_str {
    ($subpath:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/config/", $subpath))
    };
}

/// 运行时解析配置资源，结合环境变量覆盖、磁盘安全探测与编译期安全回退。
#[macro_export]
macro_rules! load_config_asset {
    ($env_var:literal, $subpath:literal) => {
        $crate::config::load_asset($env_var, $subpath, $crate::include_config_str!($subpath))
    };
}

impl Default for Config {
    fn default() -> Self {
        Self::parse(crate::include_config_str!("subboost.toml"))
            .expect("checked-in default configuration must be valid")
    }
}

/// 安全解析配置资源的磁盘路径，按优先级进行多环境探测并防范路径穿越。
pub(crate) fn resolve_asset_path(subpath: &str) -> Option<PathBuf> {
    if subpath.is_empty() || subpath.contains("..") {
        return None;
    }

    let subpath = std::path::Path::new(subpath);
    let candidates = [
        // 1. 从项目根目录运行（开发、CI、npm scripts）
        PathBuf::from("backend").join("config").join(subpath),
        // 2. 从 backend 子目录运行（cargo test, cargo run）
        PathBuf::from("config").join(subpath),
        // 3. 容器标准数据卷与系统配置目录（NAS 单目录挂载 / Docker 默认路径）
        PathBuf::from("/data").join(subpath),
        PathBuf::from("/etc/subboost").join(subpath),
        // 4. 根目录兼容软链接或绝对子路径
        PathBuf::from(subpath),
    ];

    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // 4. 二进制所在目录同级的 config/ 目录（适用于独立部署包）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("config").join(subpath);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// 加载配置资源：优先环境变量覆盖，其次探测磁盘文件，最终安全回退到编译期内嵌内容。
pub(crate) fn load_asset(env_var: &str, subpath: &str, embedded: &'static str) -> String {
    let explicit = std::env::var_os(env_var);
    if let Some(explicit_path) = explicit {
        return match std::fs::read_to_string(&explicit_path) {
            Ok(content) => content,
            Err(error) => panic!(
                "Failed to read {env_var} from {}: {error}",
                PathBuf::from(explicit_path).display()
            ),
        };
    }

    if let Some(path) = resolve_asset_path(subpath) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            return content;
        }
    }

    embedded.to_owned()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub page_size: u64,
    pub maximum_page_size: u64,
    pub maximum_page: u64,
    pub maximum_lines: usize,
    pub maximum_discoveries: usize,
    pub parallel_requests: usize,
    pub maximum_discovery_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::{Config, load_asset};

    #[test]
    fn rejects_typographical_keys_and_zero_refresh_interval() {
        let defaults = crate::include_config_str!("subboost.toml");
        assert!(Config::parse(&defaults.replace("poll_seconds = 60", "poll_seconds = 0")).is_err());
        assert!(Config::parse(&defaults.replace("poll_seconds", "poll_second")).is_err());
        let mut config = Config::default();
        config
            .authentication
            .administrator_cookie
            .clone_from(&config.authentication.guest_cookie);
        assert!(config.validate().is_err());
    }

    #[test]
    fn loads_default_and_file_or_embedded_fallback() {
        assert!(Config::load_default().is_ok());
        let content = load_asset(
            "SUBBOOST_NONEXISTENT_TEST_VAR",
            "nonexistent/path/to/test.json",
            "default_content",
        );
        assert_eq!(content, "default_content");

        let dir = tempfile::tempdir().expect("tempdir");
        let bootstrapped = dir.path().join("subboost.toml");
        assert!(Config::load(&bootstrapped).is_ok());
        assert!(bootstrapped.is_file());

        let dir_trap = dir.path().join("dir-mount").join("subboost.toml");
        std::fs::create_dir_all(&dir_trap).expect("create dir trap");
        assert!(Config::load(&dir_trap).is_ok());
    }
}
