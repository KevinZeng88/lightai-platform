<template>
  <section class="panel-header">
    <div>
      <h2>运行环境</h2>
      <p>运行环境表示某节点具备哪些本地运行能力；External 服务请在"实例"中直接接入。</p>
    </div>
    <div class="toolbar compact">
      <el-button :loading="loading" @click="loadData">刷新</el-button>
      <el-button type="primary" @click="openCreate">新增环境</el-button>
    </div>
  </section>

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-table :data="environments" row-key="id" border>
    <el-table-column prop="name" label="名称" min-width="150" fixed="left" />
    <el-table-column label="检查状态" width="130">
      <template #default="{ row }">
        <el-tag :type="checkType(row.check_status)">{{ checkLabel(row.check_status) }}</el-tag>
      </template>
    </el-table-column>
    <el-table-column label="检查信息" min-width="240">
      <template #default="{ row }">
        <span>{{ row.check_message ?? '-' }}</span>
      </template>
    </el-table-column>
    <el-table-column label="节点" min-width="150">
      <template #default="{ row }">{{ nodeName(row.node_id) }}</template>
    </el-table-column>
    <el-table-column label="后端" width="120">
      <template #default="{ row }">{{ backendLabel(row.backend) }}</template>
    </el-table-column>
    <el-table-column label="运行方式" width="120">
      <template #default="{ row }">{{ deployTypeLabel(row.deploy_type) }}</template>
    </el-table-column>
    <el-table-column prop="version" label="版本" width="120" />
    <el-table-column label="日志目录" min-width="220" show-overflow-tooltip>
      <template #default="{ row }">{{ row.log_dir ?? '未配置' }}</template>
    </el-table-column>
    <el-table-column label="状态" width="120">
      <template #default="{ row }">
        <el-tag :type="row.enabled ? 'success' : 'info'">
          {{ row.enabled ? '启用' : '停用' }}
        </el-tag>
      </template>
    </el-table-column>
    <el-table-column label="最近检查" width="190">
      <template #default="{ row }">
        {{ formatTime(row.last_checked_at) }}
      </template>
    </el-table-column>
    <el-table-column label="操作" width="230" fixed="right">
      <template #default="{ row }">
        <el-button size="small" @click="check(row)">检查</el-button>
        <el-button size="small" @click="openEdit(row)">编辑</el-button>
        <el-button size="small" type="danger" @click="remove(row)">删除</el-button>
      </template>
    </el-table-column>
  </el-table>

  <el-dialog v-model="dialogVisible" :title="editingId ? '编辑运行环境' : '新增运行环境'" width="640px">
    <el-form label-width="110px">
      <el-form-item label="节点">
        <el-select v-model="form.node_id" filterable placeholder="选择节点" :disabled="Boolean(editingId)">
          <el-option v-for="node in nodes" :key="node.id" :label="node.name" :value="node.id" />
        </el-select>
      </el-form-item>
      <el-form-item label="名称" required>
        <el-input v-model="form.name" placeholder="例如 vLLM Docker" />
      </el-form-item>
      <el-form-item label="后端" required>
        <el-select v-model="form.backend">
          <el-option v-for="backend in backends" :key="backend" :label="backendLabel(backend)" :value="backend" />
        </el-select>
      </el-form-item>
      <el-form-item label="运行方式" required>
        <el-select v-model="form.deploy_type">
          <el-option label="容器 / Docker" value="docker" />
          <el-option label="脚本 / Script" value="script" />
          <el-option label="本地程序" value="binary" />
        </el-select>
      </el-form-item>

      <!-- Docker fields -->
      <template v-if="form.deploy_type === 'docker'">
        <el-form-item label="Docker 镜像" required>
          <el-input v-model="dockerRt.image" placeholder="例如：vllm/vllm-openai:latest" />
        </el-form-item>
        <el-form-item label="容器内服务端口">
          <el-input-number v-model="dockerRt.container_port" :min="1" :max="65535" />
        </el-form-item>
        <el-divider content-position="left">Docker 参数</el-divider>
        <el-form-item label="GPU">
          <el-input v-model="dockerRt.gpu" placeholder="all" />
        </el-form-item>
        <el-form-item label="IPC">
          <el-input v-model="dockerRt.ipc" placeholder="host" />
        </el-form-item>
        <el-form-item label="缓存路径">
          <el-switch v-model="rtToggles.showCache" size="small" style="margin-right:8px" />
          <template v-if="rtToggles.showCache">
            <el-input v-model="dockerRt.cache_host_path" placeholder="宿主机路径" style="margin-bottom:4px" />
            <el-input v-model="dockerRt.cache_container_path" placeholder="容器内路径" />
          </template>
          <span v-else class="muted">未启用</span>
        </el-form-item>
        <el-form-item label="显存使用比例">
          <el-switch v-model="rtToggles.showGpuMem" size="small" style="margin-right:8px" />
          <el-input-number v-if="rtToggles.showGpuMem" v-model="dockerRt.gpu_memory_utilization" :min="0.1" :max="1.0" :step="0.05" />
          <span v-else class="muted">未启用</span>
        </el-form-item>
        <el-form-item label="最大模型长度">
          <el-switch v-model="rtToggles.showMaxModelLen" size="small" style="margin-right:8px" />
          <el-input-number v-if="rtToggles.showMaxModelLen" v-model="dockerRt.max_model_len" :min="512" :step="512" />
          <span v-else class="muted">未启用</span>
        </el-form-item>
        <el-form-item label="最大并发序列数">
          <el-switch v-model="rtToggles.showMaxNumSeqs" size="small" style="margin-right:8px" />
          <el-input-number v-if="rtToggles.showMaxNumSeqs" v-model="dockerRt.max_num_seqs" :min="1" :max="256" />
          <span v-else class="muted">未启用</span>
        </el-form-item>
        <el-form-item label="高级后端参数">
          <el-switch v-model="rtToggles.showExtraBackend" size="small" style="margin-right:8px" />
          <el-input v-if="rtToggles.showExtraBackend" v-model="form.extra_backend_args" type="textarea" :rows="3" placeholder="每行一个参数" />
          <span v-else class="muted">未启用</span>
        </el-form-item>
        <el-form-item label="高级 Docker 参数">
          <el-switch v-model="rtToggles.showExtraDocker" size="small" style="margin-right:8px" />
          <el-input v-if="rtToggles.showExtraDocker" v-model="form.extra_docker_args" type="textarea" :rows="3" placeholder="每行一个参数" />
          <span v-else class="muted">未启用</span>
        </el-form-item>
      </template>

      <!-- Local entrypoint fields -->
      <template v-if="form.deploy_type !== 'docker'">
        <el-form-item label="入口路径" :required="form.backend !== 'ollama'">
          <el-input v-model="form.binary_path" placeholder="/usr/local/bin/ollama 或程序路径" />
        </el-form-item>
        <el-form-item label="高级后端参数" v-if="form.backend !== 'ollama'">
          <el-input v-model="form.extra_backend_args" type="textarea" :rows="3" placeholder="每行一个参数" />
        </el-form-item>
      </template>

      <el-form-item label="工作目录">
        <el-input v-model="form.working_dir" placeholder="可选" />
      </el-form-item>
      <el-form-item label="日志目录">
        <el-input v-model="form.log_dir" placeholder="可选" />
      </el-form-item>
      <el-form-item label="版本">
        <el-input v-model="form.version" placeholder="可选" />
      </el-form-item>
      <el-form-item label="启用">
        <el-switch v-model="form.enabled" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="dialogVisible = false">取消</el-button>
      <el-button type="primary" @click="submit">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { ElMessage } from 'element-plus/es/components/message/index'
import { ElMessageBox } from 'element-plus/es/components/message-box/index'
import { ElNotification } from 'element-plus/es/components/notification/index'
import { computed, onMounted, reactive, ref } from 'vue'
import {
  checkRuntimeEnvironment,
  createRuntimeEnvironment,
  deleteRuntimeEnvironment,
  fetchModelInstances,
  fetchNodes,
  fetchRuntimeEnvironments,
  updateRuntimeEnvironment
} from '../api'
import type { ModelInstance, NodeStatus, RuntimeEnvironment } from '../types'
import {
  assembleDockerRuntimeParams,
  defaultDockerRuntimeFields,
  parseDockerRuntimeParams,
  type DockerRuntimeFields,
} from '../utils/templates'

const backends = ['ollama', 'llama_cpp', 'vllm', 'custom']
const nodes = ref<NodeStatus[]>([])
const environments = ref<RuntimeEnvironment[]>([])
const instances = ref<ModelInstance[]>([])
const loading = ref(false)
const error = ref('')
const dialogVisible = ref(false)
const editingId = ref('')
const form = ref({
  node_id: '',
  name: '',
  backend: 'ollama',
  deploy_type: 'binary',
  version: '',
  binary_path: '',
  docker_image: '',
  working_dir: '',
  log_dir: '',
  enabled: true,
  extra_backend_args: '',
  extra_docker_args: '',
  params_json: '',
})

const dockerRt = reactive<DockerRuntimeFields>(defaultDockerRuntimeFields())

const rtToggles = reactive({
  showCache: false,
  showGpuMem: false,
  showMaxModelLen: false,
  showMaxNumSeqs: false,
  showExtraBackend: false,
  showExtraDocker: false,
})

const currentRuntimeParamsJson = computed(() => {
  if (form.value.deploy_type === 'docker') {
    return assembleDockerRuntimeParams(dockerRt, form.value.backend, rtToggles, form.value.extra_backend_args, form.value.extra_docker_args)
  }
  // For local runtimes, assemble basic params
  if (form.value.extra_backend_args.trim()) {
    return JSON.stringify({ extra_backend_args: form.value.extra_backend_args.split('\n').map(s => s.trim()).filter(Boolean) })
  }
  return ''
})

function emptyToNull(value: string) {
  return value.trim() ? value.trim() : null
}

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [nextNodes, nextEnvironments, nextInstances] = await Promise.all([
      fetchNodes(),
      fetchRuntimeEnvironments(),
      fetchModelInstances()
    ])
    nodes.value = nextNodes
    environments.value = nextEnvironments
    instances.value = nextInstances
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

function openCreate() {
  editingId.value = ''
  form.value = {
    node_id: nodes.value[0]?.id ?? '',
    name: '',
    backend: 'vllm',
    deploy_type: 'docker',
    version: '',
    binary_path: '',
    docker_image: '',
    working_dir: '',
    log_dir: '',
    enabled: true,
    extra_backend_args: '',
    extra_docker_args: '',
    params_json: '',
  }
  Object.assign(dockerRt, defaultDockerRuntimeFields())
  Object.assign(rtToggles, { showCache: false, showGpuMem: false, showMaxModelLen: false, showMaxNumSeqs: false, showExtraBackend: false, showExtraDocker: false })
  dialogVisible.value = true
}

function openEdit(row: RuntimeEnvironment) {
  editingId.value = row.id
  form.value = {
    node_id: row.node_id ?? '',
    name: row.name,
    backend: row.backend,
    deploy_type: row.deploy_type,
    version: row.version ?? '',
    binary_path: row.binary_path ?? '',
    docker_image: row.docker_image ?? '',
    working_dir: row.working_dir ?? '',
    log_dir: row.log_dir ?? '',
    enabled: row.enabled,
    extra_backend_args: '',
    extra_docker_args: '',
    params_json: row.params_json ?? '',
  }
  if (row.deploy_type === 'docker') {
    Object.assign(dockerRt, parseDockerRuntimeParams(row.params_json))
    if (row.params_json) {
      try {
        const p = JSON.parse(row.params_json)
        rtToggles.showCache = !!(p.cache_host_path || p.cache_container_path)
        rtToggles.showGpuMem = !!(p.defaults?.gpu_memory_utilization != null)
        rtToggles.showMaxModelLen = !!(p.defaults?.max_model_len != null)
        rtToggles.showMaxNumSeqs = !!(p.defaults?.max_num_seqs != null)
        rtToggles.showExtraBackend = !!(p.extra_backend_args?.length > 0)
        rtToggles.showExtraDocker = !!(p.extra_docker_args?.length > 0)
      } catch { /* ignore */ }
    }
  }
  dialogVisible.value = true
}

function validateForm(): string | null {
  if (!form.value.name.trim()) return '请填写名称'
  if (!form.value.node_id) return '请选择节点'
  if (form.value.deploy_type === 'docker') {
    if (!dockerRt.image.trim()) return '请填写 Docker 镜像'
    if (!dockerRt.container_port || dockerRt.container_port < 1 || dockerRt.container_port > 65535) return '容器内服务端口无效'
    if (rtToggles.showGpuMem && (dockerRt.gpu_memory_utilization <= 0 || dockerRt.gpu_memory_utilization > 1)) return '显存使用比例需在 0~1 之间'
    if (rtToggles.showMaxModelLen && dockerRt.max_model_len < 1) return '最大模型长度需为正整数'
    if (rtToggles.showMaxNumSeqs && dockerRt.max_num_seqs < 1) return '最大并发序列数需为正整数'
  }
  if (form.value.deploy_type !== 'docker' && form.value.backend !== 'ollama') {
    if (!form.value.binary_path.trim()) return '请填写入口路径'
  }
  return null
}

async function submit() {
  if (editingId.value) {
    const running = instances.value.filter(
      i => i.runtime_environment_id === editingId.value && ['running', 'starting', 'stopping'].includes(i.status)
    )
    if (running.length > 0) {
      ElMessage.warning(`运行环境正在被运行中的实例 ${running.map(i => i.name).join(', ')} 使用，不能修改。请先停止实例。`)
      return
    }
  }
  const err = validateForm()
  if (err) { ElMessage.error(err); return }
  const payload = {
    name: form.value.name.trim(),
    backend: form.value.backend,
    deploy_type: form.value.deploy_type,
    version: emptyToNull(form.value.version),
    base_url: null,
    health_url: null,
    endpoint_url: null,
    binary_path: emptyToNull(form.value.binary_path),
    docker_image: form.value.deploy_type === 'docker' ? (dockerRt.image.trim() || null) : emptyToNull(form.value.docker_image),
    working_dir: emptyToNull(form.value.working_dir),
    log_dir: emptyToNull(form.value.log_dir),
    enabled: form.value.enabled,
    params_json: currentRuntimeParamsJson.value || null,
  }
  try {
    if (editingId.value) {
      await updateRuntimeEnvironment(editingId.value, payload)
      ElMessage.success('运行环境已更新')
    } else {
      await createRuntimeEnvironment(form.value.node_id, payload)
      ElMessage.success('运行环境已创建')
    }
    dialogVisible.value = false
    await loadData()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '保存失败')
  }
}

async function check(row: RuntimeEnvironment) {
  const checked = await checkRuntimeEnvironment(row.id)
  ElNotification({
    title: `检查状态：${checkLabel(checked.check_status)}`,
    message: checked.check_message ?? formatTime(checked.last_checked_at),
    type: checked.check_status === 'available' ? 'success' : checked.check_status === 'agent_offline' ? 'error' : 'warning'
  })
  await loadData()
}

async function remove(row: RuntimeEnvironment) {
  await ElMessageBox.confirm(`删除运行环境 ${row.name}？`, '确认删除', {
    type: 'warning',
    confirmButtonText: '确认',
    cancelButtonText: '取消'
  })
  try {
    await deleteRuntimeEnvironment(row.id)
    ElMessage.success('已删除')
    await loadData()
  } catch (err) {
    ElMessage.error(toBusinessMessage(err))
  }
}

function nodeName(nodeId?: string | null) {
  if (!nodeId) return '-'
  return nodes.value.find((node) => node.id === nodeId)?.name ?? nodeId
}

function checkType(status?: string | null) {
  if (status === 'available') return 'success'
  if (status === 'unavailable' || status === 'check_timeout' || status === 'agent_offline' || status === 'not_executable' || status === 'invalid_path') return 'danger'
  if (status === 'pending' || status === 'version_unavailable') return 'warning'
  return 'info'
}

function checkLabel(status?: string | null) {
  const labels: Record<string, string> = {
    available: '入口可用',
    version_unavailable: '版本无法自动获取',
    unavailable: '不可用',
    not_executable: '不可执行',
    invalid_path: '路径错误',
    check_timeout: '检查超时',
    agent_offline: 'Agent 离线',
    pending: '待检查'
  }
  return labels[status ?? ''] ?? '未知'
}

function backendLabel(value: string) {
  const labels: Record<string, string> = {
    ollama: 'Ollama',
    llama_cpp: 'llama.cpp',
    vllm: 'vLLM',
    custom: '自定义'
  }
  return labels[value] ?? value
}

function deployTypeLabel(value: string) {
  if (value === 'binary') return '本地程序'
  if (value === 'script') return '脚本'
  if (value === 'docker') return '容器'
  return value
}

function formatTime(value?: number | null) {
  if (!value) return '-'
  return new Date(value * 1000).toLocaleString()
}

function toBusinessMessage(err: unknown) {
  const message = err instanceof Error ? err.message : '操作失败'
  if (message.includes('runtime environment is used by model instances')) {
    return '运行环境已被模型实例引用，不能删除'
  }
  return message
}

onMounted(loadData)
defineExpose({ refresh: loadData })
</script>
