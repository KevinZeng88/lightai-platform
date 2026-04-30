use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub hostname: String,
    pub agent_version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub node_id: String,
    pub agent_token: String,
    pub heartbeat_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub node_id: String,
    pub sampled_at: i64,
    #[serde(default)]
    pub metrics: NodeMetrics,
    #[serde(default)]
    pub gpus: Vec<GpuMetrics>,
    #[serde(default)]
    pub collector_errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NodeMetrics {
    pub cpu_usage_percent: Option<f64>,
    pub memory_total_bytes: Option<i64>,
    pub memory_used_bytes: Option<i64>,
    pub disk_total_bytes: Option<i64>,
    pub disk_used_bytes: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
pub struct HeartbeatResponse {
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct NodeListResponse {
    pub nodes: Vec<NodeView>,
}

#[derive(Debug, Serialize)]
pub struct NodeView {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub agent_version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub status: String,
    pub registered_at: i64,
    pub updated_at: i64,
    pub last_heartbeat_at: Option<i64>,
    pub metrics: Option<NodeMetrics>,
    pub gpus: Vec<GpuView>,
}

#[derive(Debug, Serialize)]
pub struct GpuView {
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
    pub updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct NodeMetricSamplesResponse {
    pub node_id: String,
    pub requested_from: i64,
    pub requested_to: i64,
    pub actual_from: Option<i64>,
    pub actual_to: Option<i64>,
    pub sample_count: usize,
    pub samples: Vec<NodeMetricSample>,
}

#[derive(Debug, Serialize)]
pub struct NodeMetricSample {
    pub sampled_at: i64,
    pub cpu_usage_percent: Option<f64>,
    pub memory_total_bytes: Option<i64>,
    pub memory_used_bytes: Option<i64>,
    pub disk_total_bytes: Option<i64>,
    pub disk_used_bytes: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct GpuMetricSamplesResponse {
    pub node_id: String,
    pub gpu_key: String,
    pub requested_from: i64,
    pub requested_to: i64,
    pub actual_from: Option<i64>,
    pub actual_to: Option<i64>,
    pub sample_count: usize,
    pub samples: Vec<GpuMetricSample>,
}

#[derive(Debug, Serialize)]
pub struct GpuMetricSample {
    pub sampled_at: i64,
    pub vendor: String,
    pub memory_total_bytes: Option<i64>,
    pub memory_used_bytes: Option<i64>,
    pub utilization_percent: Option<f64>,
    pub temperature_celsius: Option<f64>,
    pub power_watts: Option<f64>,
}
