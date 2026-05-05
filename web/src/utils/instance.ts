import type { ModelInstance } from '../types'

export function statusType(row: ModelInstance) {
  if (row.status === 'running') {
    if (checkFailedReason(row.last_error)) return 'warning'
    return 'success'
  }
  if (row.status === 'failed') return 'danger'
  if (row.status === 'pending' || row.status === 'starting') return 'warning'
  return 'info'
}

export function statusLabel(status: string) {
  const labels: Record<string, string> = {
    pending: '待处理',
    starting: '启动中',
    running: '运行中',
    stopping: '停止中',
    stopped: '已停止',
    failed: '失败',
    unknown: '未知'
  }
  return labels[status] ?? status
}

export function deployTypeLabel(value: string) {
  if (value === 'external') return '外部服务'
  if (value === 'local') return '本地实例'
  return value
}

export function runtimeDeployTypeLabel(value: string) {
  if (value === 'binary') return '程序'
  if (value === 'script') return '脚本'
  if (value === 'docker') return '容器'
  return value
}

export function backendLabel(value: string) {
  const labels: Record<string, string> = {
    ollama: 'Ollama',
    llama_cpp: 'llama.cpp',
    vllm: 'vLLM',
    custom: '自定义'
  }
  return labels[value] ?? value
}

export function checkFailedReason(error?: string | null): boolean {
  if (!error) return false
  return ['离线', '无法检查', '不可用', '超时', '失败'].some((kw) => error.includes(kw))
}

export function formatTime(value?: number | null) {
  if (!value) return '-'
  return new Date(value * 1000).toLocaleString()
}

export function emptyToNull(value: string) {
  return value.trim() ? value.trim() : null
}
