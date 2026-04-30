use tokio::time::{sleep, Duration};

use crate::client::ServerClient;
use crate::config::Config;
use crate::gpu;
use crate::metrics::MetricsCollector;
use crate::models::{HeartbeatRequest, RegisterRequest};
use crate::state::{self, AgentState};

pub async fn run(config: Config) {
    let mut metrics_collector = MetricsCollector::new();

    loop {
        let sleep_secs = match run_once(&config, &mut metrics_collector).await {
            Ok(registration_interval_secs) => {
                next_interval_secs(config.heartbeat_interval_secs, registration_interval_secs)
            }
            Err(error) => {
                tracing::warn!(%error, "heartbeat cycle failed");
                config.heartbeat_interval_secs
            }
        };
        sleep(Duration::from_secs(sleep_secs)).await;
    }
}

async fn run_once(
    config: &Config,
    metrics_collector: &mut MetricsCollector,
) -> anyhow::Result<Option<u64>> {
    let client = ServerClient::new(config.server_url.clone());
    let mut registration_interval_secs = None;
    let mut agent_state = match state::load(&config.state_path).await? {
        Some(state) => state,
        None => {
            let registered = register(&client, config).await?;
            registration_interval_secs = Some(registered.heartbeat_interval_secs);
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
    };

    match client.heartbeat(&agent_state.agent_token, &request).await {
        Ok(()) => Ok(registration_interval_secs),
        Err(error) if is_unauthorized(&error) => {
            let registered = register(&client, config).await?;
            registration_interval_secs = Some(registered.heartbeat_interval_secs);
            agent_state = registered.state;
            let request = HeartbeatRequest {
                node_id: agent_state.node_id.clone(),
                ..request
            };
            client.heartbeat(&agent_state.agent_token, &request).await?;
            Ok(registration_interval_secs)
        }
        Err(error) => Err(error),
    }
}

struct RegisteredAgent {
    state: AgentState,
    heartbeat_interval_secs: u64,
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
        heartbeat_interval_secs: response.heartbeat_interval_secs,
    })
}

pub fn next_interval_secs(
    config_interval_secs: u64,
    registration_interval_secs: Option<u64>,
) -> u64 {
    registration_interval_secs.unwrap_or(config_interval_secs)
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
