use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub server_url: String,
    pub node_name: String,
    pub heartbeat_interval_secs: u64,
    pub state_path: String,
    pub nvidia_collector_enabled: bool,
    pub custom_collector_script: Option<String>,
    pub collector_timeout_secs: u64,
    pub collector_max_output_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8081".to_string(),
            server_url: "http://127.0.0.1:8080".to_string(),
            node_name: hostname(),
            heartbeat_interval_secs: 15,
            state_path: "data/agent-state.toml".to_string(),
            nvidia_collector_enabled: true,
            custom_collector_script: None,
            collector_timeout_secs: 5,
            collector_max_output_bytes: 1024 * 1024,
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        match std::env::var("LIGHTAI_AGENT_CONFIG") {
            Ok(path) if !path.trim().is_empty() => Self::from_file(path),
            _ => Ok(Self::default()),
        }
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let file_config: FileConfig = toml::from_str(&content)?;
        let mut config = Self::default();

        if let Some(agent) = file_config.agent {
            if let Some(value) = agent.listen_addr {
                config.listen_addr = value;
            }
            if let Some(value) = agent.server_url {
                config.server_url = value;
            }
            if let Some(value) = agent.node_name {
                config.node_name = value;
            }
            if let Some(value) = agent.heartbeat_interval_secs {
                config.heartbeat_interval_secs = value;
            }
            if let Some(value) = agent.state_path {
                config.state_path = value;
            }
        }

        if let Some(collectors) = file_config.collectors {
            if let Some(nvidia) = collectors.nvidia {
                if let Some(value) = nvidia.enabled {
                    config.nvidia_collector_enabled = value;
                }
            }
            if let Some(custom) = collectors.custom {
                if let Some(value) = custom.script_path.filter(|value| !value.trim().is_empty()) {
                    config.custom_collector_script = Some(value);
                }
                if let Some(value) = custom.timeout_secs {
                    config.collector_timeout_secs = value;
                }
                if let Some(value) = custom.max_output_bytes {
                    config.collector_max_output_bytes = value;
                }
            }
        }

        Ok(config)
    }
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    agent: Option<AgentSection>,
    collectors: Option<CollectorsSection>,
}

#[derive(Debug, Deserialize)]
struct AgentSection {
    listen_addr: Option<String>,
    server_url: Option<String>,
    node_name: Option<String>,
    heartbeat_interval_secs: Option<u64>,
    state_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CollectorsSection {
    nvidia: Option<NvidiaSection>,
    custom: Option<CustomSection>,
}

#[derive(Debug, Deserialize)]
struct NvidiaSection {
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CustomSection {
    script_path: Option<String>,
    timeout_secs: Option<u64>,
    max_output_bytes: Option<usize>,
}

fn hostname() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "lightai-agent".to_string())
}
