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
    <el-table-column label="状态" width="180">
      <template #default="{ row }">
        <el-tag :type="statusType(row)">{{ instanceStatusLabel(row) }}</el-tag>
      </template>
    </el-table-column>
    <el-table-column label="检查结果" min-width="280">
      <template #default="{ row }">
        <div v-if="isAgentOffline(row)" class="agent-offline-warning">
          <el-tag type="warning" size="small">Agent 离线</el-tag>
          <span>实例运行状态无法确认</span>
          <div v-if="row.last_heartbeat_at" class="muted tiny-text">最后心跳：{{ formatTime(row.last_heartbeat_at) }}</div>
        </div>
        <div v-else>{{ row.last_error ?? '暂无错误' }}</div>
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
          <el-select v-model="form.runtime_environment_id" filterable @change="onRuntimeChange">
            <el-option
              v-for="env in localRuntimeOptions"
              :key="env.id"
              :label="`${env.name} (${backendLabel(env.backend)} / ${runtimeDeployTypeLabel(env.deploy_type)})`"
              :value="env.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="模型文件">
          <el-select v-model="form.model_file_id" filterable @change="onModelChange">
            <el-option
              v-for="file in localFileOptions"
              :key="file.id"
              :label="`${file.model_name ?? file.model_id}: ${file.path} (${file.path_type === 'directory' ? '目录' : '文件'})`"
              :value="file.id"
            />
          </el-select>
        </el-form-item>
        <el-alert v-if="compatWarning" :title="compatWarning" type="warning" show-icon class="alert" />

        <!-- Docker instance form -->
        <template v-if="isDockerRuntime">
          <el-divider content-position="left">Docker 实例参数</el-divider>
          <el-alert title="以下为实例级覆盖参数。已启用的参数将覆盖运行环境默认值；未启用的参数使用运行环境默认值。未启用的参数不会被写入实例 params_json。image、GPU、IPC、缓存路径等通用参数来自运行环境，实例不必重复配置。" type="info" show-icon class="alert" />
          <el-form-item label="容器名称">
            <el-input v-model="form.container_name" placeholder="lightai-qwen3-0-6b" />
          </el-form-item>
          <el-form-item label="宿主机端口">
            <el-input-number v-model="form.host_port" :min="1024" :max="65535" />
          </el-form-item>
          <el-form-item label="容器端口">
            <el-input-number v-model="form.container_port" :min="1" :max="65535" />
          </el-form-item>
          <el-form-item label="模型容器路径">
            <el-input v-model="form.model_container_path" placeholder="/models/qwen3-0.6b" />
          </el-form-item>
          <el-form-item label="服务模型名">
            <el-input v-model="form.served_model_name" placeholder="qwen3-0.6b" />
          </el-form-item>

          <el-divider content-position="left">可选参数（勾选后启用）</el-divider>

          <el-form-item label="GPU 显存使用比例">
            <el-switch v-model="instToggles.showGpuMem" size="small" style="margin-right:8px" />
            <el-input-number v-if="instToggles.showGpuMem" v-model="form.gpu_memory_utilization" :min="0.1" :max="1.0" :step="0.05" />
            <span v-else class="muted">未启用（使用 Runtime 默认值）</span>
          </el-form-item>

          <el-form-item label="最大模型长度">
            <el-switch v-model="instToggles.showMaxModelLen" size="small" style="margin-right:8px" />
            <el-input-number v-if="instToggles.showMaxModelLen" v-model="form.max_model_len" :min="512" :step="512" />
            <span v-else class="muted">未启用（使用 Runtime 默认值）</span>
          </el-form-item>

          <el-form-item label="最大并发序列数">
            <el-switch v-model="instToggles.showMaxNumSeqs" size="small" style="margin-right:8px" />
            <el-input-number v-if="instToggles.showMaxNumSeqs" v-model="form.max_num_seqs" :min="1" :max="256" />
            <span v-else class="muted">未启用（使用 Runtime 默认值）</span>
          </el-form-item>

          <el-form-item label="GPU">
            <el-switch v-model="instToggles.showGpu" size="small" style="margin-right:8px" />
            <el-input v-if="instToggles.showGpu" v-model="form.docker_gpu" placeholder="all" />
            <span v-else class="muted">未启用（使用 Runtime 默认值）</span>
          </el-form-item>

          <el-form-item label="高级 Docker 参数">
            <el-switch v-model="instToggles.showExtraDocker" size="small" style="margin-right:8px" />
            <el-input v-if="instToggles.showExtraDocker" v-model="form.extra_docker_args_text" type="textarea" :rows="2" placeholder="一行一个参数" />
            <span v-else class="muted">未启用</span>
          </el-form-item>

          <el-form-item label="高级后端参数">
            <el-switch v-model="instToggles.showExtraBackend" size="small" style="margin-right:8px" />
            <el-input v-if="instToggles.showExtraBackend" v-model="form.extra_backend_args_text" type="textarea" :rows="2" placeholder="一行一个参数" />
            <span v-else class="muted">未启用</span>
          </el-form-item>

          <el-collapse class="advanced-fields">
            <el-collapse-item title="实例参数 JSON（高级编辑）" name="docker-json">
              <el-alert title="点击下方按钮根据当前表单值生成 JSON，或手动编辑。Docker 容器默认不加 --rm。" type="info" show-icon class="alert" />
              <el-button size="small" type="primary" @click="generateDockerParamsJson" style="margin-bottom:8px">根据表单生成 JSON</el-button>
              <el-button size="small" @click="generateDockerTemplate" :disabled="!form.model_file_id || !form.runtime_environment_id" style="margin-bottom:8px">从 Runtime 模板生成</el-button>
              <el-form-item label="参数 JSON">
                <el-input v-model="form.params_json" type="textarea" :rows="10" placeholder='{"container_name":"lightai-test","host_port":18000,...}' />
              </el-form-item>
            </el-collapse-item>
          </el-collapse>

          <el-alert type="warning" show-icon class="alert" style="margin-top: 8px">
            <template #title>
              <div>Docker 部署环境要求：</div>
              <ul style="margin: 4px 0; padding-left: 16px">
                <li>请确认目标 Node 已安装 Docker 和 NVIDIA Container Toolkit</li>
                <li>请确认模型路径是目标 Node 上可访问的路径</li>
                <li>host_port 可能冲突，当前需用户自行确认</li>
                <li>Docker 容器默认不加 --rm，便于失败诊断</li>
              </ul>
            </template>
          </el-alert>
        </template>

        <!-- Local (non-Docker) instance form -->
        <template v-else>
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
            <el-input v-model="form.extra_args_text" type="textarea" :rows="4" placeholder="一行一个参数，例如：&#10;--verbose&#10;--batch-size&#10;512" />
          </el-form-item>
          <el-collapse class="advanced-fields">
            <el-collapse-item title="高级探测配置（可选，留空使用后端默认值）" name="probe">
              <el-alert title="以下参数用于实例启动后的服务就绪探测" type="info" show-icon class="alert" />
              <el-form-item label="探测路径">
                <el-input v-model="form.probe_paths_text" type="textarea" :rows="2" />
              </el-form-item>
              <el-form-item label="失败重试次数">
                <el-input-number v-model="form.probe_max_attempts" :min="1" :max="60" />
              </el-form-item>
              <el-form-item label="重试间隔(ms)">
                <el-input-number v-model="form.probe_interval_ms" :min="50" :max="60000" :step="500" />
              </el-form-item>
              <el-form-item label="请求超时(ms)">
                <el-input-number v-model="form.probe_timeout_ms" :min="50" :max="60000" :step="100" />
              </el-form-item>
            </el-collapse-item>
          </el-collapse>
          <el-alert title="工作目录来自运行环境配置。" type="info" show-icon class="alert" />
        </template>
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
      <div class="log-toolbar">
        <el-button size="small" :loading="logRefreshing" @click="refreshLogs">刷新日志</el-button>
        <span v-if="logMessage" class="muted">{{ logMessage }}</span>
      </div>
      <pre class="log-box">{{ selectedLogInstance.log_tail ?? selectedLogInstance.last_error ?? '暂无日志' }}</pre>
    </div>
  </el-dialog>
</template>

<script setup lang="ts">
import { ElMessage } from 'element-plus/es/components/message/index'
import { ElMessageBox } from 'element-plus/es/components/message-box/index'
import { ElNotification } from 'element-plus/es/components/notification/index'
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import {
  checkModelInstance,
  createModelInstance,
  deleteModelInstance,
  fetchModelFiles,
  fetchModelInstance,
  fetchModelInstances,
  fetchModels,
  fetchNodes,
  fetchRuntimeEnvironments,
  refreshInstanceLogs,
  startModelInstance,
  stopModelInstance,
  testModelInstance,
  updateModelInstance
} from '../api'
import type { ModelDefinition, ModelFile, ModelInstance, NodeStatus, RuntimeEnvironment } from '../types'
import { backendLabel, checkFailedReason, deployTypeLabel, emptyToNull, formatTime, instanceStatusLabel, isAgentOffline, runtimeDeployTypeLabel, statusLabel, statusType } from '../utils/instance'
import { buildDockerInstanceParams, emptyForm, localParams, parseParams } from './instances/instanceParams'
import type { InstanceForm } from './instances/instanceParams'
import { useInstanceRefresh } from './instances/useInstanceRefresh'
import { checkModelRuntimeCompat, generateDockerInstanceOverrides, toTemplateJson } from '../utils/templates'

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
const logRefreshing = ref(false)
const logMessage = ref('')
const form = ref<InstanceForm>(emptyForm())
const compatWarning = ref('')
const instToggles = reactive({
  showGpuMem: false,
  showMaxModelLen: false,
  showMaxNumSeqs: false,
  showGpu: false,
  showExtraDocker: false,
  showExtraBackend: false,
})
const isDockerRuntime = computed(() => {
  const rt = runtimeEnvironments.value.find(e => e.id === form.value.runtime_environment_id)
  return rt?.deploy_type === 'docker'
})

function onRuntimeChange() {
  if (!isDockerRuntime.value) return
  const runtime = runtimeEnvironments.value.find(e => e.id === form.value.runtime_environment_id)
  if (!runtime?.params_json) return
  try {
    const rp = JSON.parse(runtime.params_json)
    // Populate instance fields from Runtime defaults (only if instance field is empty/default)
    const formFilled = (form.value as any).__runtime_defaults_filled
    if (!formFilled) {
      form.value.container_port = rp.container_port || 8000
      form.value.docker_gpu = rp.gpu || 'all'
      form.value.gpu_memory_utilization = rp.defaults?.gpu_memory_utilization ?? 0.5
      form.value.max_model_len = rp.defaults?.max_model_len ?? 4096
      form.value.max_num_seqs = rp.defaults?.max_num_seqs ?? 8
      ;(form.value as any).__runtime_defaults_filled = true
    }
    // Enable toggles based on what Runtime provides
    instToggles.showGpu = !!rp.gpu
    instToggles.showGpuMem = rp.defaults?.gpu_memory_utilization != null
    instToggles.showMaxModelLen = rp.defaults?.max_model_len != null
    instToggles.showMaxNumSeqs = rp.defaults?.max_num_seqs != null
    instToggles.showExtraDocker = Array.isArray(rp.extra_docker_args) && rp.extra_docker_args.length > 0
    instToggles.showExtraBackend = Array.isArray(rp.extra_backend_args) && rp.extra_backend_args.length > 0
  } catch { /* ignore */ }
}

function onModelChange() {
  if (!isDockerRuntime.value) return
  const model = models.value.find(m => m.id === form.value.model_id)
  const modelFile = modelFiles.value.find(f => f.id === form.value.model_file_id)
  if (!modelFile) return
  // Derive model_container_path from model path
  const modelDir = (modelFile.path || '').split('/').pop() || 'model'
  if (!form.value.model_container_path) {
    form.value.model_container_path = `/models/${modelDir}`
  }
  // Derive served_model_name from model metadata or model name
  if (!form.value.served_model_name) {
    if (model?.params_json) {
      try {
        const mp = JSON.parse(model.params_json)
        form.value.served_model_name = mp.served_model_name || model.name || ''
      } catch { form.value.served_model_name = model.name || '' }
    } else {
      form.value.served_model_name = model?.name || ''
    }
  }
}

function generateDockerParamsJson() {
  const overrides = buildDockerInstanceParams(form.value)
  const modelFile = modelFiles.value.find(f => f.id === form.value.model_file_id)
  if (modelFile) {
    const modelDir = (modelFile.path || '').split('/').pop() || 'model'
    ;(overrides as any).model_host_path = modelFile.path
    if (!overrides.model_container_path) (overrides as any).model_container_path = `/models/${modelDir}`
  }
  if (form.value.params_json && form.value.params_json.trim()) {
    ElMessageBox.confirm('当前已有实例参数，覆盖会丢失已填信息。是否继续？', '确认覆盖', { type: 'warning', confirmButtonText: '覆盖', cancelButtonText: '取消' }).then(() => {
      form.value.params_json = toTemplateJson(overrides)
      ElMessage.success('已根据表单生成实例参数 JSON')
    }).catch(() => {})
  } else {
    form.value.params_json = toTemplateJson(overrides)
    ElMessage.success('已根据表单生成实例参数 JSON')
  }
}

function generateDockerTemplate() {
  const model = models.value.find(m => m.id === form.value.model_id)
  const modelFile = modelFiles.value.find(f => f.id === form.value.model_file_id)
  const runtime = runtimeEnvironments.value.find(e => e.id === form.value.runtime_environment_id)
  if (!modelFile || !runtime) {
    ElMessage.warning('请先选择模型文件和运行环境')
    return
  }
  const modelParams = model?.params_json ? JSON.parse(model.params_json) : null
  const runtimeParams = runtime.params_json ? JSON.parse(runtime.params_json) : null
  const modelName = model?.name || ''
  const modelPath = modelFile.path || ''
  const overrides = generateDockerInstanceOverrides(modelName, modelPath, modelParams, runtimeParams)
  if (form.value.params_json && form.value.params_json.trim()) {
    ElMessageBox.confirm('当前已有实例参数，覆盖会丢失已填信息。是否继续？', '确认覆盖', { type: 'warning', confirmButtonText: '覆盖', cancelButtonText: '取消' }).then(() => {
      form.value.params_json = toTemplateJson(overrides)
      ElMessage.success('已从 Runtime 模板生成实例覆盖参数')
    }).catch(() => {})
  } else {
    form.value.params_json = toTemplateJson(overrides)
    ElMessage.success('已从 Runtime 模板生成实例覆盖参数')
  }
}

function checkCompat() {
  const model = models.value.find(m => m.id === form.value.model_id)
  const runtime = runtimeEnvironments.value.find(e => e.id === form.value.runtime_environment_id)
  if (!model || !runtime) { compatWarning.value = ''; return }
  const modelParams = model.params_json ? JSON.parse(model.params_json) : null
  const result = checkModelRuntimeCompat(modelParams, runtime.backend)
  compatWarning.value = result.warning
}

const instanceTypeOptions = [
  { label: '外部服务', value: 'external' },
  { label: '本地', value: 'local' }
]

const { replaceInstance, refreshSingleInstance, startPeriodicRefresh, stopPeriodicRefresh } =
  useInstanceRefresh(instances)

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
    extra_args_text: params.extra_args.join('\n'),
    container_name: params.container_name,
    host_port: params.host_port,
    container_port: params.container_port,
    model_container_path: params.model_container_path,
    served_model_name: params.served_model_name,
    gpu_memory_utilization: params.gpu_memory_utilization,
    max_model_len: params.max_model_len,
    max_num_seqs: params.max_num_seqs,
    docker_gpu: params.docker_gpu,
    extra_docker_args_text: params.extra_docker_args_text,
    extra_backend_args_text: params.extra_backend_args_text,
    probe_paths_text: params.probe_paths_text,
    probe_max_attempts: params.probe_max_attempts,
    probe_interval_ms: params.probe_interval_ms,
    probe_timeout_ms: params.probe_timeout_ms,
    params_json: row.params_json ?? '',
  }
  dialogVisible.value = true
}

function openLogs(row: ModelInstance) {
  selectedLogInstance.value = instances.value.find((inst) => inst.id === row.id) ?? row
  logMessage.value = ''
  logDialogVisible.value = true
}

async function refreshLogs() {
  if (!selectedLogInstance.value) return
  logRefreshing.value = true
  logMessage.value = ''
  try {
    const response = await refreshInstanceLogs(selectedLogInstance.value.id)
    const updated = await fetchModelInstance(selectedLogInstance.value.id)
    replaceInstance(updated)
    selectedLogInstance.value = updated
    logMessage.value = response.message ?? '日志已刷新'
  } catch (err) {
    logMessage.value = err instanceof Error ? err.message : '刷新失败'
  } finally {
    logRefreshing.value = false
  }
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
    params_json: form.value.deploy_type === 'local'
      ? (isDockerRuntime.value
          ? (form.value.params_json.trim() || JSON.stringify(buildDockerInstanceParams(form.value)))
          : JSON.stringify(localParams(form.value)))
      : null,
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

async function pollInstanceUntilStable(id: string, initialStatus: string) {
  const transitional = ['starting', 'stopping']
  if (!transitional.includes(initialStatus)) return
  for (let i = 0; i < 24; i++) {
    await new Promise((resolve) => setTimeout(resolve, 1500))
    try {
      const updated = await fetchModelInstance(id)
      replaceInstance(updated)
      if (!transitional.includes(updated.status)) {
        ElNotification({
          title: `操作完成：${statusLabel(updated.status)}`,
          message: updated.last_error ?? '实例状态已更新',
          type: updated.status === 'running' ? 'success' : updated.status === 'failed' ? 'error' : 'warning'
        })
        return
      }
    } catch {
      // keep polling on transient errors
    }
  }
  ElMessage.warning('等待实例状态超时（36 秒），请手动刷新查看结果')
}

async function check(row: ModelInstance) {
  try {
    const checked = await checkModelInstance(row.id)
    replaceInstance(checked)
    const isFailed = checkFailedReason(checked.last_error)
    ElNotification({
      title: `检查结果：${statusLabel(checked.status)}`,
      message: checked.last_error ?? formatTime(checked.last_checked_at),
      type: isFailed ? 'error' : checked.status === 'running' ? 'success' : checked.status === 'failed' ? 'error' : 'warning'
    })
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '状态检查失败')
    await refreshSingleInstance(row.id)
  }
}

async function start(row: ModelInstance) {
  try {
    const started = await startModelInstance(row.id)
    replaceInstance(started)
    if (started.status === 'running' || started.status === 'failed') {
      ElNotification({ title: `启动结果：${statusLabel(started.status)}`, message: started.last_error ?? '实例状态已更新', type: started.status === 'running' ? 'success' : started.status === 'failed' ? 'error' : 'warning' })
    }
    await pollInstanceUntilStable(started.id, started.status)
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '启动失败')
    await refreshSingleInstance(row.id)
  }
}

async function stop(row: ModelInstance) {
  try {
    const stopped = await stopModelInstance(row.id)
    replaceInstance(stopped)
    if (stopped.status === 'stopped' || stopped.status === 'failed') {
      ElNotification({ title: `停止结果：${statusLabel(stopped.status)}`, message: stopped.last_error ?? '实例状态已更新', type: stopped.status === 'stopped' ? 'success' : 'warning' })
    }
    await pollInstanceUntilStable(stopped.id, stopped.status)
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '停止失败')
    await refreshSingleInstance(row.id)
  }
}

async function testLocal(row: ModelInstance) {
  try {
    const tested = await testModelInstance(row.id)
    replaceInstance(tested)
    ElNotification({
      title: '测试完成',
      message: tested.last_error ?? '测试成功',
      type: tested.status === 'running' ? 'success' : 'warning'
    })
    await pollInstanceUntilStable(tested.id, tested.status)
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '测试失败')
    await refreshSingleInstance(row.id)
  }
}

watch([() => form.value.model_id, () => form.value.runtime_environment_id], checkCompat)

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

onMounted(async () => {
  await loadData()
  startPeriodicRefresh()
})

onUnmounted(stopPeriodicRefresh)

defineExpose({ refresh: loadData })
</script>
