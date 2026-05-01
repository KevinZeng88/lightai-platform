<template>
  <section class="panel-header">
    <div>
      <h2>模型实例</h2>
      <p>External 实例直接接入已有服务；本地实例选择节点、运行环境和已验证模型文件后由 Agent 启动/停止。</p>
    </div>
    <div class="toolbar compact">
      <el-button :loading="loading" @click="loadData">刷新</el-button>
      <el-button type="primary" @click="openCreate">新增实例</el-button>
    </div>
  </section>

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-table :data="instances" row-key="id" border>
    <el-table-column prop="name" label="实例" min-width="150" fixed="left" />
    <el-table-column label="状态" width="120">
      <template #default="{ row }">
        <el-tag :type="statusType(row.status)">{{ statusLabel(row.status) }}</el-tag>
      </template>
    </el-table-column>
    <el-table-column label="检查结果" min-width="280">
      <template #default="{ row }">
        <div>{{ row.last_error ?? '暂无错误' }}</div>
        <div v-if="row.last_checked_at" class="muted">{{ formatTime(row.last_checked_at) }}</div>
      </template>
    </el-table-column>
    <el-table-column prop="model_name" label="服务模型名" min-width="150" />
    <el-table-column label="类型" width="100">
      <template #default="{ row }">{{ deployTypeLabel(row.deploy_type) }}</template>
    </el-table-column>
    <el-table-column label="后端" width="120">
      <template #default="{ row }">{{ backendLabel(row.backend) }}</template>
    </el-table-column>
    <el-table-column label="节点 / 运行环境" min-width="220">
      <template #default="{ row }">{{ row.deploy_type === 'local' ? `${row.node_name ?? row.node_id} / ${row.runtime_environment_name ?? row.runtime_environment_id}` : '-' }}</template>
    </el-table-column>
    <el-table-column label="模型文件 / 外部地址" min-width="260" show-overflow-tooltip>
      <template #default="{ row }">{{ row.deploy_type === 'local' ? row.model_file_path : row.base_url }}</template>
    </el-table-column>
    <el-table-column label="Endpoint / 进程" min-width="230" show-overflow-tooltip>
      <template #default="{ row }">
        <div>{{ row.endpoint_url ?? row.base_url ?? '-' }}</div>
        <div v-if="row.process_id" class="muted tiny-text">PID: {{ row.process_id }}</div>
      </template>
    </el-table-column>
    <el-table-column label="操作" width="360" fixed="right">
      <template #default="{ row }">
        <el-button v-if="row.deploy_type === 'external'" size="small" @click="check(row)">检查状态</el-button>
        <el-button v-else size="small" :disabled="row.status !== 'running'" @click="check(row)">检查状态</el-button>
        <el-button v-if="row.deploy_type === 'local'" size="small" type="success" :disabled="row.status === 'running' || row.status === 'starting'" @click="start(row)">启动</el-button>
        <el-button v-if="row.deploy_type === 'local'" size="small" :disabled="row.status === 'stopped' || row.status === 'stopping'" @click="stop(row)">停止</el-button>
        <el-button v-if="row.deploy_type === 'local'" size="small" :disabled="row.status !== 'running'" @click="testLocal(row)">测试</el-button>
        <el-button size="small" @click="openLogs(row)">日志</el-button>
        <el-button size="small" @click="openEdit(row)">编辑</el-button>
        <el-button size="small" type="danger" @click="remove(row)">删除</el-button>
      </template>
    </el-table-column>
  </el-table>

  <el-dialog v-model="dialogVisible" :title="editingId ? '编辑实例' : '新增实例'" width="760px">
    <el-alert
      title="External 不依赖节点/运行环境/模型文件；本地实例必须选择同一节点上的运行环境和已验证模型文件。"
      type="info"
      show-icon
      class="alert"
    />
    <el-form label-width="120px">
      <el-form-item label="实例名称">
        <el-input v-model="form.name" />
      </el-form-item>
      <el-form-item label="实例类型">
        <el-segmented v-model="form.deploy_type" :options="instanceTypeOptions" :disabled="Boolean(editingId)" />
      </el-form-item>
      <template v-if="form.deploy_type === 'external'">
      <el-form-item label="服务模型名">
        <el-input v-model="form.model_name" placeholder="例如 local-gguf" />
      </el-form-item>
      <el-form-item label="基础地址">
        <el-input v-model="form.base_url" placeholder="http://127.0.0.1:8088" />
      </el-form-item>

      <el-collapse class="advanced-fields">
        <el-collapse-item title="高级配置（可选）" name="advanced">
          <el-form-item label="模型定义">
            <el-select v-model="form.model_id" filterable clearable :disabled="Boolean(editingId)">
          <el-option v-for="model in models" :key="model.id" :label="model.name" :value="model.id" />
            </el-select>
          </el-form-item>
          <el-form-item label="服务实现">
            <el-select v-model="form.backend" clearable placeholder="可选；默认由服务端兼容字段处理">
          <el-option v-for="backend in backends" :key="backend" :label="backendLabel(backend)" :value="backend" />
            </el-select>
          </el-form-item>
          <el-form-item label="接口类型">
            <el-input v-model="form.endpoint_url" placeholder="http://127.0.0.1:8088/v1" />
          </el-form-item>
          <el-form-item label="健康检查">
            <el-input v-model="form.health_url" placeholder="http://127.0.0.1:8088/v1/models" />
          </el-form-item>
          <el-form-item label="版本">
            <el-input v-model="form.runtime_version" />
          </el-form-item>
          <el-form-item label="备注">
            <el-input v-model="form.description" type="textarea" :rows="2" />
          </el-form-item>
        </el-collapse-item>
      </el-collapse>
      </template>
      <template v-else>
        <el-form-item label="节点">
          <el-select v-model="form.node_id" filterable @change="onLocalNodeChange">
            <el-option v-for="node in nodes" :key="node.id" :label="node.name" :value="node.id" />
          </el-select>
        </el-form-item>
        <el-form-item label="运行环境">
          <el-select v-model="form.runtime_environment_id" filterable>
            <el-option
              v-for="env in localRuntimeOptions"
              :key="env.id"
              :label="`${env.name} (${backendLabel(env.backend)} / ${runtimeDeployTypeLabel(env.deploy_type)})`"
              :value="env.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="模型文件">
          <el-select v-model="form.model_file_id" filterable>
            <el-option
              v-for="file in localFileOptions"
              :key="file.id"
              :label="`${file.model_name ?? file.model_id}: ${file.path} (${file.path_type === 'directory' ? '目录' : '文件'})`"
              :value="file.id"
            />
          </el-select>
        </el-form-item>
        <el-divider content-position="left">运行参数</el-divider>
        <el-form-item label="监听地址">
          <el-input v-model="form.host" placeholder="127.0.0.1" />
        </el-form-item>
        <el-form-item label="端口">
          <el-input-number v-model="form.port" :min="1" :max="65535" />
        </el-form-item>
        <el-form-item label="上下文">
          <el-input-number v-model="form.ctx_size" :min="0" :step="512" />
        </el-form-item>
        <el-form-item label="GPU 层数">
          <el-input-number v-model="form.gpu_layers" :min="-1" />
        </el-form-item>
        <el-form-item label="线程数">
          <el-input-number v-model="form.threads" :min="0" />
        </el-form-item>
        <el-form-item label="高级参数">
          <el-input
            v-model="form.extra_args_text"
            type="textarea"
            :rows="4"
            placeholder="一行一个参数，例如：&#10;--verbose&#10;--batch-size&#10;512"
          />
        </el-form-item>
        <el-alert
          title="工作目录来自运行环境配置。未配置时 Agent 使用自身启动目录；建议为程序或脚本配置固定应用目录，避免依赖 /tmp、用户家目录等不稳定位置。"
          type="info"
          show-icon
          class="alert"
        />
      </template>
    </el-form>
    <template #footer>
      <el-button @click="dialogVisible = false">取消</el-button>
      <el-button type="primary" @click="submit">保存</el-button>
    </template>
  </el-dialog>

  <el-dialog v-model="logDialogVisible" title="实例日志" width="780px">
    <div v-if="selectedLogInstance" class="log-detail">
      <div class="detail-grid compact-detail">
        <div><span class="muted">实例</span><p>{{ selectedLogInstance.name }}</p></div>
        <div><span class="muted">状态</span><p>{{ statusLabel(selectedLogInstance.status) }}</p></div>
        <div class="wide-detail"><span class="muted">启动命令</span><p>{{ selectedLogInstance.command ?? '暂无命令摘要' }}</p></div>
      </div>
      <pre class="log-box">{{ selectedLogInstance.log_tail ?? selectedLogInstance.last_error ?? '暂无日志' }}</pre>
    </div>
  </el-dialog>
</template>

<script setup lang="ts">
import { ElMessage } from 'element-plus/es/components/message/index'
import { ElMessageBox } from 'element-plus/es/components/message-box/index'
import { ElNotification } from 'element-plus/es/components/notification/index'
import { computed, onMounted, ref } from 'vue'
import {
  checkModelInstance,
  createModelInstance,
  deleteModelInstance,
  fetchModelFiles,
  fetchModelInstances,
  fetchModels,
  fetchNodes,
  fetchRuntimeEnvironments,
  startModelInstance,
  stopModelInstance,
  testModelInstance,
  updateModelInstance
} from '../api'
import type { ModelDefinition, ModelFile, ModelInstance, NodeStatus, RuntimeEnvironment } from '../types'

const backends = ['ollama', 'llama_cpp', 'vllm', 'custom']
const models = ref<ModelDefinition[]>([])
const nodes = ref<NodeStatus[]>([])
const runtimeEnvironments = ref<RuntimeEnvironment[]>([])
const modelFiles = ref<ModelFile[]>([])
const instances = ref<ModelInstance[]>([])
const loading = ref(false)
const error = ref('')
const dialogVisible = ref(false)
const logDialogVisible = ref(false)
const editingId = ref('')
const selectedLogInstance = ref<ModelInstance | null>(null)
const form = ref(emptyForm())
const instanceTypeOptions = [
  { label: '外部服务', value: 'external' },
  { label: '本地', value: 'local' }
]

function emptyForm() {
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
    extra_args_text: ''
  }
}

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [nextModels, nextInstances, nextNodes, nextRuntimes] = await Promise.all([
      fetchModels(),
      fetchModelInstances(),
      fetchNodes(),
      fetchRuntimeEnvironments()
    ])
    models.value = nextModels
    instances.value = nextInstances
    nodes.value = nextNodes
    runtimeEnvironments.value = nextRuntimes
    modelFiles.value = (await Promise.all(nextModels.map((model) => fetchModelFiles(model.id)))).flat()
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

function openCreate() {
  editingId.value = ''
  form.value = emptyForm()
  dialogVisible.value = true
}

function openEdit(row: ModelInstance) {
  editingId.value = row.id
  const params = parseParams(row.params_json)
  form.value = {
    model_id: row.model_id ?? '',
    model_file_id: row.model_file_id ?? '',
    node_id: row.node_id ?? '',
    runtime_environment_id: row.runtime_environment_id ?? '',
    name: row.name,
    deploy_type: row.deploy_type,
    backend: row.backend === 'custom' ? '' : row.backend,
    model_name: row.model_name ?? '',
    runtime_version: row.runtime_version ?? '',
    base_url: row.base_url ?? '',
    endpoint_url: row.endpoint_url ?? '',
    health_url: row.health_url ?? '',
    description: row.description ?? '',
    host: params.host,
    port: params.port,
    ctx_size: params.ctx_size,
    gpu_layers: params.gpu_layers,
    threads: params.threads,
    extra_args_text: params.extra_args.join('\n')
  }
  dialogVisible.value = true
}

function openLogs(row: ModelInstance) {
  selectedLogInstance.value = row
  logDialogVisible.value = true
}

async function submit() {
  if (form.value.deploy_type === 'external' && (!form.value.name || !form.value.model_name || !form.value.base_url)) {
    error.value = '请填写实例名称、服务模型名和基础地址'
    return
  }
  if (form.value.deploy_type === 'local' && (!form.value.name || !form.value.node_id || !form.value.runtime_environment_id || !form.value.model_file_id)) {
    error.value = '请填写本地实例名称、节点、运行环境和模型文件'
    return
  }
  const payload = {
    model_id: emptyToNull(form.value.model_id),
    model_file_id: emptyToNull(form.value.model_file_id),
    node_id: emptyToNull(form.value.node_id),
    runtime_environment_id: emptyToNull(form.value.runtime_environment_id),
    name: form.value.name,
    deploy_type: form.value.deploy_type,
    backend: emptyToNull(form.value.backend),
    base_url: emptyToNull(form.value.base_url),
    endpoint_url: emptyToNull(form.value.endpoint_url),
    health_url: emptyToNull(form.value.health_url),
    runtime_version: emptyToNull(form.value.runtime_version),
    model_name: emptyToNull(form.value.model_name),
    description: emptyToNull(form.value.description),
    params_json: form.value.deploy_type === 'local' ? JSON.stringify(localParams()) : null,
    status: 'unknown'
  }
  if (editingId.value) {
    await updateModelInstance(editingId.value, payload)
  } else {
    await createModelInstance(payload)
  }
  dialogVisible.value = false
  await loadData()
}

async function check(row: ModelInstance) {
  const checked = await checkModelInstance(row.id)
  ElNotification({
    title: `检查结果：${statusLabel(checked.status)}`,
    message: checked.last_error ?? formatTime(checked.last_checked_at),
    type: checked.status === 'running' ? 'success' : checked.status === 'failed' ? 'error' : 'warning'
  })
  await loadData()
}

async function start(row: ModelInstance) {
  try {
    const started = await startModelInstance(row.id)
    ElNotification({ title: `启动结果：${statusLabel(started.status)}`, message: started.last_error ?? '实例状态已更新', type: started.status === 'running' ? 'success' : 'warning' })
    await loadData()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '启动失败')
    await loadData()
  }
}

async function stop(row: ModelInstance) {
  try {
    const stopped = await stopModelInstance(row.id)
    ElNotification({ title: `停止结果：${statusLabel(stopped.status)}`, message: stopped.last_error ?? '实例状态已更新', type: stopped.status === 'stopped' ? 'success' : 'warning' })
    await loadData()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '停止失败')
    await loadData()
  }
}

async function testLocal(row: ModelInstance) {
  try {
    const tested = await testModelInstance(row.id)
    ElNotification({
      title: '测试完成',
      message: tested.last_error ?? '测试成功',
      type: tested.status === 'running' ? 'success' : 'warning'
    })
    await loadData()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '测试失败')
    await loadData()
  }
}

const localRuntimeOptions = computed(() =>
  runtimeEnvironments.value.filter(
    (env) => env.node_id === form.value.node_id && env.check_status === 'available'
  )
)
const localFileOptions = computed(() =>
  modelFiles.value.filter((file) => file.node_id === form.value.node_id && file.status === 'verified')
)

function onLocalNodeChange() {
  form.value.runtime_environment_id = ''
  form.value.model_file_id = ''
}

async function remove(row: ModelInstance) {
  await ElMessageBox.confirm(`删除实例 ${row.name}？`, '确认删除', {
    type: 'warning',
    confirmButtonText: '确认',
    cancelButtonText: '取消'
  })
  await deleteModelInstance(row.id)
  ElMessage.success('已删除')
  await loadData()
}

function statusType(status: string) {
  if (status === 'running') return 'success'
  if (status === 'failed') return 'danger'
  if (status === 'pending' || status === 'starting') return 'warning'
  return 'info'
}

function statusLabel(status: string) {
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

function deployTypeLabel(value: string) {
  if (value === 'external') return '外部服务'
  if (value === 'local') return '本地实例'
  return value
}

function runtimeDeployTypeLabel(value: string) {
  if (value === 'binary') return '程序'
  if (value === 'script') return '脚本'
  if (value === 'docker') return '容器'
  return value
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

function emptyToNull(value: string) {
  return value.trim() ? value.trim() : null
}

function localParams() {
  return {
    host: form.value.host.trim() || '127.0.0.1',
    port: form.value.port,
    ctx_size: form.value.ctx_size || undefined,
    gpu_layers: form.value.gpu_layers,
    threads: form.value.threads || undefined,
    extra_args: form.value.extra_args_text
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
  }
}

function parseParams(value?: string | null) {
  try {
    const parsed = value ? JSON.parse(value) : {}
    return {
      host: typeof parsed.host === 'string' ? parsed.host : '127.0.0.1',
      port: typeof parsed.port === 'number' ? parsed.port : 8080,
      ctx_size: typeof parsed.ctx_size === 'number' ? parsed.ctx_size : 4096,
      gpu_layers: typeof parsed.gpu_layers === 'number' ? parsed.gpu_layers : 0,
      threads: typeof parsed.threads === 'number' ? parsed.threads : 0,
      extra_args: Array.isArray(parsed.extra_args) ? parsed.extra_args.filter((item: unknown) => typeof item === 'string') : []
    }
  } catch {
    return {
      host: '127.0.0.1',
      port: 8080,
      ctx_size: 4096,
      gpu_layers: 0,
      threads: 0,
      extra_args: [] as string[]
    }
  }
}

function formatTime(value?: number | null) {
  if (!value) return '-'
  return new Date(value * 1000).toLocaleString()
}

onMounted(loadData)
defineExpose({ refresh: loadData })
</script>
