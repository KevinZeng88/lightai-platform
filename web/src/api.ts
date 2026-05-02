import type {
  AgentConfigPoliciesResponse,
  AgentConfigPolicy,
  AgentConfigPolicyView,
  AuditEvent,
  GpuMetricSample,
  MetricSampleResponse,
  ModelDefinition,
  ModelFile,
  ModelFileTrashItem,
  ModelInstance,
  LogResponse,
  LogPolicy,
  NodeMetricSample,
  NodeStatus,
  RuntimeEnvironment
} from './types'

async function readJson<T>(response: Response, fallback: string): Promise<T> {
  if (!response.ok) {
    let message = fallback
    try {
      const payload = await response.json()
      message = payload.message ?? payload.error ?? message
    } catch {
      message = `${fallback}: ${response.status}`
    }
    throw new Error(message)
  }
  return response.json()
}

async function sendJson<T>(url: string, method: string, body?: unknown): Promise<T> {
  const response = await fetch(url, {
    method,
    headers: body == null ? undefined : { 'Content-Type': 'application/json' },
    body: body == null ? undefined : JSON.stringify(body)
  })
  return readJson<T>(response, `${method} ${url} failed`)
}

async function sendEmpty(url: string, method: string): Promise<void> {
  const response = await fetch(url, { method })
  if (!response.ok) {
    let message = `${method} ${url} failed: ${response.status}`
    try {
      const payload = await response.json()
      message = payload.message ?? payload.error ?? message
    } catch {
      // Keep status-only message.
    }
    throw new Error(message)
  }
}

export async function fetchNodes(): Promise<NodeStatus[]> {
  const response = await fetch('/api/nodes')
  if (!response.ok) {
    throw new Error(`Failed to fetch nodes: ${response.status}`)
  }
  const payload = await response.json()
  return payload.nodes
}

export async function fetchAgentConfigPolicies(): Promise<AgentConfigPoliciesResponse> {
  return sendJson('/api/config/agent', 'GET')
}

export async function updateGlobalAgentConfigPolicy(
  payload: AgentConfigPolicy
): Promise<AgentConfigPolicyView> {
  return sendJson('/api/config/agent/global', 'PUT', payload)
}

export async function updateNodeAgentConfigPolicy(
  nodeId: string,
  payload: AgentConfigPolicy
): Promise<AgentConfigPolicyView> {
  return sendJson(`/api/nodes/${nodeId}/config`, 'PUT', payload)
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

export async function fetchRuntimeEnvironments(): Promise<RuntimeEnvironment[]> {
  const payload = await sendJson<{ runtime_environments: RuntimeEnvironment[] }>(
    '/api/runtime-environments',
    'GET'
  )
  return payload.runtime_environments
}

export async function createRuntimeEnvironment(
  nodeId: string,
  payload: Partial<RuntimeEnvironment>
): Promise<RuntimeEnvironment> {
  return sendJson(`/api/nodes/${nodeId}/runtime-environments`, 'POST', payload)
}

export async function updateRuntimeEnvironment(
  id: string,
  payload: Partial<RuntimeEnvironment>
): Promise<RuntimeEnvironment> {
  return sendJson(`/api/runtime-environments/${id}`, 'PUT', payload)
}

export async function deleteRuntimeEnvironment(id: string): Promise<void> {
  await sendEmpty(`/api/runtime-environments/${id}`, 'DELETE')
}

export async function checkRuntimeEnvironment(id: string): Promise<RuntimeEnvironment> {
  return sendJson(`/api/runtime-environments/${id}/check`, 'POST')
}

export async function fetchModels(): Promise<ModelDefinition[]> {
  const payload = await sendJson<{ models: ModelDefinition[] }>('/api/models', 'GET')
  return payload.models
}

export async function createModel(payload: Partial<ModelDefinition> & {
  initial_file?: {
    node_id: string
    path: string
  }
}): Promise<ModelDefinition> {
  return sendJson('/api/models', 'POST', payload)
}

export async function updateModel(
  id: string,
  payload: Partial<ModelDefinition>
): Promise<ModelDefinition> {
  return sendJson(`/api/models/${id}`, 'PUT', payload)
}

export async function deleteModel(id: string): Promise<void> {
  await sendEmpty(`/api/models/${id}`, 'DELETE')
}

export async function fetchModelFiles(modelId: string): Promise<ModelFile[]> {
  const payload = await sendJson<{ files: ModelFile[] }>(`/api/models/${modelId}/files`, 'GET')
  return payload.files
}

export async function createModelFile(
  modelId: string,
  payload: {
    node_id: string
    path: string
  }
): Promise<ModelFile> {
  return sendJson(`/api/models/${modelId}/files`, 'POST', payload)
}

export async function updateModelFile(
  id: string,
  payload: {
    node_id: string
    path: string
  }
): Promise<ModelFile> {
  return sendJson(`/api/model-files/${id}`, 'PUT', payload)
}

export async function deleteModelFile(id: string): Promise<void> {
  await sendEmpty(`/api/model-files/${id}`, 'DELETE')
}

export async function verifyModelFile(id: string): Promise<ModelFile> {
  return sendJson(`/api/model-files/${id}/verify`, 'POST')
}

export async function fetchModelInstances(): Promise<ModelInstance[]> {
  const payload = await sendJson<{ model_instances: ModelInstance[] }>(
    '/api/model-instances',
    'GET'
  )
  return payload.model_instances
}

export async function createModelInstance(payload: {
  model_id?: string | null
  model_file_id?: string | null
  node_id?: string | null
  runtime_environment_id?: string | null
  name: string
  deploy_type?: string | null
  backend?: string | null
  base_url?: string | null
  endpoint_url?: string | null
  health_url?: string | null
  runtime_version?: string | null
  model_name?: string | null
  description?: string | null
  status?: string | null
  params_json?: string | null
}): Promise<ModelInstance> {
  return sendJson('/api/model-instances', 'POST', payload)
}

export async function updateModelInstance(
  id: string,
  payload: {
    name?: string | null
    deploy_type?: string | null
    model_file_id?: string | null
    node_id?: string | null
    runtime_environment_id?: string | null
    backend?: string | null
    base_url?: string | null
    endpoint_url?: string | null
    health_url?: string | null
    runtime_version?: string | null
    model_name?: string | null
    description?: string | null
    status?: string | null
    params_json?: string | null
  }
): Promise<ModelInstance> {
  return sendJson(`/api/model-instances/${id}`, 'PUT', payload)
}

export async function deleteModelInstance(id: string): Promise<void> {
  await sendEmpty(`/api/model-instances/${id}`, 'DELETE')
}

export async function checkModelInstance(id: string): Promise<ModelInstance> {
  return sendJson(`/api/model-instances/${id}/check`, 'POST')
}

export async function startModelInstance(id: string): Promise<ModelInstance> {
  return sendJson(`/api/model-instances/${id}/start`, 'POST')
}

export async function stopModelInstance(id: string): Promise<ModelInstance> {
  return sendJson(`/api/model-instances/${id}/stop`, 'POST')
}

export async function testModelInstance(id: string): Promise<ModelInstance> {
  return sendJson(`/api/model-instances/${id}/test`, 'POST')
}

export async function fetchModelFileTrash(): Promise<ModelFileTrashItem[]> {
  const payload = await sendJson<{ items: ModelFileTrashItem[] }>('/api/model-file-trash', 'GET')
  return payload.items
}

export async function addModelFileTrash(
  modelFileId: string,
  payload: {
    reason?: string | null
    note?: string | null
  }
): Promise<ModelFileTrashItem> {
  return sendJson(`/api/model-files/${modelFileId}/trash`, 'POST', payload)
}

export async function cleanupModelFileTrash(id: string): Promise<ModelFileTrashItem> {
  return sendJson(`/api/model-file-trash/${id}/cleanup`, 'POST')
}

export async function deleteModelFileTrash(id: string): Promise<void> {
  await sendEmpty(`/api/model-file-trash/${id}`, 'DELETE')
}

export async function fetchGpuMetrics(
  nodeId: string,
  gpuKey: string,
  from: number,
  to: number
): Promise<MetricSampleResponse<GpuMetricSample>> {
  const url = gpuMetricsUrl(nodeId, gpuKey, from, to)
  const response = await fetch(
    url
  )
  if (!response.ok) {
    throw new Error(`Failed to fetch GPU metrics: ${response.status}`)
  }
  const payload = await response.json()
  return payload
}

export function gpuMetricsUrl(nodeId: string, gpuKey: string, from: number, to: number) {
  return `/api/nodes/${nodeId}/gpu-metrics?gpu_key=${encodeURIComponent(gpuKey)}&from=${from}&to=${to}`
}

export async function fetchLogs(params: {
  source_type: string
  node_id?: string | null
  instance_id?: string | null
  max_bytes?: number
}): Promise<LogResponse> {
  const search = new URLSearchParams()
  search.set('source_type', params.source_type)
  if (params.node_id) search.set('node_id', params.node_id)
  if (params.instance_id) search.set('instance_id', params.instance_id)
  if (params.max_bytes) search.set('max_bytes', String(params.max_bytes))
  return sendJson(`/api/logs?${search.toString()}`, 'GET')
}

export async function fetchAuditEvents(params: {
  operation_type?: string
  target_type?: string
  node_id?: string
  instance_id?: string
  result?: string
}): Promise<AuditEvent[]> {
  const search = new URLSearchParams()
  Object.entries(params).forEach(([key, value]) => {
    if (value) search.set(key, value)
  })
  const payload = await sendJson<{ events: AuditEvent[] }>(
    `/api/audit-events?${search.toString()}`,
    'GET'
  )
  return payload.events
}

export async function fetchServerLogPolicy(): Promise<LogPolicy> {
  return sendJson('/api/config/server-logs', 'GET')
}

export async function updateServerLogPolicy(payload: LogPolicy): Promise<LogPolicy> {
  return sendJson('/api/config/server-logs', 'PUT', payload)
}
