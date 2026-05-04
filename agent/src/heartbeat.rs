use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use crate::client::ServerClient;
use crate::config::Config;
use crate::gpu;
use crate::managed_process;
use crate::metrics::MetricsCollector;
use crate::models::{AgentConfig, HeartbeatRequest, RegisterRequest};
use crate::platform_log::{self, LogPolicy};
use crate::state::{self, AgentState};

pub async fn run(config: Config, runtime_config: Arc<RwLock<RuntimeConfig>>) {
    let mut metrics_collector = MetricsCollector::new();

    loop {
        let snapshot = runtime_config.read().await.clone();
        let _ = platform_log::append(
            &snapshot.log_policy,
            "agent.log",
            "debug",
            "Agent heartbeat cycle started",
        )
        .await;
        let sleep_secs = match run_once(&config, &snapshot, &mut metrics_collector).await {
            Ok(next_config) => {
                let mut runtime = runtime_config.write().await;
                runtime.apply_server_config(next_config);
                runtime.heartbeat_interval_secs
            }
            Err(error) => {
                tracing::warn!(%error, "heartbeat cycle failed");
                snapshot.heartbeat_interval_secs
            }
        };
        sleep(Duration::from_secs(sleep_secs)).await;
    }
}

async fn run_once(
    config: &Config,
    runtime_config: &RuntimeConfig,
    metrics_collector: &mut MetricsCollector,
) -> anyhow::Result<Option<AgentConfig>> {
    let client = ServerClient::new(config.server_url.clone());
    let mut next_config = None;
    let mut agent_state = match state::load(&config.state_path).await? {
        Some(state) => state,
        None => {
            let registered = register(&client, config).await?;
            next_config = Some(registered.agent_config.clone());
            registered.state
        }
    };

    let (gpus, collector_errors) = gpu::collect_gpus(&runtime_config.to_collector_config()).await;
    let managed_store_path = managed_process::store_path_from_state_path(&config.state_path);
    let managed_instances = managed_process::reports(Some(&managed_store_path)).await;
    let request = HeartbeatRequest {
        node_id: agent_state.node_id.clone(),
        sampled_at: now_unix_secs(),
        metrics: metrics_collector.collect(),
        gpus,
        collector_errors,
        agent_config: runtime_config.to_agent_config(),
        managed_instances,
    };

    match client.heartbeat(&agent_state.agent_token, &request).await {
        Ok(response) => {
            if let Some(ref agent_config) = response.agent_config {
                if agent_config.config_version
                    > runtime_config
                        .last_config_updated_at
                        .map_or(0, |_| runtime_config.config_version)
                {
                    let _ = platform_log::append(
                        &runtime_config.log_policy,
                        "agent.log",
                        "info",
                        &format!(
                            "Agent 配置已更新 config_version={}",
                            agent_config.config_version
                        ),
                    )
                    .await;
                }
            }
            Ok(response.agent_config.or(next_config))
        }
        Err(error) if is_unauthorized(&error) => {
            let _ = platform_log::append(
                &runtime_config.log_policy,
                "agent.log",
                "warn",
                "Agent token 过期，重新注册",
            )
            .await;
            let registered = register(&client, config).await?;
            next_config = Some(registered.agent_config.clone());
            agent_state = registered.state;
            let request = HeartbeatRequest {
                node_id: agent_state.node_id.clone(),
                ..request
            };
            let response = client.heartbeat(&agent_state.agent_token, &request).await?;
            Ok(response.agent_config.or(next_config))
        }
        Err(error) => {
            let _ = platform_log::append(
                &runtime_config.log_policy,
                "agent.log",
                "error",
                &format!("心跳失败：{error}"),
            )
            .await;
            Err(error)
        }
    }
}

struct RegisteredAgent {
    state: AgentState,
    agent_config: AgentConfig,
}

async fn register(client: &ServerClient, config: &Config) -> anyhow::Result<RegisteredAgent> {
    let response = client
        .register(&RegisterRequest {
            name: config.node_name.clone(),
            hostname: std::env::var("HOSTNAME").unwrap_or_else(|_| config.node_name.clone()),
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        })
        .await?;

    let state = AgentState {
        node_id: response.node_id.clone(),
        agent_token: response.agent_token.clone(),
    };
    state::save(&config.state_path, &state).await?;
    let _ = platform_log::append(
        &LogPolicy::default(),
        "agent.log",
        "info",
        &format!("Agent 注册成功 node_id={}", response.node_id),
    )
    .await;
    Ok(RegisteredAgent {
        state,
        agent_config: response.agent_config.unwrap_or_else(|| AgentConfig {
            config_version: 0,
            heartbeat_interval_secs: response.heartbeat_interval_secs,
            ..AgentConfig::default()
        }),
    })
}

pub fn next_interval_secs(
    config_interval_secs: u64,
    registration_interval_secs: Option<u64>,
) -> u64 {
    registration_interval_secs.unwrap_or(config_interval_secs)
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub config_version: i64,
    pub heartbeat_interval_secs: u64,
    pub metrics_sample_interval_secs: u64,
    pub command_timeout_secs: u64,
    pub environment_check_timeout_secs: u64,
    pub allowed_model_dirs: Vec<String>,
    pub nvidia_collector_enabled: bool,
    pub custom_collector_script: Option<String>,
    pub collector_timeout_secs: u64,
    pub collector_max_output_bytes: usize,
    pub log_policy: LogPolicy,
    pub last_config_updated_at: Option<i64>,
}

impl RuntimeConfig {
    pub fn default_effective() -> Self {
        Self {
            config_version: 0,
            heartbeat_interval_secs: 15,
            metrics_sample_interval_secs: 15,
            command_timeout_secs: 5,
            environment_check_timeout_secs: 5,
            allowed_model_dirs: Vec::new(),
            nvidia_collector_enabled: true,
            custom_collector_script: None,
            collector_timeout_secs: 5,
            collector_max_output_bytes: 1024 * 1024,
            log_policy: LogPolicy::default(),
            last_config_updated_at: None,
        }
    }

    pub fn from_config(_config: &Config) -> Self {
        Self::default_effective()
    }

    pub fn apply_server_config(&mut self, config: Option<AgentConfig>) {
        let Some(config) = config else {
            return;
        };
        if config.config_version >= self.config_version {
            self.config_version = config.config_version;
            self.heartbeat_interval_secs = config.heartbeat_interval_secs;
            self.metrics_sample_interval_secs = config.metrics_sample_interval_secs;
            self.command_timeout_secs = config.command_timeout_secs;
            self.environment_check_timeout_secs = config.environment_check_timeout_secs;
            self.allowed_model_dirs = config.allowed_model_dirs;
            self.nvidia_collector_enabled = config.nvidia_collector_enabled;
            self.custom_collector_script = config.custom_collector_script;
            self.collector_timeout_secs = config.collector_timeout_secs;
            self.collector_max_output_bytes = config.collector_max_output_bytes;
            self.log_policy = config.log_policy;
            self.last_config_updated_at = config.last_config_updated_at;
        }
    }

    pub fn to_agent_config(&self) -> AgentConfig {
        AgentConfig {
            config_version: self.config_version,
            heartbeat_interval_secs: self.heartbeat_interval_secs,
            metrics_sample_interval_secs: self.metrics_sample_interval_secs,
            task_poll_interval_secs: 15,
            config_refresh_interval_secs: 60,
            command_timeout_secs: self.command_timeout_secs,
            environment_check_timeout_secs: self.environment_check_timeout_secs,
            allowed_model_dirs: self.allowed_model_dirs.clone(),
            nvidia_collector_enabled: self.nvidia_collector_enabled,
            custom_collector_script: self.custom_collector_script.clone(),
            collector_timeout_secs: self.collector_timeout_secs,
            collector_max_output_bytes: self.collector_max_output_bytes,
            log_policy: self.log_policy.clone(),
            last_config_updated_at: self.last_config_updated_at,
        }
    }

    pub fn to_collector_config(&self) -> gpu::CollectorConfig {
        gpu::CollectorConfig {
            nvidia_collector_enabled: self.nvidia_collector_enabled,
            custom_collector_script: self.custom_collector_script.clone(),
            collector_timeout_secs: self.collector_timeout_secs,
            collector_max_output_bytes: self.collector_max_output_bytes,
        }
    }
}

fn is_unauthorized(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<reqwest::Error>()
        .and_then(reqwest::Error::status)
        .is_some_and(|status| status == reqwest::StatusCode::UNAUTHORIZED)
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
