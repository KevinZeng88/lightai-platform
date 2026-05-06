interface InstanceForm {
  model_id: string
  model_file_id: string
  node_id: string
  runtime_environment_id: string
  name: string
  deploy_type: string
  backend: string
  model_name: string
  runtime_version: string
  base_url: string
  endpoint_url: string
  health_url: string
  description: string
  host: string
  port: number
  ctx_size: number
  gpu_layers: number
  threads: number
  extra_args_text: string
  params_json: string
  container_name: string
  host_port: number
  container_port: number
  model_container_path: string
  served_model_name: string
  gpu_memory_utilization: number
  max_model_len: number
  max_num_seqs: number
  docker_gpu: string
  extra_docker_args_text: string
  extra_backend_args_text: string
  probe_paths_text: string
  probe_max_attempts: number
  probe_interval_ms: number
  probe_timeout_ms: number
}

export type { InstanceForm }

export interface DockerRuntimeDefaults {
  container_port: number
  gpu: string
  gpu_memory_utilization: number
  max_model_len: number
  max_num_seqs: number
  extra_docker_args: string[]
  extra_backend_args: string[]
}

export function parseRuntimeDefaults(runtimeParamsJson?: string | null): DockerRuntimeDefaults {
  const d = {
    container_port: 8000,
    gpu: 'all',
    gpu_memory_utilization: 0.5,
    max_model_len: 4096,
    max_num_seqs: 8,
    extra_docker_args: [] as string[],
    extra_backend_args: [] as string[],
  }
  if (!runtimeParamsJson) return d
  try {
    const p = JSON.parse(runtimeParamsJson)
    if (typeof p.container_port === 'number') d.container_port = p.container_port
    if (typeof p.gpu === 'string') d.gpu = p.gpu
    if (p.defaults?.gpu_memory_utilization != null) d.gpu_memory_utilization = p.defaults.gpu_memory_utilization
    if (p.defaults?.max_model_len != null) d.max_model_len = p.defaults.max_model_len
    if (p.defaults?.max_num_seqs != null) d.max_num_seqs = p.defaults.max_num_seqs
    if (Array.isArray(p.extra_docker_args)) d.extra_docker_args = p.extra_docker_args
    if (Array.isArray(p.extra_backend_args)) d.extra_backend_args = p.extra_backend_args
  } catch { /* use defaults */ }
  return d
}

export function detectOverrides(instanceParamsJson?: string | null): Set<string> {
  const overrides = new Set<string>()
  if (!instanceParamsJson) return overrides
  try {
    const p = JSON.parse(instanceParamsJson)
    if (p.gpu !== undefined) overrides.add('gpu')
    if (p.gpu_memory_utilization !== undefined) overrides.add('gpu_memory_utilization')
    if (p.max_model_len !== undefined) overrides.add('max_model_len')
    if (p.max_num_seqs !== undefined) overrides.add('max_num_seqs')
    if (Array.isArray(p.extra_docker_args) && p.extra_docker_args.length > 0) overrides.add('extra_docker_args')
    if (Array.isArray(p.extra_backend_args) && p.extra_backend_args.length > 0) overrides.add('extra_backend_args')
    if (typeof p.container_port === 'number') overrides.add('container_port')
  } catch { /* empty */ }
  return overrides
}

export function emptyForm(): InstanceForm {
  return {
    model_id: '',
    model_file_id: '',
    node_id: '',
    runtime_environment_id: '',
    name: '',
    deploy_type: 'external',
    backend: '',
    model_name: '',
    runtime_version: '',
    base_url: '',
    endpoint_url: '',
    health_url: '',
    description: '',
    host: '127.0.0.1',
    port: 8080,
    ctx_size: 4096,
    gpu_layers: 0,
    threads: 0,
    extra_args_text: '',
    params_json: '',
    container_name: '',
    host_port: 18000,
    container_port: 8000,
    model_container_path: '',
    served_model_name: '',
    gpu_memory_utilization: 0.5,
    max_model_len: 4096,
    max_num_seqs: 8,
    docker_gpu: 'all',
    extra_docker_args_text: '',
    extra_backend_args_text: '',
    probe_paths_text: '',
    probe_max_attempts: 5,
    probe_interval_ms: 5000,
    probe_timeout_ms: 400
  }
}

export function localParams(form: InstanceForm) {
  const probePaths = form.probe_paths_text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
  return {
    host: form.host.trim() || '127.0.0.1',
    port: form.port,
    ctx_size: form.ctx_size || undefined,
    gpu_layers: form.gpu_layers,
    threads: form.threads || undefined,
    extra_args: form.extra_args_text
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean),
    ...(probePaths.length > 0 ? { probe_paths: probePaths } : {}),
    ...(form.probe_max_attempts !== 5 ? { probe_max_attempts: form.probe_max_attempts } : {}),
    ...(form.probe_interval_ms !== 5000 ? { probe_interval_ms: form.probe_interval_ms } : {}),
    ...(form.probe_timeout_ms !== 400 ? { probe_timeout_ms: form.probe_timeout_ms } : {})
  }
}

/** Build Docker instance params JSON. Only saves overridden fields + mandatory instance fields. */
export function buildDockerInstanceParams(
  form: InstanceForm,
  overrides: Set<string>,
): Record<string, unknown> {
  const result: Record<string, unknown> = {}

  // Always-included instance fields
  if (form.container_name.trim()) result.container_name = form.container_name.trim()
  if (form.host_port) result.host_port = form.host_port
  if (form.model_container_path.trim()) result.model_container_path = form.model_container_path.trim()
  if (form.served_model_name.trim()) result.served_model_name = form.served_model_name.trim()

  // Override-only fields (only saved if explicitly overridden)
  if (overrides.has('gpu') && form.docker_gpu.trim()) result.gpu = form.docker_gpu.trim()
  if (overrides.has('gpu_memory_utilization')) result.gpu_memory_utilization = form.gpu_memory_utilization
  if (overrides.has('max_model_len')) result.max_model_len = form.max_model_len
  if (overrides.has('max_num_seqs')) result.max_num_seqs = form.max_num_seqs
  if (overrides.has('container_port')) result.container_port = form.container_port

  if (overrides.has('extra_docker_args')) {
    const args = form.extra_docker_args_text
      .split('\n')
      .map((line: string) => line.trim())
      .filter(Boolean)
    if (args.length > 0) result.extra_docker_args = args
  }

  if (overrides.has('extra_backend_args')) {
    const args = form.extra_backend_args_text
      .split('\n')
      .map((line: string) => line.trim())
      .filter(Boolean)
    if (args.length > 0) result.extra_backend_args = args
  }

  return result
}

export function paramsJsonFromOverrides(overrides: Record<string, unknown>): string {
  const obj = { ...overrides }
  // Remove empty/undefined values
  for (const key of Object.keys(obj)) {
    if (obj[key] === undefined || obj[key] === null) delete obj[key]
  }
  return JSON.stringify(obj)
}

export function parseParams(value?: string | null) {
  try {
    const parsed = value ? JSON.parse(value) : {}
    return {
      host: typeof parsed.host === 'string' ? parsed.host : '127.0.0.1',
      port: typeof parsed.port === 'number' ? parsed.port : 8080,
      ctx_size: typeof parsed.ctx_size === 'number' ? parsed.ctx_size : 4096,
      gpu_layers: typeof parsed.gpu_layers === 'number' ? parsed.gpu_layers : 0,
      threads: typeof parsed.threads === 'number' ? parsed.threads : 0,
      extra_args: Array.isArray(parsed.extra_args) ? parsed.extra_args.filter((item: unknown) => typeof item === 'string') : [],
      container_name: typeof parsed.container_name === 'string' ? parsed.container_name : '',
      host_port: typeof parsed.host_port === 'number' ? parsed.host_port : 18000,
      container_port: typeof parsed.container_port === 'number' ? parsed.container_port : 8000,
      model_container_path: typeof parsed.model_container_path === 'string' ? parsed.model_container_path : '',
      served_model_name: typeof parsed.served_model_name === 'string' ? parsed.served_model_name : '',
      gpu_memory_utilization: typeof parsed.gpu_memory_utilization === 'number' ? parsed.gpu_memory_utilization : 0.5,
      max_model_len: typeof parsed.max_model_len === 'number' ? parsed.max_model_len : 4096,
      max_num_seqs: typeof parsed.max_num_seqs === 'number' ? parsed.max_num_seqs : 8,
      docker_gpu: typeof parsed.gpu === 'string' ? parsed.gpu : 'all',
      extra_docker_args: Array.isArray(parsed.extra_docker_args) ? parsed.extra_docker_args.filter((item: unknown) => typeof item === 'string') : [],
      extra_backend_args_text: Array.isArray(parsed.extra_backend_args) ? parsed.extra_backend_args.filter((p: unknown) => typeof p === 'string').join('\n') : '',
      extra_docker_args_text: Array.isArray(parsed.extra_docker_args) ? parsed.extra_docker_args.filter((p: unknown) => typeof p === 'string').join('\n') : '',
      probe_paths_text: Array.isArray(parsed.probe_paths) ? parsed.probe_paths.filter((p: unknown) => typeof p === 'string').join('\n') : '',
      probe_max_attempts: typeof parsed.probe_max_attempts === 'number' ? parsed.probe_max_attempts : 5,
      probe_interval_ms: typeof parsed.probe_interval_ms === 'number' ? parsed.probe_interval_ms : 5000,
      probe_timeout_ms: typeof parsed.probe_timeout_ms === 'number' ? parsed.probe_timeout_ms : 400
    }
  } catch {
    return {
      host: '127.0.0.1',
      port: 8080,
      ctx_size: 4096,
      gpu_layers: 0,
      threads: 0,
      extra_args: [] as string[],
      container_name: '',
      host_port: 18000,
      container_port: 8000,
      model_container_path: '',
      served_model_name: '',
      gpu_memory_utilization: 0.5,
      max_model_len: 4096,
      max_num_seqs: 8,
      docker_gpu: 'all',
      extra_docker_args: [] as string[],
      extra_backend_args_text: '',
      extra_docker_args_text: '',
      probe_paths_text: '',
      probe_max_attempts: 5,
      probe_interval_ms: 5000,
      probe_timeout_ms: 400
    }
  }
}
