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
        <el-button type="primary" @click="openRegister">登记新采集器</el-button>
      </div>
    </template>
    <el-table :data="entries" row-key="id" border size="small">
      <el-table-column prop="id" label="ID" min-width="120" />
      <el-table-column prop="vendor" label="厂商" width="100" />
      <el-table-column prop="name" label="名称" min-width="180" />
      <el-table-column prop="version" label="版本" width="90" />
      <el-table-column label="enabled" width="90">
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
</template>

<script setup lang="ts">
import { ElMessage } from 'element-plus'
import { onMounted, ref } from 'vue'
import {
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

onMounted(load)
defineExpose({ refresh: load })
</script>

<style scoped>
.hash-cell {
  font-size: 12px;
  user-select: all;
}
</style>
