<template>
  <section class="panel-header">
    <div>
      <h2>模型文件垃圾箱</h2>
      <p>仅登记待清理模型文件路径；Stage 3A 不会物理删除磁盘文件。</p>
    </div>
    <el-button :loading="loading" @click="loadData">刷新</el-button>
  </section>

  <el-alert
    title="这里是待清理入口，不是删除执行器。未来物理删除必须由 Agent 在受控目录范围内完成。"
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
    <el-table-column prop="status" label="状态" width="120" />
    <el-table-column prop="note" label="备注" min-width="180" show-overflow-tooltip />
    <el-table-column label="登记时间" width="190">
      <template #default="{ row }">{{ formatTime(row.created_at) }}</template>
    </el-table-column>
  </el-table>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { fetchModelFileTrash } from '../api'
import type { ModelFileTrashItem } from '../types'

const items = ref<ModelFileTrashItem[]>([])
const loading = ref(false)
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

function formatTime(value?: number | null) {
  if (!value) return '-'
  return new Date(value * 1000).toLocaleString()
}

onMounted(loadData)
</script>
