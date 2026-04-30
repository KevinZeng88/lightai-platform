use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub database_url: String,
    pub metrics_retention_days: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            database_url: "sqlite://data/lightai.db".to_string(),
            metrics_retention_days: 7,
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

        Ok(config)
    }
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    server: Option<ServerSection>,
    database: Option<DatabaseSection>,
    metrics: Option<MetricsSection>,
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
