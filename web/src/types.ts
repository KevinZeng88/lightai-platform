export interface NodeMetrics {
  cpu_usage_percent?: number | null
  memory_total_bytes?: number | null
  memory_used_bytes?: number | null
  disk_total_bytes?: number | null
  disk_used_bytes?: number | null
}

export interface GpuStatus {
  gpu_key: string
  gpu_index?: number | null
  vendor: string
  name: string
  uuid?: string | null
  driver_version?: string | null
  memory_total_bytes?: number | null
  memory_used_bytes?: number | null
  utilization_percent?: number | null
  temperature_celsius?: number | null
  power_watts?: number | null
  collector: string
  updated_at: number
}

export interface NodeStatus {
  id: string
  name: string
  hostname: string
  agent_version?: string | null
  os?: string | null
  arch?: string | null
  status: string
  registered_at: number
  updated_at: number
  last_heartbeat_at?: number | null
  metrics?: NodeMetrics | null
  agent_config?: AgentConfig | null
  effective_agent_config: AgentConfig
  config_sync_status: string
  gpus: GpuStatus[]
}

export interface AgentConfig {
  config_version: number
  heartbeat_interval_secs: number
  metrics_sample_interval_secs: number
  task_poll_interval_secs: number
  config_refresh_interval_secs: number
  command_timeout_secs: number
  environment_check_timeout_secs: number
  allowed_model_dirs: string[]
  nvidia_collector_enabled: boolean
  custom_collector_script?: string | null
  collector_timeout_secs: number
  collector_max_output_bytes: number
  log_dir: string
  log_level: string
  log_max_file_bytes: number
  log_retention_files: number
  log_retention_days: number
  last_config_updated_at?: number | null
}

export interface AgentConfigPolicy {
  heartbeat_interval_secs?: number | null
  metrics_sample_interval_secs?: number | null
  command_timeout_secs?: number | null
  environment_check_timeout_secs?: number | null
  allowed_model_dirs?: string[] | null
  nvidia_collector_enabled?: boolean | null
  custom_collector_script?: string | null
  collector_timeout_secs?: number | null
  collector_max_output_bytes?: number | null
  log_dir?: string | null
  log_level?: string | null
  log_max_file_bytes?: number | null
  log_retention_files?: number | null
  log_retention_days?: number | null
}

export interface AgentConfigPolicyView {
  scope: string
  node_id?: string | null
  version: number
  updated_at: number
  policy: AgentConfigPolicy
  effective_config: AgentConfig
  restart_required_fields: string[]
  online_reload_fields: string[]
}

export interface AgentConfigPoliciesResponse {
  global: AgentConfigPolicyView
  nodes: AgentConfigPolicyView[]
}

export interface NodeMetricSample extends NodeMetrics {
  sampled_at: number
}

export interface GpuMetricSample {
  sampled_at: number
  vendor: string
  memory_total_bytes?: number | null
  memory_used_bytes?: number | null
  utilization_percent?: number | null
  temperature_celsius?: number | null
  power_watts?: number | null
}

export interface MetricSampleResponse<TSample> {
  requested_from: number
  requested_to: number
  actual_from?: number | null
  actual_to?: number | null
  sample_count: number
  samples: TSample[]
}

export interface RuntimeEnvironment {
  id: string
  node_id?: string | null
  name: string
  backend: string
  deploy_type: string
  version?: string | null
  base_url?: string | null
  health_url?: string | null
  endpoint_url?: string | null
  binary_path?: string | null
  docker_image?: string | null
  working_dir?: string | null
  log_dir?: string | null
  allowed_model_dirs_json?: string | null
  config_json?: string | null
  enabled: boolean
  last_checked_at?: number | null
  check_status?: string | null
  check_message?: string | null
  created_at: number
  updated_at: number
}

export interface ModelDefinition {
  id: string
  name: string
  display_name?: string | null
  model_type: string
  model_path?: string | null
  description?: string | null
  default_backend?: string | null
  config_json?: string | null
  created_at: number
  updated_at: number
  deleted_at?: number | null
  file_status: string
  total_file_count: number
  verified_file_count: number
  available_node_count: number
  last_file_verified_at?: number | null
}

export interface ModelFile {
  id: string
  model_id: string
  model_name?: string | null
  node_id: string
  node_name?: string | null
  node_status: string
  path: string
  path_type?: string | null
  status: string
  size_bytes?: number | null
  last_verified_at?: number | null
  last_error?: string | null
  verify_task_id?: string | null
  verify_task_status?: string | null
  created_at: number
  updated_at: number
}

export interface ModelInstance {
  id: string
  model_id?: string | null
  model_file_id?: string | null
  model_definition_name?: string | null
  model_file_path?: string | null
  model_name?: string | null
  node_id?: string | null
  node_name?: string | null
  node_online?: boolean
  last_heartbeat_at?: number | null
  runtime_environment_id?: string | null
  runtime_environment_name?: string | null
  name: string
  backend: string
  deploy_type: string
  status: string
  base_url?: string | null
  endpoint_url?: string | null
  health_url?: string | null
  runtime_version?: string | null
  description?: string | null
  params_json?: string | null
  process_id?: number | null
  process_ref?: string | null
  log_tail?: string | null
  command?: string | null
  last_checked_at?: number | null
  last_error?: string | null
  created_at: number
  updated_at: number
}

export interface ModelFileTrashItem {
  id: string
  model_file_id?: string | null
  model_id?: string | null
  model_name?: string | null
  node_id?: string | null
  node_name?: string | null
  path: string
  reason?: string | null
  status: string
  file_deleted_at?: number | null
  cleanup_task_id?: string | null
  last_error?: string | null
  note?: string | null
  created_at: number
  updated_at: number
}

export interface LogResponse {
  source_type: string
  node_id?: string | null
  instance_id?: string | null
  content: string
  message?: string | null
}

export interface LogPolicy {
  log_dir: string
  log_level: string
  log_max_file_bytes: number
  log_retention_files: number
  log_retention_days: number
}

export interface AuditEvent {
  id: string
  occurred_at: number
  actor_type: string
  actor_id?: string | null
  actor_group_id?: string | null
  operation_type: string
  target_type: string
  target_id?: string | null
  node_id?: string | null
  instance_id?: string | null
  result: string
  error_message?: string | null
  source: string
  detail_json?: string | null
}
