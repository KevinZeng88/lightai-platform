use tokio::time::{sleep, Duration};

use crate::client::ServerClient;
use crate::config::Config;
use crate::gpu;
use crate::metrics::MetricsCollector;
use crate::models::{AgentConfig, HeartbeatRequest, RegisterRequest};
use crate::state::{self, AgentState};

pub async fn run(config: Config) {
    let mut metrics_collector = MetricsCollector::new();
    let mut runtime_config = RuntimeConfig::from_config(&config);

    loop {
        let sleep_secs = match run_once(&config, &runtime_config, &mut metrics_collector).await {
            Ok(next_config) => {
                runtime_config.apply_server_config(next_config);
                runtime_config.heartbeat_interval_secs
            }
            Err(error) => {
                tracing::warn!(%error, "heartbeat cycle failed");
                runtime_config.heartbeat_interval_secs
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

    let (gpus, collector_errors) = gpu::collect_gpus(config).await;
    let request = HeartbeatRequest {
        node_id: agent_state.node_id.clone(),
        sampled_at: now_unix_secs(),
        metrics: metrics_collector.collect(),
        gpus,
        collector_errors,
        agent_config: runtime_config.to_agent_config(),
    };

    match client.heartbeat(&agent_state.agent_token, &request).await {
        Ok(response) => Ok(response.agent_config.or(next_config)),
        Err(error) if is_unauthorized(&error) => {
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
        Err(error) => Err(error),
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
        node_id: response.node_id,
        agent_token: response.agent_token,
    };
    state::save(&config.state_path, &state).await?;
    Ok(RegisteredAgent {
        state,
        agent_config: response.agent_config.unwrap_or_else(|| AgentConfig {
            config_version: 0,
            heartbeat_interval_secs: response.heartbeat_interval_secs,
            metrics_sample_interval_secs: config.metrics_sample_interval_secs,
            task_poll_interval_secs: config.task_poll_interval_secs,
            config_refresh_interval_secs: config.config_refresh_interval_secs,
            command_timeout_secs: config.command_timeout_secs,
            environment_check_timeout_secs: config.environment_check_timeout_secs,
            last_config_updated_at: None,
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
    pub task_poll_interval_secs: u64,
    pub config_refresh_interval_secs: u64,
    pub command_timeout_secs: u64,
    pub environment_check_timeout_secs: u64,
    pub last_config_updated_at: Option<i64>,
}

impl RuntimeConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            config_version: 0,
            heartbeat_interval_secs: config.heartbeat_interval_secs,
            metrics_sample_interval_secs: config.metrics_sample_interval_secs,
            task_poll_interval_secs: config.task_poll_interval_secs,
            config_refresh_interval_secs: config.config_refresh_interval_secs,
            command_timeout_secs: config.command_timeout_secs,
            environment_check_timeout_secs: config.environment_check_timeout_secs,
            last_config_updated_at: None,
        }
    }

    pub fn apply_server_config(&mut self, config: Option<AgentConfig>) {
        let Some(config) = config else {
            return;
        };
        if config.config_version >= self.config_version {
            self.config_version = config.config_version;
            self.heartbeat_interval_secs = config.heartbeat_interval_secs;
            self.metrics_sample_interval_secs = config.metrics_sample_interval_secs;
            self.task_poll_interval_secs = config.task_poll_interval_secs;
            self.config_refresh_interval_secs = config.config_refresh_interval_secs;
            self.command_timeout_secs = config.command_timeout_secs;
            self.environment_check_timeout_secs = config.environment_check_timeout_secs;
            self.last_config_updated_at = config.last_config_updated_at;
        }
    }

    pub fn to_agent_config(&self) -> AgentConfig {
        AgentConfig {
            config_version: self.config_version,
            heartbeat_interval_secs: self.heartbeat_interval_secs,
            metrics_sample_interval_secs: self.metrics_sample_interval_secs,
            task_poll_interval_secs: self.task_poll_interval_secs,
            config_refresh_interval_secs: self.config_refresh_interval_secs,
            command_timeout_secs: self.command_timeout_secs,
            environment_check_timeout_secs: self.environment_check_timeout_secs,
            last_config_updated_at: self.last_config_updated_at,
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
