import type { GpuMetricSample, MetricSampleResponse, NodeMetricSample, NodeStatus } from './types'

export async function fetchNodes(): Promise<NodeStatus[]> {
  const response = await fetch('/api/nodes')
  if (!response.ok) {
    throw new Error(`Failed to fetch nodes: ${response.status}`)
  }
  const payload = await response.json()
  return payload.nodes
}

export async function fetchNodeMetrics(
  nodeId: string,
  from: number,
  to: number
): Promise<MetricSampleResponse<NodeMetricSample>> {
  const response = await fetch(`/api/nodes/${nodeId}/metrics?from=${from}&to=${to}`)
  if (!response.ok) {
    throw new Error(`Failed to fetch node metrics: ${response.status}`)
  }
  const payload = await response.json()
  return payload
}

export async function fetchGpuMetrics(
  nodeId: string,
  gpuKey: string,
  from: number,
  to: number
): Promise<MetricSampleResponse<GpuMetricSample>> {
  const response = await fetch(
    `/api/nodes/${nodeId}/gpus/${encodeURIComponent(gpuKey)}/metrics?from=${from}&to=${to}`
  )
  if (!response.ok) {
    throw new Error(`Failed to fetch GPU metrics: ${response.status}`)
  }
  const payload = await response.json()
  return payload
}
