use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct RegisterRequest {
    pub name: String,
    pub hostname: String,
    pub agent_version: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterResponse {
    pub node_id: String,
    pub agent_token: String,
    pub heartbeat_interval_secs: u64,
    pub agent_config: Option<AgentConfig>,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatResponse {
    pub agent_config: Option<AgentConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub config_version: i64,
    pub heartbeat_interval_secs: u64,
    pub metrics_sample_interval_secs: u64,
    #[serde(default)]
    pub task_poll_interval_secs: u64,
    #[serde(default)]
    pub config_refresh_interval_secs: u64,
    pub command_timeout_secs: u64,
    pub environment_check_timeout_secs: u64,
    #[serde(default)]
    pub allowed_model_dirs: Vec<String>,
    #[serde(default = "default_nvidia_collector_enabled")]
    pub nvidia_collector_enabled: bool,
    #[serde(default)]
    pub custom_collector_script: Option<String>,
    #[serde(default = "default_collector_timeout_secs")]
    pub collector_timeout_secs: u64,
    #[serde(default = "default_collector_max_output_bytes")]
    pub collector_max_output_bytes: usize,
    pub last_config_updated_at: Option<i64>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            config_version: 0,
            heartbeat_interval_secs: 15,
            metrics_sample_interval_secs: 15,
            task_poll_interval_secs: 15,
            config_refresh_interval_secs: 60,
            command_timeout_secs: 5,
            environment_check_timeout_secs: 5,
            allowed_model_dirs: Vec::new(),
            nvidia_collector_enabled: true,
            custom_collector_script: None,
            collector_timeout_secs: default_collector_timeout_secs(),
            collector_max_output_bytes: default_collector_max_output_bytes(),
            last_config_updated_at: None,
        }
    }
}

fn default_nvidia_collector_enabled() -> bool {
    true
}

fn default_collector_timeout_secs() -> u64 {
    5
}

fn default_collector_max_output_bytes() -> usize {
    1024 * 1024
}

#[derive(Debug, Serialize)]
pub struct HeartbeatRequest {
    pub node_id: String,
    pub sampled_at: i64,
    pub metrics: NodeMetrics,
    pub gpus: Vec<GpuMetrics>,
    pub collector_errors: Vec<String>,
    pub agent_config: AgentConfig,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NodeMetrics {
    pub cpu_usage_percent: Option<f64>,
    pub memory_total_bytes: Option<i64>,
    pub memory_used_bytes: Option<i64>,
    pub disk_total_bytes: Option<i64>,
    pub disk_used_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuMetrics {
    pub gpu_key: String,
    pub gpu_index: Option<i64>,
    pub vendor: String,
    pub name: String,
    pub uuid: Option<String>,
    pub driver_version: Option<String>,
    pub memory_total_bytes: Option<i64>,
    pub memory_used_bytes: Option<i64>,
    pub utilization_percent: Option<f64>,
    pub temperature_celsius: Option<f64>,
    pub power_watts: Option<f64>,
    pub collector: String,
    pub raw_json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentTaskPollRequest {
    pub node_id: String,
    pub current_config_version: i64,
}

#[derive(Debug, Deserialize)]
pub struct AgentTaskPollResponse {
    pub task: Option<AgentTask>,
    pub agent_config: Option<AgentConfig>,
}

#[derive(Debug, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub kind: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct AgentTaskResultRequest {
    pub node_id: String,
    pub status: String,
    pub result: serde_json::Value,
}
