<template>
  <section class="panel-header">
    <div>
      <h2>模型定义</h2>
      <p>管理模型配置；删除配置不会删除磁盘模型文件。</p>
    </div>
    <div class="toolbar compact">
      <el-button :loading="loading" @click="loadData">刷新</el-button>
      <el-button type="primary" @click="openCreate">新增模型</el-button>
    </div>
  </section>

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-table :data="models" row-key="id" border>
    <el-table-column type="expand">
      <template #default="{ row }">
        <div class="detail-grid">
          <div><span class="muted">模型路径</span><p>{{ row.model_path ?? '-' }}</p></div>
          <div><span class="muted">描述</span><p>{{ row.description ?? '-' }}</p></div>
          <div><span class="muted">默认后端</span><p>{{ row.default_backend ?? '-' }}</p></div>
        </div>
      </template>
    </el-table-column>
    <el-table-column prop="name" label="名称" min-width="160" />
    <el-table-column prop="display_name" label="显示名" min-width="160" />
    <el-table-column prop="model_type" label="类型" width="120" />
    <el-table-column prop="default_backend" label="默认后端" width="130" />
    <el-table-column label="更新时间" width="190">
      <template #default="{ row }">{{ formatTime(row.updated_at) }}</template>
    </el-table-column>
    <el-table-column label="操作" width="300" fixed="right">
      <template #default="{ row }">
        <el-button size="small" @click="openTrash(row)">加入垃圾箱</el-button>
        <el-button size="small" @click="openEdit(row)">编辑</el-button>
        <el-button size="small" type="danger" @click="remove(row)">删除配置</el-button>
      </template>
    </el-table-column>
  </el-table>

  <el-dialog v-model="dialogVisible" :title="editingId ? '编辑模型' : '新增模型'" width="640px">
    <el-form label-width="110px">
      <el-form-item label="名称">
        <el-input v-model="form.name" />
      </el-form-item>
      <el-form-item label="显示名">
        <el-input v-model="form.display_name" />
      </el-form-item>
      <el-form-item label="类型">
        <el-select v-model="form.model_type">
          <el-option v-for="type in modelTypes" :key="type" :label="type" :value="type" />
        </el-select>
      </el-form-item>
      <el-form-item label="模型路径">
        <el-input v-model="form.model_path" />
      </el-form-item>
      <el-form-item label="默认后端">
        <el-select v-model="form.default_backend" clearable>
          <el-option v-for="backend in backends" :key="backend" :label="backend" :value="backend" />
        </el-select>
      </el-form-item>
      <el-form-item label="描述">
        <el-input v-model="form.description" type="textarea" :rows="3" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="dialogVisible = false">取消</el-button>
      <el-button type="primary" @click="submit">保存</el-button>
    </template>
  </el-dialog>

  <el-dialog v-model="trashVisible" title="加入模型文件垃圾箱" width="620px">
    <el-alert
      title="这里只登记待清理路径，不会立即删除磁盘文件。后续物理清理必须由 Agent 在受控目录内执行。"
      type="warning"
      show-icon
      class="alert"
    />
    <el-form label-width="110px">
      <el-form-item label="模型">
        <el-input :model-value="trashModel?.name" disabled />
      </el-form-item>
      <el-form-item label="文件路径">
        <el-input v-model="trashForm.path" />
      </el-form-item>
      <el-form-item label="原因">
        <el-input v-model="trashForm.reason" />
      </el-form-item>
      <el-form-item label="备注">
        <el-input v-model="trashForm.note" type="textarea" :rows="2" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="trashVisible = false">取消</el-button>
      <el-button type="primary" @click="submitTrash">登记</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
import { ElMessage, ElMessageBox } from 'element-plus'
import { onMounted, ref } from 'vue'
import { addModelFileTrash, createModel, deleteModel, fetchModels, updateModel } from '../api'
import type { ModelDefinition } from '../types'

const modelTypes = ['llm', 'embedding', 'rerank', 'vlm', 'asr', 'tts', 'other']
const backends = ['vllm', 'ollama', 'lmdeploy', 'mindie', 'llama_cpp', 'triton', 'custom']
const models = ref<ModelDefinition[]>([])
const loading = ref(false)
const error = ref('')
const dialogVisible = ref(false)
const trashVisible = ref(false)
const editingId = ref('')
const trashModel = ref<ModelDefinition | null>(null)
const form = ref(emptyForm())
const trashForm = ref({ path: '', reason: '', note: '' })

function emptyForm() {
  return {
    name: '',
    display_name: '',
    model_type: 'llm',
    model_path: '',
    description: '',
    default_backend: ''
  }
}

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    models.value = await fetchModels()
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

function openEdit(row: ModelDefinition) {
  editingId.value = row.id
  form.value = {
    name: row.name,
    display_name: row.display_name ?? '',
    model_type: row.model_type,
    model_path: row.model_path ?? '',
    description: row.description ?? '',
    default_backend: row.default_backend ?? ''
  }
  dialogVisible.value = true
}

async function submit() {
  if (!form.value.name) return
  const payload = {
    name: form.value.name,
    display_name: emptyToNull(form.value.display_name),
    model_type: form.value.model_type,
    model_path: emptyToNull(form.value.model_path),
    description: emptyToNull(form.value.description),
    default_backend: emptyToNull(form.value.default_backend)
  }
  if (editingId.value) {
    await updateModel(editingId.value, payload)
  } else {
    await createModel(payload)
  }
  dialogVisible.value = false
  await loadData()
}

async function remove(row: ModelDefinition) {
  await ElMessageBox.confirm(
    `删除模型配置 ${row.name}？此操作不会删除磁盘模型文件。`,
    '确认删除模型配置',
    { type: 'warning' }
  )
  await deleteModel(row.id)
  ElMessage.success('模型配置已删除')
  await loadData()
}

function openTrash(row: ModelDefinition) {
  trashModel.value = row
  trashForm.value = {
    path: row.model_path ?? '',
    reason: '',
    note: ''
  }
  trashVisible.value = true
}

async function submitTrash() {
  if (!trashModel.value || !trashForm.value.path) return
  await addModelFileTrash(trashModel.value.id, {
    path: trashForm.value.path,
    reason: emptyToNull(trashForm.value.reason),
    note: emptyToNull(trashForm.value.note)
  })
  trashVisible.value = false
  ElMessage.success('已加入模型文件垃圾箱')
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
