use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub server_url: String,
    pub node_name: String,
    pub state_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8081".to_string(),
            server_url: "http://127.0.0.1:8080".to_string(),
            node_name: hostname(),
            state_path: "data/agent-state.toml".to_string(),
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
            if let Some(value) = agent.state_path {
                config.state_path = value;
            }
        }

        Ok(config)
    }
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    agent: Option<AgentSection>,
}

#[derive(Debug, Deserialize)]
struct AgentSection {
    listen_addr: Option<String>,
    server_url: Option<String>,
    node_name: Option<String>,
    state_path: Option<String>,
}

fn hostname() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "lightai-agent".to_string())
}
