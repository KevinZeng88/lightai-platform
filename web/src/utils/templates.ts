export const MODEL_TEMPLATES = {
  'huggingface-vllm': {
    label: 'HuggingFace 目录 + vLLM',
    template: {
      path_type: 'directory',
      model_format: 'huggingface',
      supported_backends: ['vllm'],
      served_model_name: ''
    }
  },
  'gguf-llamacpp': {
    label: 'GGUF 文件 + llama.cpp',
    template: {
      path_type: 'file',
      model_format: 'gguf',
      supported_backends: ['llama.cpp'],
      served_model_name: ''
    }
  },
  'ollama': {
    label: 'Ollama 模型 + Ollama',
    template: {
      path_type: 'ollama',
      model_format: 'ollama',
      supported_backends: ['ollama'],
      served_model_name: ''
    }
  },
  'custom': {
    label: 'Custom 自定义',
    template: {
      path_type: 'custom',
      model_format: 'custom',
      supported_backends: ['custom'],
      served_model_name: ''
    }
  }
}

export const RUNTIME_TEMPLATES = {
  'vllm-docker': {
    label: 'vLLM + Docker',
    template: {
      backend: 'vllm',
      deploy_type: 'docker',
      image: 'vllm/vllm-openai:latest',
      gpu: 'all',
      ipc: 'host',
      container_port: 8000,
      cache_host_path: '/data/vllm-cache',
      cache_container_path: '/root/.cache/huggingface',
      defaults: {
        host: '0.0.0.0',
        port: 8000,
        gpu_memory_utilization: 0.5,
        max_model_len: 4096,
        max_num_seqs: 8
      },
      extra_docker_args: [],
      extra_backend_args: []
    }
  },
  'llamacpp-local': {
    label: 'llama.cpp + Local',
    template: {
      backend: 'llama_cpp',
      deploy_type: 'local',
      entrypoint: '/opt/llama.cpp/build/bin/llama-server',
      defaults: {
        host: '0.0.0.0',
        port: 8080,
        ctx_size: 4096,
        n_gpu_layers: -1
      },
      extra_backend_args: []
    }
  },
  'ollama-local': {
    label: 'Ollama + Local',
    template: {
      backend: 'ollama',
      deploy_type: 'local',
      host: '127.0.0.1',
      port: 11434,
      defaults: {},
      extra_backend_args: []
    }
  },
  'custom': {
    label: 'Custom 自定义',
    template: {
      backend: 'custom',
      deploy_type: 'local',
      entrypoint: '',
      defaults: {},
      extra_backend_args: []
    }
  }
}

export function toTemplateJson(template: unknown): string {
  return JSON.stringify(template, null, 2)
}

export function generateDockerInstanceOverrides(
  modelName: string,
  modelHostPath: string,
  modelParams: Record<string, unknown> | null,
  runtimeParams: Record<string, unknown> | null
): Record<string, unknown> {
  const sanitize = (s: string) => s.replace(/[^a-zA-Z0-9_-]/g, '-').toLowerCase()
  const containerName = `lightai-${sanitize(modelName || 'model')}`
  const hostPort = 18000

  const modelDir = modelHostPath.split('/').pop() || 'model'
  const modelContainerPath = `/models/${modelDir}`

  const servedModelName =
    (modelParams as any)?.served_model_name || modelName || ''

  const runtime = (runtimeParams || {}) as Record<string, unknown>
  const defaults = (runtime.defaults || {}) as Record<string, unknown>

  return {
    container_name: containerName,
    host_port: hostPort,
    model_container_path: modelContainerPath,
    served_model_name: servedModelName,
    gpu_memory_utilization: defaults.gpu_memory_utilization ?? 0.5,
    max_model_len: defaults.max_model_len ?? 4096,
    max_num_seqs: defaults.max_num_seqs ?? 8,
    extra_docker_args: [],
    extra_backend_args: []
  }
}

export function checkModelRuntimeCompat(
  modelParams: Record<string, unknown> | null,
  runtimeBackend: string
): { compatible: boolean; warning: string } {
  if (!modelParams) {
    return { compatible: true, warning: '模型元数据不完整，无法确认兼容性。请确认模型与后端兼容。' }
  }
  const format = (modelParams as any).model_format
  const backends = (modelParams as any).supported_backends || []

  if (!format || backends.length === 0) {
    return { compatible: true, warning: '模型元数据不完整，无法确认兼容性。请确认模型与后端兼容。' }
  }

  if (!backends.includes(runtimeBackend)) {
    if (format === 'gguf' && runtimeBackend === 'vllm') {
      return { compatible: false, warning: `GGUF 格式模型不兼容 vLLM 后端。建议使用 llama.cpp。` }
    }
    if (format === 'huggingface' && runtimeBackend === 'llama_cpp') {
      return { compatible: false, warning: `HuggingFace 目录模型默认不兼容 llama.cpp。如已转换为 GGUF 请使用 GGUF 格式。` }
    }
    return { compatible: false, warning: `模型支持后端 [${backends.join(', ')}] 不包含 ${runtimeBackend}，可能不兼容。` }
  }
  return { compatible: true, warning: '' }
}

// ── Extra args line helpers ──

export function linesToArgs(text: string): string[] {
  return text
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
}

export function argsToLines(args: string[]): string {
  if (!args || args.length === 0) return ''
  return args.join('\n')
}

// ── Runtime params_json assemble/parse ──

export interface DockerRuntimeFields {
  image: string
  gpu: string
  ipc: string
  container_port: number
  cache_host_path: string
  cache_container_path: string
  default_host: string
  default_port: number
  gpu_memory_utilization: number
  max_model_len: number
  max_num_seqs: number
  extra_docker_args: string
  extra_backend_args: string
}

export function defaultDockerRuntimeFields(): DockerRuntimeFields {
  return {
    image: 'vllm/vllm-openai:latest',
    gpu: 'all',
    ipc: 'host',
    container_port: 8000,
    cache_host_path: '/data/vllm-cache',
    cache_container_path: '/root/.cache/huggingface',
    default_host: '0.0.0.0',
    default_port: 8000,
    gpu_memory_utilization: 0.5,
    max_model_len: 4096,
    max_num_seqs: 8,
    extra_docker_args: '',
    extra_backend_args: ''
  }
}

export interface RuntimeToggles {
  showGpu: boolean
  showIpc: boolean
  showCache: boolean
  showGpuMem: boolean
  showMaxModelLen: boolean
  showMaxNumSeqs: boolean
  showExtraBackend: boolean
  showExtraDocker: boolean
}

export function assembleDockerRuntimeParams(
  fields: DockerRuntimeFields,
  backend: string,
  toggles?: Partial<RuntimeToggles>,
  extraBackendText?: string,
  extraDockerText?: string,
): string {
  const t = toggles || {}
  const cp = fields.container_port || 8000
  const result: Record<string, unknown> = {
    backend,
    deploy_type: 'docker',
    image: fields.image || 'vllm/vllm-openai:latest',
    container_port: cp,
    gpu: fields.gpu || 'all',
    ipc: fields.ipc || 'host',
    defaults: {
      host: fields.default_host || '0.0.0.0',
      port: cp,
    } as Record<string, unknown>,
  }

  if (t.showCache) {
    ;(result as any).cache_host_path = fields.cache_host_path || ''
    ;(result as any).cache_container_path = fields.cache_container_path || ''
  }
  if (t.showGpuMem) (result.defaults as any).gpu_memory_utilization = fields.gpu_memory_utilization || 0.5
  if (t.showMaxModelLen) (result.defaults as any).max_model_len = fields.max_model_len || 4096
  if (t.showMaxNumSeqs) (result.defaults as any).max_num_seqs = fields.max_num_seqs || 8
  if (t.showExtraBackend)
    (result as any).extra_backend_args = linesToArgs(extraBackendText || '')
  if (t.showExtraDocker)
    (result as any).extra_docker_args = linesToArgs(extraDockerText || '')

  return JSON.stringify(result)
}

export function parseDockerRuntimeParams(paramsJson: string | null | undefined): DockerRuntimeFields {
  const defaults = defaultDockerRuntimeFields()
  if (!paramsJson) return defaults
  try {
    const p = JSON.parse(paramsJson)
    const d = p.defaults || {}
    const container_port = p.container_port || d.port || defaults.container_port
    return {
      image: p.image || defaults.image,
      gpu: p.gpu || defaults.gpu,
      ipc: p.ipc || defaults.ipc,
      container_port,
      cache_host_path: p.cache_host_path || defaults.cache_host_path,
      cache_container_path: p.cache_container_path || defaults.cache_container_path,
      default_host: '0.0.0.0',
      default_port: container_port,
      gpu_memory_utilization: d.gpu_memory_utilization ?? defaults.gpu_memory_utilization,
      max_model_len: d.max_model_len ?? defaults.max_model_len,
      max_num_seqs: d.max_num_seqs ?? defaults.max_num_seqs,
      extra_docker_args: argsToLines(p.extra_docker_args || []),
      extra_backend_args: argsToLines(p.extra_backend_args || [])
    }
  } catch {
    return defaults
  }
}

// ── Model params_json assemble/parse ──

export interface ModelMetaFields {
  path_type: string
  model_format: string
  supported_backends: string[]
  served_model_name: string
  extra_backend_args: string
}

export function defaultModelMeta(): ModelMetaFields {
  return {
    path_type: 'directory',
    model_format: 'huggingface',
    supported_backends: ['vllm'],
    served_model_name: '',
    extra_backend_args: ''
  }
}

export function assembleModelMeta(fields: ModelMetaFields): string {
  return JSON.stringify({
    path_type: fields.path_type || 'directory',
    model_format: fields.model_format || 'huggingface',
    supported_backends: fields.supported_backends || [],
    served_model_name: fields.served_model_name || '',
    extra_backend_args: linesToArgs(fields.extra_backend_args || '')
  })
}

export function parseModelMeta(paramsJson: string | null | undefined): ModelMetaFields {
  const defaults = defaultModelMeta()
  if (!paramsJson) return defaults
  try {
    const p = JSON.parse(paramsJson)
    return {
      path_type: p.path_type || defaults.path_type,
      model_format: p.model_format || defaults.model_format,
      supported_backends: Array.isArray(p.supported_backends) ? p.supported_backends : defaults.supported_backends,
      served_model_name: p.served_model_name || defaults.served_model_name,
      extra_backend_args: argsToLines(p.extra_backend_args || [])
    }
  } catch {
    return defaults
  }
}
