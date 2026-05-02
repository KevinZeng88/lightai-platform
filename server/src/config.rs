use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub database_url: String,
    pub metrics_retention_days: u32,
    pub log_policy: crate::platform_log::LogPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            database_url: "sqlite://data/lightai.db".to_string(),
            metrics_retention_days: 7,
            log_policy: crate::platform_log::LogPolicy::default(),
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        match std::env::var("LIGHTAI_SERVER_CONFIG") {
            Ok(path) if !path.trim().is_empty() => Self::from_file(path),
            _ => Ok(Self::default()),
        }
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
                config.metrics_retention_days = value;
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
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    server: Option<ServerSection>,
    database: Option<DatabaseSection>,
    metrics: Option<MetricsSection>,
    logs: Option<LogsSection>,
}

#[derive(Debug, Deserialize)]
struct ServerSection {
    listen_addr: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DatabaseSection {
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MetricsSection {
    retention_days: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct LogsSection {
    dir: Option<String>,
    level: Option<String>,
    max_file_bytes: Option<u64>,
    retention_files: Option<usize>,
    retention_days: Option<u64>,
}
