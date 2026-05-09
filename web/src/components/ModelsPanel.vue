<template>
  <section class="panel-header">
    <div>
      <h2>模型定义</h2>
      <p>模型是管理入口；节点文件路径在模型详情中维护，文件验证只代表该节点文件存在且基础信息可读取。</p>
    </div>
    <div class="toolbar compact">
      <el-button :loading="loading" @click="loadData">刷新</el-button>
      <el-button v-if="role !== 'viewer'" type="primary" @click="openCreate">新增模型</el-button>
    </div>
  </section>

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-table :data="models" row-key="id" border @expand-change="onExpand">
    <el-table-column type="expand">
      <template #default="{ row }">
        <section class="model-files-block">
          <div class="card-header-row">
            <div>
              <strong>节点模型路径</strong>
              <p class="muted">支持单文件模型和目录模型；同一路径在不同节点也需要分别验证。</p>
            </div>
            <el-button v-if="role !== 'viewer'" size="small" type="primary" @click="openFileCreate(row)">新增节点路径</el-button>
          </div>
          <el-table :data="filesByModel[row.id] ?? []" row-key="id" size="small" border>
            <el-table-column label="节点" min-width="150">
              <template #default="{ row: file }">{{ file.node_name ?? file.node_id }}</template>
            </el-table-column>
            <el-table-column label="Agent" width="100">
              <template #default="{ row: file }">
                <el-tag :type="file.node_status === 'online' ? 'success' : 'info'">{{ nodeStatusLabel(file.node_status) }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="path" label="路径" min-width="260" show-overflow-tooltip />
            <el-table-column label="类型" width="90">
              <template #default="{ row: file }">{{ file.path_type === 'directory' ? '目录' : '文件' }}</template>
            </el-table-column>
            <el-table-column label="验证状态" width="150">
              <template #default="{ row: file }">
                <el-tag :type="fileStatusType(file.status)">{{ fileStatusLabel(file.status) }}</el-tag>
                <div v-if="file.verify_task_status" class="muted tiny-text">任务：{{ taskStatusLabel(file.verify_task_status) }}</div>
              </template>
            </el-table-column>
            <el-table-column label="文件大小" width="120">
              <template #default="{ row: file }">{{ formatBytes(file.size_bytes) }}</template>
            </el-table-column>
            <el-table-column label="最近验证" width="180">
              <template #default="{ row: file }">{{ formatTime(file.last_verified_at) }}</template>
            </el-table-column>
            <el-table-column label="失败原因" min-width="200">
              <template #default="{ row: file }">{{ file.last_error ?? '-' }}</template>
            </el-table-column>
            <el-table-column v-if="role !== 'viewer'" label="操作" width="230" fixed="right">
              <template #default="{ row: file }">
                <el-button size="small" @click="verifyFile(file)">验证文件</el-button>
                <el-button size="small" @click="openFileEdit(row, file)">编辑</el-button>
                <el-button size="small" type="danger" @click="removeFile(row, file)">删除</el-button>
              </template>
            </el-table-column>
          </el-table>
        </section>
      </template>
    </el-table-column>
    <el-table-column prop="name" label="名称" min-width="160" />
    <el-table-column prop="display_name" label="显示名" min-width="160" />
    <el-table-column prop="model_type" label="类型" width="120" />
    <el-table-column label="文件状态" min-width="170">
      <template #default="{ row }">
        <el-tag :type="modelStatusType(row.file_status)">{{ modelStatusLabel(row.file_status) }}</el-tag>
      </template>
    </el-table-column>
    <el-table-column label="节点文件" width="130">
      <template #default="{ row }">{{ row.verified_file_count }} / {{ row.total_file_count }}</template>
    </el-table-column>
    <el-table-column label="可用节点文件" width="120">
      <template #default="{ row }">{{ row.available_node_count }}</template>
    </el-table-column>
    <el-table-column label="最近验证" width="180">
      <template #default="{ row }">{{ formatTime(row.last_file_verified_at) }}</template>
    </el-table-column>
    <el-table-column v-if="role !== 'viewer'" label="操作" width="220" fixed="right">
      <template #default="{ row }">
        <el-button size="small" @click="openEdit(row)">编辑</el-button>
        <el-button size="small" type="danger" @click="remove(row)">删除配置</el-button>
      </template>
    </el-table-column>
  </el-table>

  <el-dialog v-model="dialogVisible" :title="editingId ? '编辑模型' : '新增模型'" width="640px">
    <el-form label-width="120px">
      <!-- 1. 模型名称 -->
      <el-form-item label="模型名称" required>
        <el-input v-model="form.name" placeholder="例如 qwen3-0.6b" />
      </el-form-item>
      <!-- 2. 显示名 -->
      <el-form-item label="显示名">
        <el-input v-model="form.display_name" placeholder="可选" />
      </el-form-item>
      <!-- 3. 类型 -->
      <el-form-item label="类型">
        <el-select v-model="form.model_type">
          <el-option v-for="type in modelTypes" :key="type" :label="type" :value="type" />
        </el-select>
      </el-form-item>

      <!-- New model: node + path -->
      <template v-if="!editingId">
        <el-alert
          title="保存前会由所选节点 Agent 验证模型文件或目录；验证成功后才会创建模型。"
          type="info"
          show-icon
          class="alert"
        />
        <el-alert
          v-if="saving"
          title="正在验证节点文件路径，请等待 Agent 返回结果。"
          type="warning"
          show-icon
          class="alert"
        />
        <el-form-item label="节点" required>
          <el-select v-model="form.initial_node_id" filterable>
            <el-option v-for="node in nodes" :key="node.id" :label="node.name" :value="node.id" />
          </el-select>
        </el-form-item>
        <el-form-item label="模型资产路径" required>
          <el-input v-model="form.initial_path" placeholder="/models/qwen2.5-0.5b 或 /models/model.gguf" />
        </el-form-item>
      </template>

      <!-- 4. 路径类型 -->
      <el-form-item label="路径类型">
        <el-select v-model="modelMeta.path_type">
          <el-option label="目录" value="directory" />
          <el-option label="文件" value="file" />
          <el-option label="Ollama 模型名" value="ollama" />
          <el-option label="自定义" value="custom" />
        </el-select>
      </el-form-item>
      <!-- 5. 模型格式 -->
      <el-form-item label="模型格式">
        <el-select v-model="modelMeta.model_format" @change="onModelFormatChange">
          <el-option label="HuggingFace" value="huggingface" />
          <el-option label="GGUF" value="gguf" />
          <el-option label="Ollama" value="ollama" />
          <el-option label="自定义" value="custom" />
        </el-select>
      </el-form-item>
      <!-- 6. 支持后端 -->
      <el-form-item label="支持后端">
        <el-checkbox-group v-model="modelMeta.supported_backends" @change="onBackendsChange">
          <el-checkbox label="vllm">vLLM</el-checkbox>
          <el-checkbox label="llama_cpp">llama.cpp</el-checkbox>
          <el-checkbox label="ollama">Ollama</el-checkbox>
          <el-checkbox label="custom">自定义</el-checkbox>
        </el-checkbox-group>
      </el-form-item>
      <!-- 7. 服务模型名 -->
      <el-form-item label="服务模型名">
        <el-input v-model="modelMeta.served_model_name" :placeholder="form.name || '默认使用模型名称'" />
        <div class="muted tiny-text" style="margin-top:2px">默认使用模型名称，可在实例中覆盖</div>
      </el-form-item>
      <!-- 8. 描述 -->
      <el-form-item label="描述">
        <el-input v-model="form.description" type="textarea" :rows="2" placeholder="可选" />
      </el-form-item>
      <!-- 9. 高级额外参数 -->
      <el-collapse class="advanced-fields">
        <el-collapse-item title="高级额外参数" name="extra">
          <el-form-item label="额外后端参数">
            <el-input v-model="modelMeta.extra_backend_args" type="textarea" :rows="3" placeholder="每行一个参数，例如：&#10;--tensor-parallel-size&#10;4" />
          </el-form-item>
        </el-collapse-item>
      </el-collapse>
    </el-form>
    <template #footer>
      <el-button @click="dialogVisible = false">取消</el-button>
      <el-button type="primary" :disabled="modelMeta.supported_backends.length === 0" :loading="saving" @click="submit">{{ saving && !editingId ? '验证并保存中' : '保存' }}</el-button>
    </template>
  </el-dialog>

  <el-dialog v-model="fileDialogVisible" :title="editingFileId ? '编辑节点文件路径' : '新增节点文件路径'" width="640px">
    <el-alert
      title="路径验证由所选节点 Agent 执行，支持普通文件和目录；验证不代表模型格式或推理可用。"
      type="info"
      show-icon
      class="alert"
    />
    <el-alert
      v-if="fileSaving"
      title="正在验证节点文件路径，验证成功后才会保存记录。"
      type="warning"
      show-icon
      class="alert"
    />
    <el-form label-width="110px">
      <el-form-item label="模型">
        <el-input :model-value="fileModel?.name" disabled />
      </el-form-item>
      <el-form-item label="节点">
        <el-select v-model="fileForm.node_id" filterable>
          <el-option v-for="node in nodes" :key="node.id" :label="node.name" :value="node.id" />
        </el-select>
      </el-form-item>
      <el-form-item label="模型文件/目录路径">
        <el-input v-model="fileForm.path" placeholder="/models/qwen2.5-0.5b 或 /models/model.gguf" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="fileDialogVisible = false">取消</el-button>
      <el-button type="primary" :loading="fileSaving" @click="submitFile">{{ fileSaving ? '验证并保存中' : '保存' }}</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
defineProps<{ role: string }>()

import { ElMessage } from 'element-plus/es/components/message/index'
import { ElMessageBox } from 'element-plus/es/components/message-box/index'
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import {
  createModel,
  createModelFile,
  deleteModel,
  deleteModelFile,
  fetchModelFiles,
  fetchModelInstances,
  fetchModels,
  fetchNodes,
  updateModel,
  updateModelFile,
  verifyModelFile
} from '../api'
import type { ModelDefinition, ModelFile, ModelInstance, NodeStatus } from '../types'
import { emptyToNull, formatTime } from '../utils/instance'
import {
  assembleModelMeta,
  autoFillFromFormat,
  defaultModelMeta,
  parseModelMeta,
  type ModelMetaFields,
} from '../utils/templates'

const modelTypes = ['llm', 'embedding', 'rerank', 'vlm', 'asr', 'tts', 'other']
const backends = ['vllm', 'ollama', 'lmdeploy', 'mindie', 'llama_cpp', 'triton', 'custom']
const models = ref<ModelDefinition[]>([])
const nodes = ref<NodeStatus[]>([])
const instances = ref<ModelInstance[]>([])
const filesByModel = ref<Record<string, ModelFile[]>>({})
const loading = ref(false)
const saving = ref(false)
const fileSaving = ref(false)
const error = ref('')
const dialogVisible = ref(false)
const fileDialogVisible = ref(false)
const editingId = ref('')
const editingFileId = ref('')
const fileModel = ref<ModelDefinition | null>(null)
const form = ref(emptyForm())
const fileForm = ref({ node_id: '', path: '' })
const verificationTimers = new Map<string, ReturnType<typeof window.setInterval>>()
const modelMeta = reactive<ModelMetaFields>(defaultModelMeta())
let initMeta = false
const modelBackendsTouched = ref(false)
let settingBackends = false

function emptyForm() {
  return {
    name: '',
    display_name: '',
    model_type: 'llm',
    description: '',
    params_json: '',
    initial_node_id: '',
    initial_path: ''
  }
}

/** Auto-fill path_type and supported_backends from model_format. Does NOT overwrite user manual selections unless backends are empty. */
function onModelFormatChange(format: string) {
  if (initMeta) return
  const auto = autoFillFromFormat(format)
  if (!editingId.value) {
    modelMeta.path_type = auto.path_type
  }
  // Fill if user hasn't touched backends, or if they cleared all (empty)
  if (!modelBackendsTouched.value || modelMeta.supported_backends.length === 0) {
    settingBackends = true
    modelMeta.supported_backends = [...auto.supported_backends]
    settingBackends = false
  }
}

/** Mark that the user has manually edited supported_backends (not programmatic fill). */
function onBackendsChange() {
  if (!initMeta && !settingBackends) {
    modelBackendsTouched.value = true
  }
}

/** Apply default backends from current format (only when empty). */
function applyDefaultBackendsForFormat() {
  if (modelMeta.supported_backends.length === 0 && modelMeta.model_format) {
    settingBackends = true
    const auto = autoFillFromFormat(modelMeta.model_format)
    modelMeta.supported_backends = [...auto.supported_backends]
    settingBackends = false
  }
}

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [nextModels, nextNodes, nextInstances] = await Promise.all([fetchModels(), fetchNodes(), fetchModelInstances()])
    models.value = nextModels
    nodes.value = nextNodes
    instances.value = nextInstances
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

async function onExpand(row: ModelDefinition, expandedRows: ModelDefinition[]) {
  if (expandedRows.some((expanded) => expanded.id === row.id)) {
    await loadModelFiles(row.id)
  }
}

async function loadModelFiles(modelId: string) {
  filesByModel.value = {
    ...filesByModel.value,
    [modelId]: await fetchModelFiles(modelId)
  }
}

function openCreate() {
  editingId.value = ''
  form.value = {
    ...emptyForm(),
    initial_node_id: nodes.value[0]?.id ?? ''
  }
  modelBackendsTouched.value = false
  initMeta = true
  Object.assign(modelMeta, { ...defaultModelMeta(), supported_backends: [] })
  initMeta = false
  // Auto-fill backends from initial format (empty → fill)
  applyDefaultBackendsForFormat()
  dialogVisible.value = true
}

function openEdit(row: ModelDefinition) {
  editingId.value = row.id
  form.value = {
    name: row.name,
    display_name: row.display_name ?? '',
    model_type: row.model_type,
    description: row.description ?? '',
    params_json: '',
    initial_node_id: '',
    initial_path: ''
  }
  modelBackendsTouched.value = false
  initMeta = true
  Object.assign(modelMeta, parseModelMeta(row.params_json))
  initMeta = false
  // If saved data has no supported_backends, auto-fill from current format
  applyDefaultBackendsForFormat()
  dialogVisible.value = true
}

async function submit() {
  if (editingId.value) {
    const running = instances.value.filter(
      i => i.model_id === editingId.value && ['running', 'starting', 'stopping'].includes(i.status)
    )
    if (running.length > 0) {
      ElMessage.warning(`模型正在被运行中的实例 ${running.map(i => i.name).join(', ')} 使用，不能修改。请先停止实例。`)
      return
    }
  }
  if (!form.value.name.trim()) {
    ElMessage.error('请填写模型名称')
    return
  }
  if (!editingId.value && !form.value.initial_node_id) {
    ElMessage.error('请选择节点')
    return
  }
  if (!editingId.value && !form.value.initial_path.trim()) {
    ElMessage.error('请填写节点上的模型文件路径')
    return
  }
  const payload = {
    name: form.value.name,
    display_name: emptyToNull(form.value.display_name),
    model_type: form.value.model_type,
    model_path: null,
    description: emptyToNull(form.value.description),
    params_json: assembleModelMeta(modelMeta) || null,
    default_backend: null,
    initial_file: editingId.value
      ? undefined
      : {
          node_id: form.value.initial_node_id,
          path: form.value.initial_path.trim()
        }
  }
  saving.value = true
  try {
    if (editingId.value) {
      await updateModel(editingId.value, payload)
      ElMessage.success('模型配置已保存')
    } else {
      await createModel(payload)
      ElMessage.success('文件已验证，模型已创建')
    }
    dialogVisible.value = false
    await loadData()
  } catch (err) {
    ElMessage.error(toBusinessMessage(err))
  } finally {
    saving.value = false
  }
}

function openFileCreate(row: ModelDefinition) {
  fileModel.value = row
  editingFileId.value = ''
  fileForm.value = {
    node_id: nodes.value[0]?.id ?? '',
    path: ''
  }
  fileDialogVisible.value = true
}

function openFileEdit(model: ModelDefinition, file: ModelFile) {
  fileModel.value = model
  editingFileId.value = file.id
  fileForm.value = {
    node_id: file.node_id,
    path: file.path
  }
  fileDialogVisible.value = true
}

async function submitFile() {
  if (!fileModel.value) return
  if (!fileForm.value.node_id) {
    ElMessage.error('请选择节点')
    return
  }
  if (!fileForm.value.path.trim()) {
    ElMessage.error('请填写节点上的模型文件路径')
    return
  }
  fileSaving.value = true
  try {
    const payload = {
      node_id: fileForm.value.node_id,
      path: fileForm.value.path.trim()
    }
    if (editingFileId.value) {
      await updateModelFile(editingFileId.value, payload)
      ElMessage.success('文件已验证，节点文件路径已保存')
    } else {
      await createModelFile(fileModel.value.id, payload)
      ElMessage.success('文件已验证，节点文件路径已添加')
    }
    fileDialogVisible.value = false
    await loadModelFiles(fileModel.value.id)
    await loadData()
  } catch (err) {
    ElMessage.error(toBusinessMessage(err))
  } finally {
    fileSaving.value = false
  }
}

async function verifyFile(file: ModelFile) {
  try {
    await verifyModelFile(file.id)
    ElMessage.success('已创建文件验证任务，等待节点 Agent 执行')
    await loadModelFiles(file.model_id)
    await loadData()
    startVerificationRefresh(file.model_id)
  } catch (err) {
    ElMessage.error(toBusinessMessage(err))
  }
}

function startVerificationRefresh(modelId: string) {
  stopVerificationRefresh(modelId)
  verificationTimers.set(
    modelId,
    window.setInterval(async () => {
      try {
        await loadModelFiles(modelId)
        await loadData()
        const files = filesByModel.value[modelId] ?? []
        if (!files.some((file) => isVerificationActive(file.status))) {
          stopVerificationRefresh(modelId)
        }
      } catch (err) {
        stopVerificationRefresh(modelId)
        ElMessage.error(toBusinessMessage(err))
      }
    }, 3000)
  )
}

function stopVerificationRefresh(modelId: string) {
  const timer = verificationTimers.get(modelId)
  if (timer) {
    window.clearInterval(timer)
    verificationTimers.delete(modelId)
  }
}

function isVerificationActive(status: string) {
  return status === 'verify_pending' || status === 'verifying'
}

async function removeFile(model: ModelDefinition, file: ModelFile) {
  await ElMessageBox.confirm(
    `从模型中删除该节点文件路径？该操作会将 ${file.path} 加入模型垃圾箱，不会立即删除真实文件。后续可在模型垃圾箱中执行“删除文件”或“删除记录”。`,
    '确认删除',
    {
      type: 'warning',
      confirmButtonText: '确认删除',
      cancelButtonText: '取消'
    }
  )
  try {
    await deleteModelFile(file.id)
    ElMessage.success('节点文件路径已移入模型垃圾箱')
    await loadModelFiles(model.id)
    await loadData()
  } catch (err) {
    ElMessage.error(toBusinessMessage(err))
  }
}

async function remove(row: ModelDefinition) {
  await ElMessageBox.confirm(
    `删除模型配置 ${row.name}？删除后模型配置将不再显示，关联的所有节点文件路径将进入模型垃圾箱，真实文件不会立即删除。如需物理删除，需要到模型垃圾箱中逐条执行“删除文件”。`,
    '确认删除模型配置',
    {
      type: 'warning',
      confirmButtonText: '确认删除',
      cancelButtonText: '取消'
    }
  )
  try {
    await deleteModel(row.id)
    ElMessage.success('模型配置已删除，关联路径已进入模型垃圾箱')
    await loadData()
  } catch (err) {
    ElMessage.error(toBusinessMessage(err))
  }
}

function modelStatusLabel(status: string) {
  const labels: Record<string, string> = {
    no_files: '未配置文件',
    pending_verification: '待验证',
    partially_verified: '部分节点文件已验证',
    all_files_verified: '全部节点文件已验证',
    verification_failed: '验证失败'
  }
  return labels[status] ?? status
}

function modelStatusType(status: string) {
  if (status === 'all_files_verified') return 'success'
  if (status === 'partially_verified') return 'warning'
  if (status === 'verification_failed') return 'danger'
  return 'info'
}

function fileStatusLabel(status: string) {
  const labels: Record<string, string> = {
    unverified: '未验证',
    verify_pending: '等待验证',
    verifying: '验证中',
    verified: '文件已验证',
    missing: '文件不存在',
    invalid_path: '路径非法',
    not_file: '不是普通文件',
    agent_offline: 'Agent 离线',
    verify_timeout: '验证超时',
    failed: '验证失败'
  }
  return labels[status] ?? status
}

function taskStatusLabel(status: string) {
  const labels: Record<string, string> = {
    queued: '等待执行',
    running: '执行中',
    succeeded: '已完成',
    failed: '失败',
    timed_out: '超时'
  }
  return labels[status] ?? status
}

function fileStatusType(status: string) {
  if (status === 'verified') return 'success'
  if (status === 'verify_pending' || status === 'verifying' || status === 'unverified') return 'warning'
  return 'danger'
}

function nodeStatusLabel(status: string) {
  if (status === 'online') return '在线'
  if (status === 'offline') return '离线'
  return '已注册'
}

function toBusinessMessage(err: unknown) {
  const message = err instanceof Error ? err.message : '操作失败'
  if (message.includes('starting or running')) {
    return '模型存在启动中或运行中的实例，不能删除'
  }
  if (message.includes('trash records')) {
    return '该节点文件路径已有垃圾箱记录，不能直接删除记录'
  }
  if (message.includes('initial_file is required')) {
    return '新增模型时必须配置至少一个节点文件路径'
  }
  if (message.includes('模型名称已存在') || message.includes('UNIQUE constraint failed: models.name')) {
    return '模型名称已存在，请使用其他名称'
  }
  if (message.includes('node not found')) {
    return '节点不存在，请刷新后重试'
  }
  return message
}

function formatBytes(value?: number | null) {
  if (value == null) return '-'
  if (value < 1024) return `${value} B`
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`
  if (value < 1024 * 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MiB`
  return `${(value / 1024 / 1024 / 1024).toFixed(1)} GiB`
}

onMounted(loadData)
onUnmounted(() => {
  for (const modelId of verificationTimers.keys()) {
    stopVerificationRefresh(modelId)
  }
})
defineExpose({ refresh: loadData })
</script>
