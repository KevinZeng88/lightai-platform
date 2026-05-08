<template>
  <section class="overview">
    <div>
      <p class="eyebrow">LightAI Platform</p>
      <h1>GPU 采集器登记</h1>
      <p class="summary">登记可信 collector 脚本 hash，Agent 仅执行 hash 匹配的脚本。</p>
    </div>
  </section>

  <el-alert
    title="脚本必须由运维放到 Agent 本地 collector 目录。本页面只登记 inspect 命令输出的可信 hash，不下发脚本内容。"
    type="warning"
    show-icon
    class="alert"
  />

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-card shadow="never" class="section-card">
    <template #header>
      <div class="card-header-row">
        <span>已登记采集器</span>
        <el-button v-if="role === 'admin'" type="primary" @click="openRegister">登记新采集器</el-button>
      </div>
    </template>
    <el-table :data="entries" row-key="id" border size="small">
      <el-table-column prop="id" label="ID" min-width="120" />
      <el-table-column prop="vendor" label="厂商" width="100" />
      <el-table-column prop="name" label="名称" min-width="180" />
      <el-table-column prop="version" label="版本" width="90" />
      <el-table-column v-if="role === 'admin'" label="enabled" width="90">
        <template #default="{ row }">
          <el-switch
            :model-value="row.enabled"
            @change="(val: boolean) => toggleEnabled(row, val)"
          />
        </template>
      </el-table-column>
      <el-table-column label="discover SHA256" min-width="140">
        <template #default="{ row }">
          <code class="hash-cell">{{ row.discover_sha256.slice(0, 16) }}…</code>
        </template>
      </el-table-column>
      <el-table-column label="metrics SHA256" min-width="140">
        <template #default="{ row }">
          <code class="hash-cell">{{ row.metrics_sha256.slice(0, 16) }}…</code>
        </template>
      </el-table-column>
      <el-table-column v-if="role === 'admin'" label="操作" width="80" fixed="right">
        <template #default="{ row }">
          <el-button size="small" @click="openEdit(row)">修改</el-button>
          <el-button size="small" type="danger" @click="removeEntry(row)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>
    <el-empty v-if="!loading && entries.length === 0" description="暂无已登记的采集器" :image-size="80" />
  </el-card>

  <el-dialog v-model="dialogVisible" title="登记采集器" width="640px">
    <el-alert
      title="在 Agent 机器上执行 lightai-agent collector inspect &lt;collector_dir&gt;，将输出的 JSON 粘贴到下方。"
      type="info"
      show-icon
      class="alert"
    />
    <el-input
      v-model="inspectJson"
      type="textarea"
      :rows="12"
      placeholder='粘贴 collector inspect 输出的 JSON …'
      style="margin-top: 12px"
    />
    <template #footer>
      <el-button @click="dialogVisible = false">取消</el-button>
      <el-button type="primary" :loading="saving" @click="submitRegister">登记</el-button>
    </template>
  </el-dialog>

  <el-dialog v-model="editDialogVisible" title="修改采集器" width="640px">
    <el-form label-width="110px">
      <el-form-item label="ID">
        <el-input :model-value="editForm.id" disabled />
      </el-form-item>
      <el-form-item label="厂商">
        <el-input v-model="editForm.vendor" />
      </el-form-item>
      <el-form-item label="名称">
        <el-input v-model="editForm.name" />
      </el-form-item>
      <el-form-item label="版本">
        <el-input v-model="editForm.version" />
      </el-form-item>
      <el-form-item label="描述">
        <el-input v-model="editForm.description" />
      </el-form-item>
      <el-form-item label="discover SHA256">
        <el-input v-model="editForm.discover_sha256" />
      </el-form-item>
      <el-form-item label="metrics SHA256">
        <el-input v-model="editForm.metrics_sha256" />
      </el-form-item>
      <el-form-item label="启用">
        <el-switch v-model="editForm.enabled" />
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="editDialogVisible = false">取消</el-button>
      <el-button type="primary" :loading="saving" @click="submitEdit">保存</el-button>
    </template>
  </el-dialog>
</template>

<script setup lang="ts">
defineProps<{ role: string }>()

import { ElMessage, ElMessageBox } from 'element-plus'
import { onMounted, ref } from 'vue'
import {
  deleteCollector,
  fetchCollectorRegistry,
  registerCollector,
  type CollectorRegistryEntry
} from '../api'

const entries = ref<CollectorRegistryEntry[]>([])
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const dialogVisible = ref(false)
const inspectJson = ref('')

async function load() {
  loading.value = true
  error.value = ''
  try {
    entries.value = await fetchCollectorRegistry()
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

function openRegister() {
  inspectJson.value = ''
  dialogVisible.value = true
}

async function submitRegister() {
  if (!inspectJson.value.trim()) {
    ElMessage.error('请粘贴 collector inspect 输出的 JSON')
    return
  }
  let parsed: Record<string, unknown>
  try {
    parsed = JSON.parse(inspectJson.value)
  } catch {
    ElMessage.error('JSON 格式无效')
    return
  }
  const id = parsed.id
  const version = parsed.version
  if (typeof id !== 'string' || !id || typeof version !== 'string' || !version) {
    ElMessage.error('JSON 缺少必需的 id 或 version 字段')
    return
  }
  saving.value = true
  try {
    await registerCollector({
      id,
      vendor: String(parsed.vendor ?? ''),
      name: String(parsed.name ?? ''),
      version,
      description: String(parsed.description ?? ''),
      discover_sha256: String(parsed.discover_sha256 ?? ''),
      metrics_sha256: String(parsed.metrics_sha256 ?? ''),
      enabled: true
    })
    ElMessage.success('采集器已登记')
    dialogVisible.value = false
    await load()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '登记失败')
  } finally {
    saving.value = false
  }
}

async function toggleEnabled(row: CollectorRegistryEntry, enabled: boolean) {
  try {
    await registerCollector({ ...row, enabled })
    row.enabled = enabled
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '更新失败')
  }
}

// ── Edit dialog ──

const editDialogVisible = ref(false)
const editForm = ref({
  id: '',
  vendor: '',
  name: '',
  version: '',
  description: '',
  discover_sha256: '',
  metrics_sha256: '',
  enabled: true
})

function openEdit(row: CollectorRegistryEntry) {
  editForm.value = {
    id: row.id,
    vendor: row.vendor,
    name: row.name,
    version: row.version,
    description: row.description ?? '',
    discover_sha256: row.discover_sha256,
    metrics_sha256: row.metrics_sha256,
    enabled: row.enabled
  }
  editDialogVisible.value = true
}

async function submitEdit() {
  saving.value = true
  try {
    await registerCollector({
      id: editForm.value.id,
      vendor: editForm.value.vendor,
      name: editForm.value.name,
      version: editForm.value.version,
      description: editForm.value.description,
      discover_sha256: editForm.value.discover_sha256,
      metrics_sha256: editForm.value.metrics_sha256,
      enabled: editForm.value.enabled
    })
    ElMessage.success('采集器已更新')
    editDialogVisible.value = false
    await load()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '更新失败')
  } finally {
    saving.value = false
  }
}

async function removeEntry(row: CollectorRegistryEntry) {
  await ElMessageBox.confirm(
    `确认删除采集器 ${row.id} v${row.version}？删除后 Agent 将无法执行该采集器脚本。`,
    '确认删除',
    { type: 'warning', confirmButtonText: '确认删除', cancelButtonText: '取消' }
  )
  try {
    await deleteCollector(row.id, row.version)
    ElMessage.success('采集器已删除')
    await load()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '删除失败')
  }
}

onMounted(load)
defineExpose({ refresh: load })
</script>

<style scoped>
.hash-cell {
  font-size: 12px;
  user-select: all;
}
</style>
