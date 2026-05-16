use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub listen_addr: String,
    pub log_level: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:18082".to_string(),
            log_level: "info".to_string(),
        }
    }
}

impl Config {
    pub fn from_file(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|error| {
            anyhow::anyhow!(
                "cannot read config file '{}': {error}",
                path.as_ref().display()
            )
        })?;
        Self::from_toml(&content)
    }

    pub fn from_toml(content: &str) -> anyhow::Result<Self> {
        let file_config: FileConfig = toml::from_str(content)?;
        let mut config = Self::default();

        if let Some(gateway) = file_config.gateway {
            if let Some(value) = gateway.listen_addr {
                if value.trim().is_empty() {
                    anyhow::bail!("gateway.listen_addr must not be empty");
                }
                config.listen_addr = value;
            }
            if let Some(value) = gateway.log_level {
                validate_log_level(&value)?;
                config.log_level = value;
            }
        }

        Ok(config)
    }
}

fn validate_log_level(value: &str) -> anyhow::Result<()> {
    match value {
        "error" | "warn" | "info" | "debug" | "trace" => Ok(()),
        _ => anyhow::bail!("gateway.log_level is invalid"),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    gateway: Option<GatewaySection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewaySection {
    listen_addr: Option<String>,
    log_level: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn default_config_uses_local_listen_addr() {
        let config = Config::default();

        assert_eq!(config.listen_addr, "127.0.0.1:18082");
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn loads_gateway_config_from_toml() {
        let config = Config::from_toml(
            r#"
[gateway]
listen_addr = "127.0.0.1:19090"
log_level = "debug"
"#,
        )
        .unwrap();

        assert_eq!(config.listen_addr, "127.0.0.1:19090");
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn rejects_unknown_gateway_config_fields() {
        let error = Config::from_toml(
            r#"
[gateway]
listen_addr = "127.0.0.1:19090"
api_key = "not-supported"
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("unknown field") || error.contains("unexpected"),
            "{error}"
        );
    }
}
