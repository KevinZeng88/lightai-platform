<template>
  <section class="panel-header">
    <div>
      <h2>模型垃圾箱</h2>
      <p>模型垃圾箱记录对应具体节点上的具体模型文件路径；删除文件由节点 Agent 执行，删除记录只移除平台记录。</p>
    </div>
    <el-button :loading="loading" @click="loadData">刷新</el-button>
  </section>

  <el-alert
    title="清理文件会物理删除节点上的文件且不可恢复；删除记录不会删除任何真实文件。"
    type="warning"
    show-icon
    class="alert"
  />
  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-table :data="items" row-key="id" border>
    <el-table-column prop="path" label="路径" min-width="280" show-overflow-tooltip />
    <el-table-column label="模型" min-width="150">
      <template #default="{ row }">{{ row.model_name ?? row.model_id ?? '-' }}</template>
    </el-table-column>
    <el-table-column label="节点" min-width="140">
      <template #default="{ row }">{{ row.node_name ?? row.node_id ?? '-' }}</template>
    </el-table-column>
    <el-table-column prop="reason" label="原因" min-width="180" show-overflow-tooltip />
    <el-table-column label="状态" width="140">
      <template #default="{ row }">
        <el-tag :type="statusType(row.status)">{{ statusLabel(row.status) }}</el-tag>
      </template>
    </el-table-column>
    <el-table-column label="文件清理时间" width="190">
      <template #default="{ row }">{{ formatTime(row.file_deleted_at) }}</template>
    </el-table-column>
    <el-table-column prop="last_error" label="失败原因" min-width="180" show-overflow-tooltip />
    <el-table-column prop="note" label="备注" min-width="180" show-overflow-tooltip />
    <el-table-column label="最近操作" width="190">
      <template #default="{ row }">{{ formatTime(row.updated_at) }}</template>
    </el-table-column>
    <el-table-column label="操作" width="210" fixed="right">
      <template #default="{ row }">
        <el-button size="small" type="danger" :loading="cleaningId === row.id" @click="cleanupFile(row)">
          删除文件
        </el-button>
        <el-button size="small" @click="removeRecord(row)">删除记录</el-button>
      </template>
    </el-table-column>
  </el-table>
</template>

<script setup lang="ts">
import { ElMessage, ElMessageBox } from 'element-plus'
import { onMounted, ref } from 'vue'
import { cleanupModelFileTrash, deleteModelFileTrash, fetchModelFileTrash } from '../api'
import type { ModelFileTrashItem } from '../types'

const items = ref<ModelFileTrashItem[]>([])
const loading = ref(false)
const cleaningId = ref('')
const error = ref('')

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    items.value = await fetchModelFileTrash()
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

async function cleanupFile(row: ModelFileTrashItem) {
  await ElMessageBox.confirm(
    `清理文件会由节点 Agent 物理删除 ${row.path}，删除后不可恢复。确认继续？`,
    '确认删除文件',
    {
      type: 'warning',
      confirmButtonText: '确认删除文件',
      cancelButtonText: '取消'
    }
  )
  cleaningId.value = row.id
  try {
    await cleanupModelFileTrash(row.id)
    ElMessage.success('文件已清理')
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '文件清理失败')
  } finally {
    cleaningId.value = ''
    await loadData()
  }
}

async function removeRecord(row: ModelFileTrashItem) {
  const message =
    row.status === 'cleaned'
      ? '删除记录只会从模型垃圾箱移除这条记录，文件已标记为清理完成。确认继续？'
      : '删除记录只会从模型垃圾箱移除这条记录，不会删除节点上的真实文件。确认继续？'
  await ElMessageBox.confirm(message, '确认删除记录', {
    type: 'warning',
    confirmButtonText: '确认删除记录',
    cancelButtonText: '取消'
  })
  try {
    await deleteModelFileTrash(row.id)
    ElMessage.success('模型垃圾箱记录已删除')
    await loadData()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '删除记录失败')
  }
}

function statusLabel(status: string) {
  const labels: Record<string, string> = {
    pending: '待处理',
    cleanup_pending: '等待 Agent',
    cleanup_running: '清理中',
    cleaned: '文件已清理',
    cleanup_failed: '清理失败',
    cleanup_timeout: '清理超时'
  }
  return labels[status] ?? status
}

function statusType(status: string) {
  if (status === 'cleaned') return 'success'
  if (status === 'cleanup_failed' || status === 'cleanup_timeout') return 'danger'
  if (status === 'cleanup_pending' || status === 'cleanup_running') return 'warning'
  return 'info'
}

function formatTime(value?: number | null) {
  if (value == null) return '-'
  return new Date(value * 1000).toLocaleString()
}

onMounted(loadData)
defineExpose({ refresh: loadData })
</script>
