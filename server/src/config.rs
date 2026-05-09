use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub database_url: String,
    pub metrics_retention_days: u32,
    pub history_cleanup_interval_hours: u32,
    pub password_policy: crate::repository::PasswordPolicy,
    pub session_policy: crate::repository::SessionPolicy,
    pub log_policy: crate::platform_log::LogPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:10080".to_string(),
            database_url: "sqlite://./data/lightai.db".to_string(),
            metrics_retention_days: 7,
            history_cleanup_interval_hours: 6,
            password_policy: crate::repository::PasswordPolicy::default(),
            session_policy: crate::repository::SessionPolicy::default(),
            log_policy: crate::platform_log::LogPolicy::default(),
        }
    }
}

/// Default Server config path.
const DEFAULT_SERVER_CONFIG_PATH: &str = "/etc/lightai/lightai-server.toml";

impl Config {
    /// Load config: `LIGHTAI_SERVER_CONFIG` env → default path → built-in defaults.
    /// Missing default path silently falls back to built-in defaults.
    pub fn load() -> anyhow::Result<Self> {
        if let Ok(path) = std::env::var("LIGHTAI_SERVER_CONFIG") {
            if !path.trim().is_empty() {
                return Self::from_file(path);
            }
        }
        if std::path::Path::new(DEFAULT_SERVER_CONFIG_PATH).is_file() {
            return Self::from_file(DEFAULT_SERVER_CONFIG_PATH);
        }
        Ok(Self::default())
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_config: FileConfig = toml::from_str(&content)?;
        let mut config = Self::default();

        if let Some(server) = file_config.server {
            if let Some(value) = server.listen_addr {
                config.listen_addr = value;
            }
        }

        if let Some(database) = file_config.database {
            if let Some(value) = database.url {
                config.database_url = value;
            }
        }

        if let Some(metrics) = file_config.metrics {
            if let Some(value) = metrics.retention_days {
                if value < 1 {
                    anyhow::bail!("metrics.retention_days must be at least 1");
                }
                config.metrics_retention_days = value;
            }
            if let Some(value) = metrics.cleanup_interval_hours {
                if value < 1 {
                    anyhow::bail!("metrics.cleanup_interval_hours must be at least 1");
                }
                config.history_cleanup_interval_hours = value;
            }
        }
        if let Some(auth) = file_config.auth {
            if let Some(password) = auth.password {
                if let Some(value) = password.min_length {
                    config.password_policy.min_length = value;
                }
                if let Some(value) = password.complexity_required {
                    config.password_policy.complexity_required = value;
                }
                if let Some(value) = password.expires_days {
                    config.password_policy.expires_days =
                        if value == 0 { None } else { Some(value) };
                }
                if let Some(value) = password.force_change_after_reset {
                    config.password_policy.force_change_after_reset = value;
                }
            }
            if let Some(session) = auth.session {
                if let Some(value) = session.ttl_secs {
                    config.session_policy.ttl_secs = value;
                }
                if let Some(value) = session.idle_timeout_secs {
                    config.session_policy.idle_timeout_secs =
                        if value == 0 { None } else { Some(value) };
                }
                if let Some(value) = session.secure_cookie {
                    config.session_policy.secure_cookie = value;
                }
            }
        }
        if let Some(logs) = file_config.logs {
            if let Some(value) = logs.dir {
                config.log_policy.log_dir = value;
            }
            if let Some(value) = logs.level {
                config.log_policy.log_level = value;
            }
            if let Some(value) = logs.max_file_bytes {
                config.log_policy.log_max_file_bytes = value;
            }
            if let Some(value) = logs.retention_files {
                config.log_policy.log_retention_files = value;
            }
            if let Some(value) = logs.retention_days {
                config.log_policy.log_retention_days = value;
            }
            crate::platform_log::validate_policy(&config.log_policy)?;
        }

        Ok(config)
    }

    pub fn validate_auth(&self) -> anyhow::Result<()> {
        if self.password_policy.min_length < 8 {
            anyhow::bail!("password min_length must be at least 8");
        }
        if self.session_policy.ttl_secs < 300 {
            anyhow::bail!("session ttl_secs must be at least 300");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    server: Option<ServerSection>,
    database: Option<DatabaseSection>,
    metrics: Option<MetricsSection>,
    auth: Option<AuthSection>,
    logs: Option<LogsSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerSection {
    listen_addr: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseSection {
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricsSection {
    retention_days: Option<u32>,
    cleanup_interval_hours: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthSection {
    password: Option<PasswordSection>,
    session: Option<SessionSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordSection {
    min_length: Option<usize>,
    complexity_required: Option<bool>,
    expires_days: Option<i64>,
    force_change_after_reset: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionSection {
    ttl_secs: Option<i64>,
    idle_timeout_secs: Option<i64>,
    secure_cookie: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogsSection {
    dir: Option<String>,
    level: Option<String>,
    max_file_bytes: Option<u64>,
    retention_files: Option<usize>,
    retention_days: Option<u64>,
}
