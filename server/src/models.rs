use serde::{Deserialize, Serialize};

use crate::platform_log::LogPolicy;

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
    pub agent_config: AgentConfig,
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
    pub agent_config: Option<AgentConfig>,
    #[serde(default)]
    pub managed_instances: Vec<ManagedInstanceReport>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManagedInstanceReport {
    pub instance_id: String,
    pub status: String,
    pub message: String,
    pub process_id: Option<i64>,
    pub process_ref: Option<String>,
    pub base_url: Option<String>,
    pub endpoint_url: Option<String>,
    pub command: Option<String>,
    pub log_path: Option<String>,
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
    #[serde(default, flatten)]
    pub log_policy: LogPolicy,
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
            log_policy: LogPolicy::default(),
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

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AgentConfigPolicy {
    pub heartbeat_interval_secs: Option<u64>,
    pub metrics_sample_interval_secs: Option<u64>,
    pub command_timeout_secs: Option<u64>,
    pub environment_check_timeout_secs: Option<u64>,
    pub allowed_model_dirs: Option<Vec<String>>,
    pub nvidia_collector_enabled: Option<bool>,
    pub custom_collector_script: Option<Option<String>>,
    pub collector_timeout_secs: Option<u64>,
    pub collector_max_output_bytes: Option<usize>,
    pub log_dir: Option<String>,
    pub log_level: Option<String>,
    pub log_max_file_bytes: Option<u64>,
    pub log_retention_files: Option<usize>,
    pub log_retention_days: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct AgentConfigPolicyView {
    pub scope: String,
    pub node_id: Option<String>,
    pub version: i64,
    pub updated_at: i64,
    pub policy: AgentConfigPolicy,
    pub effective_config: AgentConfig,
    pub restart_required_fields: Vec<&'static str>,
    pub online_reload_fields: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct AgentConfigPoliciesResponse {
    pub global: AgentConfigPolicyView,
    pub nodes: Vec<AgentConfigPolicyView>,
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
    pub agent_config: AgentConfig,
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
    pub agent_config: Option<AgentConfig>,
    pub effective_agent_config: AgentConfig,
    pub config_sync_status: String,
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

#[derive(Debug, Deserialize)]
pub struct GpuMetricsQuery {
    pub gpu_key: String,
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

#[derive(Debug, Deserialize)]
pub struct RuntimeEnvironmentRequest {
    pub name: String,
    pub backend: String,
    pub deploy_type: String,
    pub version: Option<String>,
    pub base_url: Option<String>,
    pub health_url: Option<String>,
    pub endpoint_url: Option<String>,
    pub binary_path: Option<String>,
    pub docker_image: Option<String>,
    pub working_dir: Option<String>,
    pub log_dir: Option<String>,
    pub allowed_model_dirs_json: Option<String>,
    pub config_json: Option<String>,
    pub params_json: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeEnvironmentView {
    pub id: String,
    pub node_id: Option<String>,
    pub name: String,
    pub backend: String,
    pub deploy_type: String,
    pub version: Option<String>,
    pub base_url: Option<String>,
    pub health_url: Option<String>,
    pub endpoint_url: Option<String>,
    pub binary_path: Option<String>,
    pub docker_image: Option<String>,
    pub working_dir: Option<String>,
    pub log_dir: Option<String>,
    pub allowed_model_dirs_json: Option<String>,
    pub config_json: Option<String>,
    pub params_json: Option<String>,
    pub enabled: bool,
    pub last_checked_at: Option<i64>,
    pub check_status: Option<String>,
    pub check_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct RuntimeEnvironmentListResponse {
    pub runtime_environments: Vec<RuntimeEnvironmentView>,
}

#[derive(Debug, Deserialize)]
pub struct ModelRequest {
    pub name: String,
    pub display_name: Option<String>,
    pub model_type: String,
    pub model_path: Option<String>,
    pub description: Option<String>,
    pub default_backend: Option<String>,
    pub config_json: Option<String>,
    #[serde(default)]
    pub params_json: Option<String>,
    pub initial_file: Option<ModelFileRequest>,
}

#[derive(Debug, Serialize)]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub model_type: String,
    pub model_path: Option<String>,
    pub description: Option<String>,
    pub default_backend: Option<String>,
    pub config_json: Option<String>,
    pub params_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub file_status: String,
    pub total_file_count: i64,
    pub verified_file_count: i64,
    pub available_node_count: i64,
    pub last_file_verified_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ModelListResponse {
    pub models: Vec<ModelView>,
}

#[derive(Debug, Deserialize)]
pub struct ModelFileRequest {
    pub node_id: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct ModelFileView {
    pub id: String,
    pub model_id: String,
    pub model_name: Option<String>,
    pub node_id: String,
    pub node_name: Option<String>,
    pub node_status: String,
    pub path: String,
    pub path_type: Option<String>,
    pub status: String,
    pub size_bytes: Option<i64>,
    pub last_verified_at: Option<i64>,
    pub last_error: Option<String>,
    pub verify_task_id: Option<String>,
    pub verify_task_status: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct ModelFileListResponse {
    pub files: Vec<ModelFileView>,
}

#[derive(Debug, Deserialize)]
pub struct AgentTaskPollRequest {
    pub node_id: String,
    pub current_config_version: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AgentTaskPollResponse {
    pub task: Option<AgentTaskView>,
    pub agent_config: AgentConfig,
}

#[derive(Debug, Serialize)]
pub struct AgentTaskView {
    pub id: String,
    pub node_id: String,
    pub kind: String,
    pub status: String,
    pub payload: serde_json::Value,
    pub lease_until: Option<i64>,
    pub attempt_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct AgentTaskResultRequest {
    pub node_id: String,
    pub status: String,
    pub result: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct ModelInstanceCreateRequest {
    pub model_id: Option<String>,
    pub model_file_id: Option<String>,
    pub node_id: Option<String>,
    pub runtime_environment_id: Option<String>,
    pub name: String,
    pub deploy_type: Option<String>,
    pub backend: Option<String>,
    pub base_url: Option<String>,
    pub endpoint_url: Option<String>,
    pub health_url: Option<String>,
    pub runtime_version: Option<String>,
    pub model_name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub params_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModelInstanceUpdateRequest {
    pub name: Option<String>,
    pub backend: Option<String>,
    pub base_url: Option<String>,
    pub endpoint_url: Option<String>,
    pub health_url: Option<String>,
    pub runtime_version: Option<String>,
    pub model_name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub params_json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelInstanceView {
    pub id: String,
    pub model_id: Option<String>,
    pub model_file_id: Option<String>,
    pub model_definition_name: Option<String>,
    pub model_file_path: Option<String>,
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    pub node_online: bool,
    pub last_heartbeat_at: Option<i64>,
    pub runtime_environment_id: Option<String>,
    pub runtime_environment_name: Option<String>,
    pub name: String,
    pub backend: String,
    pub deploy_type: String,
    pub status: String,
    pub base_url: Option<String>,
    pub endpoint_url: Option<String>,
    pub health_url: Option<String>,
    pub runtime_version: Option<String>,
    pub model_name: Option<String>,
    pub description: Option<String>,
    pub params_json: Option<String>,
    pub process_id: Option<i64>,
    pub process_ref: Option<String>,
    pub log_tail: Option<String>,
    pub command: Option<String>,
    pub last_checked_at: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct ModelInstanceListResponse {
    pub model_instances: Vec<ModelInstanceView>,
}

#[derive(Debug, Deserialize)]
pub struct ModelFileTrashRequest {
    pub reason: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelFileTrashView {
    pub id: String,
    pub model_file_id: Option<String>,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    pub path: String,
    pub reason: Option<String>,
    pub status: String,
    pub file_deleted_at: Option<i64>,
    pub cleanup_task_id: Option<String>,
    pub last_error: Option<String>,
    pub note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct ModelFileTrashListResponse {
    pub items: Vec<ModelFileTrashView>,
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub source_type: Option<String>,
    pub node_id: Option<String>,
    pub instance_id: Option<String>,
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct LogResponse {
    pub source_type: String,
    pub node_id: Option<String>,
    pub instance_id: Option<String>,
    pub content: String,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub operation_type: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub node_id: Option<String>,
    pub instance_id: Option<String>,
    pub actor_type: Option<String>,
    pub result: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct FrontendErrorReport {
    pub message: String,
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub occurred_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AuditEventView {
    pub id: String,
    pub occurred_at: i64,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub actor_group_id: Option<String>,
    pub operation_type: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub node_id: Option<String>,
    pub instance_id: Option<String>,
    pub result: String,
    pub error_message: Option<String>,
    pub source: String,
    pub detail_json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditListResponse {
    pub events: Vec<AuditEventView>,
}
