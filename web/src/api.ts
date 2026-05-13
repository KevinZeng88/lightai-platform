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

const isFrontendErrorUrl = (url: string) => url.includes('/api/frontend-errors')

export type Role = 'admin' | 'operator' | 'viewer'

export interface AuthUser {
  id: string
  username: string
  role: Role
  effective_role: Role
  enabled: boolean
  must_change_password: boolean
}

export interface UserGroup {
  id: string
  name: string
  role: Role
  enabled: boolean
  member_count: number
  members: AuthUser[]
}

function jsonHeaders(extra?: HeadersInit): HeadersInit {
  const headers: Record<string, string> = {}
  if (extra instanceof Headers) {
    extra.forEach((value, key) => {
      headers[key] = value
    })
  } else if (Array.isArray(extra)) {
    for (const [key, value] of extra) headers[key] = value
  } else if (extra) {
    Object.assign(headers, extra)
  }
  return headers
}

async function readJson<T>(response: Response, fallback: string): Promise<T> {
  if (!response.ok) {
    let message = fallback
    const isAuthCheck = response.status === 401
    if (isAuthCheck) {
      // /api/auth/login 401 = wrong credentials; other 401 = session expired.
      message = response.url.includes('/api/auth/login')
        ? '用户名或密码错误'
        : '登录已过期或未登录，请重新登录'
    } else if (response.status === 403) {
      message = '当前用户没有权限执行该操作'
    }
    try {
      const payload = await response.json()
      if (!isAuthCheck) {
        message = payload.message ?? payload.error ?? message
      }
    } catch {
      message = `${fallback}: ${response.status}`
    }
    const apiError = new Error(message)
    // Only report non-401 errors to frontend-errors (401 is normal for unauthenticated state).
    if (!isAuthCheck && !isFrontendErrorUrl(response.url)) {
      fetch('/api/frontend-errors', {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          message: `API 请求失败：${message}`,
          url: response.url,
          occurred_at: Math.floor(Date.now() / 1000)
        })
      }).catch(() => {})
    }
    throw apiError
  }
  return response.json()
}

async function sendJson<T>(url: string, method: string, body?: unknown): Promise<T> {
  const response = await fetch(url, {
    method,
    credentials: 'include',
    headers: jsonHeaders(body == null ? undefined : { 'Content-Type': 'application/json' }),
    body: body == null ? undefined : JSON.stringify(body)
  })
  return readJson<T>(response, `${method} ${url} failed`)
}

async function sendEmpty(url: string, method: string): Promise<void> {
  const response = await fetch(url, { method, credentials: 'include', headers: jsonHeaders() })
  if (!response.ok) {
    let message = `${method} ${url} failed: ${response.status}`
    const isAuthCheck = response.status === 401
    if (isAuthCheck) {
      message = response.url.includes('/api/auth/login')
        ? '用户名或密码错误'
        : '登录已过期或未登录，请重新登录'
    } else if (response.status === 403) {
      message = '当前用户没有权限执行该操作'
    }
    try {
      const payload = await response.json()
      if (!isAuthCheck) {
        message = payload.message ?? payload.error ?? message
      }
    } catch {
      // Keep status-only message.
    }
    const apiError = new Error(message)
    if (!isAuthCheck && !isFrontendErrorUrl(response.url)) {
      fetch('/api/frontend-errors', {
        method: 'POST',
        credentials: 'include',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          message: `API 请求失败：${message}`,
          url: response.url,
          occurred_at: Math.floor(Date.now() / 1000)
        })
      }).catch(() => {})
    }
    throw apiError
  }
}

export async function login(username: string, password: string): Promise<AuthUser> {
  const payload = await sendJson<{ user: AuthUser }>('/api/auth/login', 'POST', {
    username,
    password
  })
  return payload.user
}

export async function logout(): Promise<void> {
  await sendJson('/api/auth/logout', 'POST')
}

export async function fetchCurrentUser(): Promise<AuthUser> {
  const payload = await sendJson<{ user: AuthUser }>('/api/auth/me', 'GET')
  return payload.user
}

export async function fetchSetupStatus(): Promise<boolean> {
  const payload = await sendJson<{ setup_required: boolean }>('/api/setup/status', 'GET')
  return payload.setup_required
}

export interface SecurityStatus {
  ca_fingerprint: string | null
  ca_download_url: string
  setup_required: boolean
  note: string
}

export async function fetchSecurityStatus(): Promise<SecurityStatus> {
  return sendJson<SecurityStatus>('/api/security/status', 'GET')
}

export async function setupAdmin(username: string, password: string, setupToken: string): Promise<AuthUser> {
  const payload = await sendJson<{ user: AuthUser }>('/api/setup/admin', 'POST', {
    username,
    password,
    setup_token: setupToken
  })
  return payload.user
}

export async function changePassword(
  currentPassword: string,
  newPassword: string
): Promise<void> {
  await sendJson('/api/auth/change-password', 'POST', {
    current_password: currentPassword,
    new_password: newPassword
  })
}

export async function fetchUsers(): Promise<AuthUser[]> {
  const payload = await sendJson<{ users: AuthUser[] }>('/api/users', 'GET')
  return payload.users
}

export async function createUser(payload: {
  username: string
  password: string
  role: Role
}): Promise<AuthUser> {
  const response = await sendJson<{ user: AuthUser }>('/api/users', 'POST', payload)
  return response.user
}

export async function updateUser(
  id: string,
  payload: {
    password?: string
    role?: Role
    enabled?: boolean
  }
): Promise<AuthUser> {
  const response = await sendJson<{ user: AuthUser }>(`/api/users/${id}`, 'PUT', payload)
  return response.user
}

export async function fetchGroups(): Promise<UserGroup[]> {
  const payload = await sendJson<{ groups: UserGroup[] }>('/api/groups', 'GET')
  return payload.groups
}

export async function createGroup(payload: {
  name: string
  role: Role
}): Promise<UserGroup> {
  const response = await sendJson<{ group: UserGroup }>('/api/groups', 'POST', payload)
  return response.group
}

export async function updateGroup(
  id: string,
  payload: {
    name?: string
    role?: Role
    enabled?: boolean
  }
): Promise<UserGroup> {
  const response = await sendJson<{ group: UserGroup }>(`/api/groups/${id}`, 'PUT', payload)
  return response.group
}

export async function updateGroupMembers(id: string, userIds: string[]): Promise<UserGroup> {
  const response = await sendJson<{ group: UserGroup }>(`/api/groups/${id}/members`, 'PUT', {
    user_ids: userIds
  })
  return response.group
}

export async function deleteGroup(id: string): Promise<void> {
  await sendEmpty(`/api/groups/${id}`, 'DELETE')
}

export async function fetchNodes(): Promise<NodeStatus[]> {
  const response = await fetch('/api/nodes', { credentials: 'include', headers: jsonHeaders() })
  const payload = await readJson<{ nodes: NodeStatus[] }>(response, 'Failed to fetch nodes')
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
  const response = await fetch(`/api/nodes/${nodeId}/metrics?from=${from}&to=${to}`, {
    credentials: 'include',
    headers: jsonHeaders()
  })
  const payload = await readJson<MetricSampleResponse<NodeMetricSample>>(
    response,
    'Failed to fetch node metrics'
  )
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

export async function fetchModelInstance(id: string): Promise<ModelInstance> {
  return sendJson<ModelInstance>(`/api/model-instances/${id}`, 'GET')
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

export async function refreshInstanceLogs(id: string): Promise<LogResponse> {
  return sendJson(`/api/model-instances/${id}/logs`, 'POST')
}

export function reportFrontendError(payload: {
  message: string
  stack?: string
  url?: string
  occurred_at?: number
}): void {
  const body = JSON.stringify(payload)
  const maxLen = 4096
  if (body.length > maxLen) {
    payload.stack = payload.stack?.slice(0, 1024)
    payload.message = payload.message.slice(0, 1024)
  }
  fetch('/api/frontend-errors', {
    method: 'POST',
    credentials: 'include',
    headers: jsonHeaders({ 'Content-Type': 'application/json' }),
    body: JSON.stringify(payload)
  }).catch(() => {
    // fire and forget; don't throw if reporting fails
  })
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
  const response = await fetch(url, { credentials: 'include', headers: jsonHeaders() })
  const payload = await readJson<MetricSampleResponse<GpuMetricSample>>(
    response,
    'Failed to fetch GPU metrics'
  )
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
  limit?: number
  offset?: number
}): Promise<AuditEvent[]> {
  const search = new URLSearchParams()
  Object.entries(params).forEach(([key, value]) => {
    if (value !== undefined && value !== null) search.set(key, String(value))
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

// ── Collector registry ──

export interface CollectorRegistryEntry {
  id: string
  vendor: string
  name: string
  version: string
  description: string
  discover_sha256: string
  metrics_sha256: string
  enabled: boolean
  created_at: number
  updated_at: number
}

export async function fetchCollectorRegistry(): Promise<CollectorRegistryEntry[]> {
  const payload = await sendJson<{ collectors: CollectorRegistryEntry[] }>(
    '/api/collector-registry',
    'GET'
  )
  return payload.collectors
}

export async function registerCollector(
  payload: Omit<CollectorRegistryEntry, 'created_at' | 'updated_at'>
): Promise<CollectorRegistryEntry> {
  return sendJson('/api/collector-registry', 'POST', payload)
}

export async function deleteCollector(id: string, version: string): Promise<void> {
  await sendEmpty(`/api/collector-registry/${encodeURIComponent(id)}/${encodeURIComponent(version)}`, 'DELETE')
}

// ── Ollama ──

export interface OllamaModelItem {
  name: string
  size?: number | null
  digest?: string | null
  modified_at?: string | null
}

export async function fetchOllamaModels(nodeId: string, runtimeEnvId: string): Promise<OllamaModelItem[]> {
  const search = new URLSearchParams({ node_id: nodeId, runtime_env_id: runtimeEnvId })
  const payload = await sendJson<{ models: OllamaModelItem[] }>(`/api/ollama/models?${search.toString()}`, 'GET')
  return payload.models
}
