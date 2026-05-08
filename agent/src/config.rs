use serde::Deserialize;

/// Config loading source, for logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Cli,
    Env,
    ExecutableDir,
    BuiltInDefault,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub server_url: String,
    pub node_name: String,
    pub state_path: String,
    pub collector_root: Option<String>,
    pub collector_mode: String,
    pub collector_enabled: Vec<String>,
    pub collector_disabled: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8081".to_string(),
            server_url: "http://127.0.0.1:8080".to_string(),
            node_name: hostname(),
            state_path: "data/agent-state.toml".to_string(),
            collector_root: None,
            collector_mode: "explicit".to_string(),
            collector_enabled: Vec::new(),
            collector_disabled: Vec::new(),
        }
    }
}

impl Config {
    /// Load config with full priority:
    /// 1. `--config <PATH>` (cli_path)
    /// 2. `LIGHTAI_AGENT_CONFIG` env var
    /// 3. `<executable_dir>/agent.toml`
    /// 4. `<executable_dir>/lightai-agent.toml`
    /// 5. Built-in defaults
    ///
    /// Returns the config and the source used for logging.
    pub fn load_with_priority(cli_path: Option<&str>) -> anyhow::Result<(Self, ConfigSource)> {
        // 1. --config
        if let Some(path) = cli_path {
            if path.is_empty() {
                anyhow::bail!("--config requires a non-empty path");
            }
            let config = Self::from_file(path)?;
            return Ok((config, ConfigSource::Cli));
        }

        // 2. LIGHTAI_AGENT_CONFIG env var
        if let Ok(env_path) = std::env::var("LIGHTAI_AGENT_CONFIG") {
            if !env_path.trim().is_empty() {
                let config = Self::from_file(&env_path)?;
                return Ok((config, ConfigSource::Env));
            }
        }

        // 3 & 4. Executable directory defaults
        if let Some(dir) = executable_dir() {
            for name in &["agent.toml", "lightai-agent.toml"] {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    let config = Self::from_file(&candidate)?;
                    return Ok((config, ConfigSource::ExecutableDir));
                }
            }
        }

        // 5. Built-in defaults
        Ok((Self::default(), ConfigSource::BuiltInDefault))
    }

    /// Legacy load method — uses env var only, for backward compat in tests.
    pub fn load() -> anyhow::Result<Self> {
        match std::env::var("LIGHTAI_AGENT_CONFIG") {
            Ok(path) if !path.trim().is_empty() => Self::from_file(path),
            _ => Ok(Self::default()),
        }
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            anyhow::anyhow!("cannot read config file '{}': {e}", path.as_ref().display())
        })?;
        Self::from_toml(&content)
    }

    pub fn from_toml(toml_str: &str) -> anyhow::Result<Self> {
        let file_config: FileConfig = toml::from_str(toml_str)?;
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

        if let Some(gc) = file_config.gpu_collectors {
            if let Some(root) = gc.root {
                if !root.trim().is_empty() {
                    config.collector_root = Some(root);
                }
            }
            if let Some(mode) = gc.mode {
                match mode.as_str() {
                    "explicit" | "auto" => config.collector_mode = mode,
                    other => anyhow::bail!(
                        "gpu_collectors.mode must be 'explicit' or 'auto', got '{other}'"
                    ),
                }
            }
            if let Some(enabled) = gc.enabled {
                config.collector_enabled = enabled;
            }
            if let Some(disabled) = gc.disabled {
                config.collector_disabled = disabled;
            }
        }

        Ok(config)
    }

    pub fn source_label(source: ConfigSource) -> &'static str {
        match source {
            ConfigSource::Cli => "cli (--config)",
            ConfigSource::Env => "env (LIGHTAI_AGENT_CONFIG)",
            ConfigSource::ExecutableDir => "executable_dir (agent.toml / lightai-agent.toml)",
            ConfigSource::BuiltInDefault => "built_in_default",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    agent: Option<AgentSection>,
    gpu_collectors: Option<GpuCollectorsSection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentSection {
    listen_addr: Option<String>,
    server_url: Option<String>,
    node_name: Option<String>,
    state_path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GpuCollectorsSection {
    root: Option<String>,
    mode: Option<String>,
    enabled: Option<Vec<String>>,
    disabled: Option<Vec<String>>,
}

fn hostname() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "lightai-agent".to_string())
}

fn executable_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(std::path::Path::to_path_buf)
}

/// Template TOML content for `lightai-agent config init`.
pub const CONFIG_TEMPLATE: &str = r#"# LightAI Agent configuration.
# Generated by: lightai-agent config init

[agent]
# Agent local listen address.
listen_addr = "127.0.0.1:8081"

# LightAI Server URL.
server_url = "http://127.0.0.1:8080"

# Node name.  Defaults to hostname if omitted or empty.
# node_name = "gpu-node-01"

# Agent state file path.
state_path = "data/agent-state.toml"

# ── GPU / accelerator collector configuration (optional) ──
# Uncomment [gpu_collectors] and set root to enable the script-based collector
# framework.  Without [gpu_collectors], no GPU collector scripts execute.
#
# Enabling workflow:
#   1. Place collector files on the agent machine, e.g.
#      /opt/lightai/collectors/gpu/nvidia/{collector.toml,discover.sh,metrics.sh}
#   2. Run: lightai-agent collector inspect /opt/lightai/collectors/gpu/nvidia
#   3. Paste the JSON output into Web → 采集器登记
#   4. Uncomment [gpu_collectors] below and set root + enabled
#   5. Start the agent
#
# Without a registered Server registry, the framework fails closed (no scripts run).

# [gpu_collectors]
# root = "/opt/lightai/collectors/gpu"
# mode = "explicit"         # "explicit" (only enabled list) or "auto" (scan all)
# enabled = ["nvidia"]
# disabled = []
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn parses_minimal_agent_config() {
        let toml = "[agent]\nlisten_addr = \"0.0.0.0:9090\"\n";
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.listen_addr, "0.0.0.0:9090");
        assert!(config.collector_root.is_none());
    }

    #[test]
    fn missing_gpu_collectors_section_not_an_error() {
        let toml = "[agent]\nnode_name = \"test-node\"\n";
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.node_name, "test-node");
        assert!(config.collector_root.is_none());
    }

    #[test]
    fn parses_gpu_collectors_section() {
        let toml = r#"
[agent]
node_name = "gpu-01"

[gpu_collectors]
root = "/opt/lightai/collectors/gpu"
mode = "explicit"
enabled = ["nvidia"]
disabled = ["nvidia-legacy"]
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(
            config.collector_root,
            Some("/opt/lightai/collectors/gpu".to_string())
        );
        assert_eq!(config.collector_mode, "explicit");
        assert_eq!(config.collector_enabled, vec!["nvidia"]);
    }

    #[test]
    fn gpu_collectors_invalid_mode_errors() {
        let toml = "[gpu_collectors]\nroot = \"/tmp/gpu\"\nmode = \"invalid\"\n";
        assert!(Config::from_toml(toml).is_err());
    }

    #[test]
    fn config_template_parses() {
        let config = Config::from_toml(CONFIG_TEMPLATE).unwrap();
        assert_eq!(config.listen_addr, "127.0.0.1:8081");
        assert!(config.collector_root.is_none()); // commented out
    }

    #[test]
    fn config_template_with_gpu_uncommented_parses() {
        let toml = CONFIG_TEMPLATE
            .replace("# [gpu_collectors]", "[gpu_collectors]")
            .replace("# root =", "root =")
            .replace("# mode =", "mode =")
            .replace("# enabled =", "enabled =")
            .replace("# disabled =", "disabled =");
        let config = Config::from_toml(&toml).unwrap();
        assert_eq!(
            config.collector_root,
            Some("/opt/lightai/collectors/gpu".to_string())
        );
        assert_eq!(config.collector_mode, "explicit");
        assert_eq!(config.collector_enabled, vec!["nvidia"]);
    }

    #[test]
    fn cli_path_has_top_priority() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let cli_file = dir.path().join("cli.toml");
        std::fs::write(&cli_file, "[agent]\nnode_name = \"cli-node\"\n").unwrap();
        // Set env to point somewhere else.
        let env_file = dir.path().join("env.toml");
        std::fs::write(&env_file, "[agent]\nnode_name = \"env-node\"\n").unwrap();
        std::env::set_var("LIGHTAI_AGENT_CONFIG", env_file.to_str().unwrap());

        let (config, source) =
            Config::load_with_priority(Some(cli_file.to_str().unwrap())).unwrap();
        assert_eq!(config.node_name, "cli-node");
        assert_eq!(source, ConfigSource::Cli);

        std::env::remove_var("LIGHTAI_AGENT_CONFIG");
    }

    #[test]
    fn env_path_falls_back_from_cli() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let env_file = dir.path().join("env.toml");
        std::fs::write(&env_file, "[agent]\nnode_name = \"env-node\"\n").unwrap();
        std::env::set_var("LIGHTAI_AGENT_CONFIG", env_file.to_str().unwrap());

        let (config, source) = Config::load_with_priority(None).unwrap();
        assert_eq!(config.node_name, "env-node");
        assert_eq!(source, ConfigSource::Env);

        std::env::remove_var("LIGHTAI_AGENT_CONFIG");
    }

    #[test]
    fn explicit_cli_missing_file_errors() {
        let result = Config::load_with_priority(Some("/nonexistent/path/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn explicit_env_missing_file_errors() {
        let _guard = env_lock();
        std::env::set_var("LIGHTAI_AGENT_CONFIG", "/nonexistent/path/env.toml");
        let result = Config::load_with_priority(None);
        std::env::remove_var("LIGHTAI_AGENT_CONFIG");
        assert!(result.is_err());
    }

    #[test]
    fn no_config_falls_back_to_defaults() {
        let _guard = env_lock();
        std::env::remove_var("LIGHTAI_AGENT_CONFIG");
        let (config, source) = Config::load_with_priority(None).unwrap();
        assert_eq!(source, ConfigSource::BuiltInDefault);
        // defaults
        assert_eq!(config.listen_addr, "127.0.0.1:8081");
    }

    #[test]
    fn from_file_missing_errors() {
        let result = Config::from_file("/nonexistent/path.toml");
        assert!(result.is_err());
    }

    #[test]
    fn empty_cli_path_errors() {
        let result = Config::load_with_priority(Some(""));
        assert!(result.is_err());
    }

    // ── exec-dir tests: these manipulate files in the test binary directory
    //     and require LIGHTAI_AGENT_CONFIG to be unset.

    #[test]
    fn executable_dir_agent_toml_loaded() {
        let _guard = env_lock();
        std::env::remove_var("LIGHTAI_AGENT_CONFIG");

        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let agent_toml = exe_dir.join("agent.toml");
        let lightai_toml = exe_dir.join("lightai-agent.toml");
        let _ = std::fs::remove_file(&agent_toml);
        let _ = std::fs::remove_file(&lightai_toml);
        std::fs::write(&agent_toml, "[agent]\nnode_name = \"exec-dir-node\"\n").unwrap();

        let (config, source) = Config::load_with_priority(None).unwrap();
        let _ = std::fs::remove_file(&agent_toml);
        assert_eq!(config.node_name, "exec-dir-node");
        assert_eq!(source, ConfigSource::ExecutableDir);
    }

    #[test]
    fn executable_dir_lightai_agent_toml_fallback() {
        let _guard = env_lock();
        std::env::remove_var("LIGHTAI_AGENT_CONFIG");

        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let agent_toml = exe_dir.join("agent.toml");
        let lightai_toml = exe_dir.join("lightai-agent.toml");
        let _ = std::fs::remove_file(&agent_toml);
        let _ = std::fs::remove_file(&lightai_toml);
        std::fs::write(&lightai_toml, "[agent]\nnode_name = \"lightai-dir-node\"\n").unwrap();

        let (config, source) = Config::load_with_priority(None).unwrap();
        let _ = std::fs::remove_file(&lightai_toml);
        assert_eq!(config.node_name, "lightai-dir-node");
        assert_eq!(source, ConfigSource::ExecutableDir);
    }

    #[test]
    fn executable_dir_both_present_agent_toml_wins() {
        let _guard = env_lock();
        std::env::remove_var("LIGHTAI_AGENT_CONFIG");

        let exe_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let agent_toml = exe_dir.join("agent.toml");
        let lightai_toml = exe_dir.join("lightai-agent.toml");
        let _ = std::fs::remove_file(&agent_toml);
        let _ = std::fs::remove_file(&lightai_toml);
        // Both exist — agent.toml must win (checked first).
        std::fs::write(&agent_toml, "[agent]\nnode_name = \"first-wins\"\n").unwrap();
        std::fs::write(&lightai_toml, "[agent]\nnode_name = \"second-loses\"\n").unwrap();

        let (config, source) = Config::load_with_priority(None).unwrap();
        let _ = std::fs::remove_file(&agent_toml);
        let _ = std::fs::remove_file(&lightai_toml);
        assert_eq!(config.node_name, "first-wins");
        assert_eq!(source, ConfigSource::ExecutableDir);
    }
}
