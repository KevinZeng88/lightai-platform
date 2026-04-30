<template>
  <section class="panel-header">
    <div>
      <h2>External 模型实例</h2>
      <p>接入已有模型服务；Docker / Script 才是在节点上部署模型服务。</p>
    </div>
    <div class="toolbar compact">
      <el-button :loading="loading" @click="loadData">刷新</el-button>
      <el-button type="primary" @click="openCreate">新增 External 实例</el-button>
    </div>
  </section>

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-table :data="instances" row-key="id" border>
    <el-table-column prop="name" label="实例" min-width="150" fixed="left" />
    <el-table-column label="状态" width="120">
      <template #default="{ row }">
        <el-tag :type="statusType(row.status)">{{ row.status }}</el-tag>
      </template>
    </el-table-column>
    <el-table-column label="检查结果" min-width="220">
      <template #default="{ row }">
        <span>{{ row.last_error ?? '-' }}</span>
        <span v-if="row.last_checked_at" class="muted"> · {{ formatTime(row.last_checked_at) }}</span>
      </template>
    </el-table-column>
    <el-table-column label="模型定义" min-width="150">
      <template #default="{ row }">{{ row.model_definition_name ?? row.model_id }}</template>
    </el-table-column>
    <el-table-column prop="model_name" label="服务模型名" min-width="140" />
    <el-table-column prop="backend" label="后端" width="120" />
    <el-table-column prop="runtime_version" label="版本" width="120" />
    <el-table-column prop="base_url" label="Base URL" min-width="220" show-overflow-tooltip />
    <el-table-column prop="health_url" label="Health URL" min-width="240" show-overflow-tooltip />
    <el-table-column prop="endpoint_url" label="Endpoint" min-width="220" show-overflow-tooltip />
    <el-table-column label="操作" width="230" fixed="right">
      <template #default="{ row }">
        <el-button size="small" @click="check(row)">检查状态</el-button>
        <el-button size="small" @click="openEdit(row)">编辑</el-button>
        <el-button size="small" type="danger" @click="remove(row)">删除</el-button>
      </template>
    </el-table-column>
  </el-table>

  <el-dialog v-model="dialogVisible" :title="editingId ? '编辑 External 实例' : '新增 External 实例'" width="700px">
    <el-alert
      title="External 用于接入已有模型服务，不需要先登记运行环境，也不要求绑定节点。"
      type="info"
      show-icon
      class="alert"
    />
    <el-form label-width="120px">
      <el-form-item label="模型定义">
        <el-select v-model="form.model_id" filterable :disabled="Boolean(editingId)">
          <el-option v-for="model in models" :key="model.id" :label="model.name" :value="model.id" />
        </el-select>
      </el-form-item>
      <el-form-item label="实例名称">
        <el-input v-model="form.name" />
      </el-form-item>
      <el-form-item label="后端">
        <el-select v-model="form.backend">
          <el-option v-for="backend in backends" :key="backend" :label="backend" :value="backend" />
        </el-select>
      </el-form-item>
      <el-form-item label="服务模型名">
        <el-input v-model="form.model_name" placeholder="例如 local-gguf" />
      </el-form-item>
      <el-form-item label="版本">
        <el-input v-model="form.runtime_version" />
      </el-form-item>
      <el-form-item label="Base URL">
        <el-input v-model="form.base_url" placeholder="http://127.0.0.1:8088" />
      </el-form-item>
      <el-form-item label="Health URL">
        <el-input v-model="form.health_url" placeholder="http://127.0.0.1:8088/v1/models" />
      </el-form-item>
      <el-form-item label="Endpoint URL">
        <el-input v-model="form.endpoint_url" placeholder="http://127.0.0.1:8088/v1" />
      </el-form-item>
      <el-form-item label="备注">
        <el-input v-model="form.description" type="textarea" :rows="2" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="dialogVisible = false">取消</el-button>
      <el-button type="primary" @click="submit">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { ElMessage, ElMessageBox, ElNotification } from 'element-plus'
import { onMounted, ref } from 'vue'
import {
  checkModelInstance,
  createModelInstance,
  deleteModelInstance,
  fetchModelInstances,
  fetchModels,
  updateModelInstance
} from '../api'
import type { ModelDefinition, ModelInstance } from '../types'

const backends = ['vllm', 'ollama', 'lmdeploy', 'mindie', 'llama_cpp', 'triton', 'custom']
const models = ref<ModelDefinition[]>([])
const instances = ref<ModelInstance[]>([])
const loading = ref(false)
const error = ref('')
const dialogVisible = ref(false)
const editingId = ref('')
const form = ref(emptyForm())

function emptyForm() {
  return {
    model_id: '',
    name: '',
    backend: 'llama_cpp',
    model_name: '',
    runtime_version: '',
    base_url: '',
    endpoint_url: '',
    health_url: '',
    description: ''
  }
}

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [nextModels, nextInstances] = await Promise.all([fetchModels(), fetchModelInstances()])
    models.value = nextModels
    instances.value = nextInstances
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

function openCreate() {
  editingId.value = ''
  form.value = { ...emptyForm(), model_id: models.value[0]?.id ?? '' }
  dialogVisible.value = true
}

function openEdit(row: ModelInstance) {
  editingId.value = row.id
  form.value = {
    model_id: row.model_id,
    name: row.name,
    backend: row.backend,
    model_name: row.model_name ?? '',
    runtime_version: row.runtime_version ?? '',
    base_url: row.base_url ?? '',
    endpoint_url: row.endpoint_url ?? '',
    health_url: row.health_url ?? '',
    description: row.description ?? ''
  }
  dialogVisible.value = true
}

async function submit() {
  if (!form.value.model_id || !form.value.name || !form.value.backend) return
  if (!form.value.base_url && !form.value.endpoint_url && !form.value.health_url) {
    error.value = '至少填写 Base URL、Endpoint URL 或 Health URL 中的一个'
    return
  }
  const payload = {
    model_id: form.value.model_id,
    name: form.value.name,
    backend: form.value.backend,
    base_url: emptyToNull(form.value.base_url),
    endpoint_url: emptyToNull(form.value.endpoint_url),
    health_url: emptyToNull(form.value.health_url),
    runtime_version: emptyToNull(form.value.runtime_version),
    model_name: emptyToNull(form.value.model_name),
    description: emptyToNull(form.value.description),
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
    title: `检查结果：${checked.status}`,
    message: checked.last_error ?? formatTime(checked.last_checked_at),
    type: checked.status === 'running' ? 'success' : checked.status === 'failed' ? 'error' : 'warning'
  })
  await loadData()
}

async function remove(row: ModelInstance) {
  await ElMessageBox.confirm(`删除实例 ${row.name}？`, '确认删除', { type: 'warning' })
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

function emptyToNull(value: string) {
  return value.trim() ? value.trim() : null
}

function formatTime(value?: number | null) {
  if (!value) return '-'
  return new Date(value * 1000).toLocaleString()
}

onMounted(loadData)
</script>
