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
  gpus: GpuStatus[]
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
