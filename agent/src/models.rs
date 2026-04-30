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
    pub task_poll_interval_secs: u64,
    pub config_refresh_interval_secs: u64,
    pub command_timeout_secs: u64,
    pub environment_check_timeout_secs: u64,
    pub last_config_updated_at: Option<i64>,
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
