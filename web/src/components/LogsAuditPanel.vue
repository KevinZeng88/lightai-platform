<template>
  <section class="panel-header">
    <div>
      <h2>日志审计</h2>
      <p>查看平台受控日志、实例日志、最近错误摘要和操作审计记录。</p>
    </div>
    <el-button :loading="loading" type="primary" @click="refresh">刷新</el-button>
  </section>

  <el-alert
    title="日志查看只读取平台管理的 Server/Agent/实例日志；不支持输入任意文件路径。Agent 离线时日志读取会返回明确提示。"
    type="info"
    show-icon
    class="alert"
  />
  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-tabs v-model="activeView">
    <el-tab-pane label="日志配置" name="config">
      <el-card shadow="never" class="section-card">
        <template #header>Server 日志策略</template>
        <el-alert
          title="Server 日志策略保存后立即用于平台日志写入和日志读取；Agent 日志策略请在“配置”页按全局或节点覆盖下发。"
          type="info"
          show-icon
          class="alert"
        />
        <el-form label-width="150px" class="config-form">
          <el-form-item label="日志目录">
            <el-input v-model="serverLogPolicy.log_dir" />
          </el-form-item>
          <el-form-item label="日志级别">
            <el-select v-model="serverLogPolicy.log_level">
              <el-option label="error" value="error" />
              <el-option label="warn" value="warn" />
              <el-option label="info" value="info" />
              <el-option label="debug" value="debug" />
              <el-option label="trace" value="trace" />
            </el-select>
          </el-form-item>
          <el-form-item label="单文件上限">
            <el-input-number v-model="serverLogPolicy.log_max_file_bytes" :min="1" :max="1073741824" />
          </el-form-item>
          <el-form-item label="保留文件数">
            <el-input-number v-model="serverLogPolicy.log_retention_files" :min="1" :max="100" />
          </el-form-item>
          <el-form-item label="保留天数">
            <el-input-number v-model="serverLogPolicy.log_retention_days" :min="0" :max="3650" />
          </el-form-item>
          <el-form-item>
            <el-button type="primary" :loading="savingServerPolicy" @click="saveServerLogPolicy">保存 Server 日志策略</el-button>
          </el-form-item>
        </el-form>
      </el-card>
    </el-tab-pane>

    <el-tab-pane label="系统/实例日志" name="logs">
      <el-form inline class="toolbar">
        <el-form-item label="日志类型">
          <el-select v-model="logSource" class="filter-select">
            <el-option label="Server 系统日志" value="server" />
            <el-option label="Agent 系统日志" value="agent" />
            <el-option label="本地模型实例日志" value="instance" />
            <el-option label="最近错误摘要" value="errors" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="logSource === 'agent'" label="节点">
          <el-select v-model="selectedNodeId" filterable class="filter-select">
            <el-option v-for="node in nodes" :key="node.id" :label="node.name" :value="node.id" />
          </el-select>
        </el-form-item>
        <el-form-item v-if="logSource === 'instance'" label="实例">
          <el-select v-model="selectedInstanceId" filterable class="wide-select">
            <el-option
              v-for="instance in instances"
              :key="instance.id"
              :label="`${instance.name} (${instance.status})`"
              :value="instance.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item label="读取上限">
          <el-input-number v-model="maxBytes" :min="1024" :max="524288" :step="8192" />
        </el-form-item>
        <el-form-item>
          <el-button :loading="loadingLog" type="primary" @click="loadLogs">读取日志</el-button>
        </el-form-item>
      </el-form>
      <pre class="log-box">{{ logContent || '暂无日志' }}</pre>
    </el-tab-pane>

    <el-tab-pane label="审计日志" name="audit">
      <el-form inline class="toolbar">
        <el-form-item label="操作类型">
          <el-input v-model="auditFilters.operation_type" placeholder="例如 instance.start" clearable />
        </el-form-item>
        <el-form-item label="目标类型">
          <el-input v-model="auditFilters.target_type" placeholder="例如 model" clearable />
        </el-form-item>
        <el-form-item label="结果">
          <el-select v-model="auditFilters.result" clearable class="filter-select">
            <el-option label="成功" value="success" />
            <el-option label="失败" value="failed" />
          </el-select>
        </el-form-item>
        <el-form-item>
          <el-button :loading="loadingAudit" type="primary" @click="loadAudit">筛选</el-button>
        </el-form-item>
      </el-form>
      <el-table :data="auditEvents" row-key="id" border>
        <el-table-column label="时间" width="180">
          <template #default="{ row }">{{ formatTime(row.occurred_at) }}</template>
        </el-table-column>
        <el-table-column prop="operation_type" label="操作" min-width="150" />
        <el-table-column prop="target_type" label="目标类型" width="130" />
        <el-table-column prop="target_id" label="目标 ID" min-width="220" show-overflow-tooltip />
        <el-table-column prop="node_id" label="节点" min-width="180" show-overflow-tooltip />
        <el-table-column prop="instance_id" label="实例" min-width="180" show-overflow-tooltip />
        <el-table-column label="结果" width="100">
          <template #default="{ row }">
            <el-tag :type="row.result === 'success' ? 'success' : 'danger'">{{ row.result === 'success' ? '成功' : '失败' }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="error_message" label="错误原因" min-width="220" show-overflow-tooltip />
        <el-table-column label="来源" width="130">
          <template #default="{ row }">{{ row.actor_type }}/{{ row.actor_id ?? 'local' }}</template>
        </el-table-column>
      </el-table>
    </el-tab-pane>
  </el-tabs>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import {
  fetchAuditEvents,
  fetchLogs,
  fetchModelInstances,
  fetchNodes,
  fetchServerLogPolicy,
  updateServerLogPolicy
} from '../api'
import type { AuditEvent, LogPolicy, ModelInstance, NodeStatus } from '../types'

const nodes = ref<NodeStatus[]>([])
const instances = ref<ModelInstance[]>([])
const activeView = ref('logs')
const logSource = ref('server')
const selectedNodeId = ref('')
const selectedInstanceId = ref('')
const maxBytes = ref(65536)
const logContent = ref('')
const auditEvents = ref<AuditEvent[]>([])
const auditFilters = ref({ operation_type: '', target_type: '', result: '' })
const serverLogPolicy = ref<LogPolicy>({
  log_dir: 'logs',
  log_level: 'info',
  log_max_file_bytes: 10485760,
  log_retention_files: 5,
  log_retention_days: 7
})
const loading = ref(false)
const loadingLog = ref(false)
const loadingAudit = ref(false)
const savingServerPolicy = ref(false)
const error = ref('')

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const [nextNodes, nextInstances, nextServerLogPolicy] = await Promise.all([
      fetchNodes(),
      fetchModelInstances(),
      fetchServerLogPolicy()
    ])
    nodes.value = nextNodes
    instances.value = nextInstances
    serverLogPolicy.value = nextServerLogPolicy
    if (!selectedNodeId.value) selectedNodeId.value = nextNodes[0]?.id ?? ''
    if (!selectedInstanceId.value) selectedInstanceId.value = nextInstances[0]?.id ?? ''
    await Promise.all([loadLogs(), loadAudit()])
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载日志审计失败'
  } finally {
    loading.value = false
  }
}

async function saveServerLogPolicy() {
  savingServerPolicy.value = true
  try {
    serverLogPolicy.value = await updateServerLogPolicy(serverLogPolicy.value)
  } catch (err) {
    error.value = err instanceof Error ? err.message : '保存 Server 日志策略失败'
  } finally {
    savingServerPolicy.value = false
  }
}

async function loadLogs() {
  loadingLog.value = true
  try {
    const payload = await fetchLogs({
      source_type: logSource.value,
      node_id: logSource.value === 'agent' ? selectedNodeId.value : null,
      instance_id: logSource.value === 'instance' ? selectedInstanceId.value : null,
      max_bytes: maxBytes.value
    })
    logContent.value = payload.content
  } catch (err) {
    logContent.value = err instanceof Error ? err.message : '日志读取失败'
  } finally {
    loadingLog.value = false
  }
}

async function loadAudit() {
  loadingAudit.value = true
  try {
    auditEvents.value = await fetchAuditEvents(auditFilters.value)
  } catch (err) {
    error.value = err instanceof Error ? err.message : '审计日志读取失败'
  } finally {
    loadingAudit.value = false
  }
}

function formatTime(value?: number | null) {
  if (!value) return '-'
  return new Date(value * 1000).toLocaleString()
}

onMounted(refresh)
defineExpose({ refresh })
</script>
