<template>
  <section class="panel-header">
    <div>
      <h2>运行环境</h2>
      <p>运行环境表示某节点具备哪些本地运行能力；External 服务请在“实例”中直接接入。</p>
    </div>
    <div class="toolbar compact">
      <el-button :loading="loading" @click="loadData">刷新</el-button>
      <el-button type="primary" @click="openCreate">新增环境</el-button>
    </div>
  </section>

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-table :data="environments" row-key="id" border>
    <el-table-column prop="name" label="名称" min-width="150" fixed="left" />
    <el-table-column label="检查状态" width="130">
      <template #default="{ row }">
        <el-tag :type="checkType(row.check_status)">{{ row.check_status ?? 'unknown' }}</el-tag>
      </template>
    </el-table-column>
    <el-table-column label="检查信息" min-width="240">
      <template #default="{ row }">
        <span>{{ row.check_message ?? '-' }}</span>
      </template>
    </el-table-column>
    <el-table-column label="节点" min-width="150">
      <template #default="{ row }">{{ nodeName(row.node_id) }}</template>
    </el-table-column>
    <el-table-column prop="backend" label="后端" width="120" />
    <el-table-column prop="deploy_type" label="部署方式" width="120" />
    <el-table-column prop="version" label="版本" width="120" />
    <el-table-column label="状态" width="120">
      <template #default="{ row }">
        <el-tag :type="row.enabled ? 'success' : 'info'">
          {{ row.enabled ? 'enabled' : 'disabled' }}
        </el-tag>
      </template>
    </el-table-column>
    <el-table-column label="最近检查" width="190">
      <template #default="{ row }">
        {{ formatTime(row.last_checked_at) }}
      </template>
    </el-table-column>
    <el-table-column label="操作" width="230" fixed="right">
      <template #default="{ row }">
        <el-button size="small" @click="check(row)">检查</el-button>
        <el-button size="small" @click="openEdit(row)">编辑</el-button>
        <el-button size="small" type="danger" @click="remove(row)">删除</el-button>
      </template>
    </el-table-column>
  </el-table>

  <el-dialog v-model="dialogVisible" :title="editingId ? '编辑运行环境' : '新增运行环境'" width="640px">
    <el-form label-width="110px">
      <el-form-item label="节点">
        <el-select v-model="form.node_id" filterable placeholder="选择节点" :disabled="Boolean(editingId)">
          <el-option v-for="node in nodes" :key="node.id" :label="node.name" :value="node.id" />
        </el-select>
      </el-form-item>
      <el-form-item label="名称">
        <el-input v-model="form.name" />
      </el-form-item>
      <el-form-item label="后端">
        <el-select v-model="form.backend">
          <el-option v-for="backend in backends" :key="backend" :label="backend" :value="backend" />
        </el-select>
      </el-form-item>
      <el-form-item label="运行方式">
        <el-select v-model="form.deploy_type">
          <el-option label="Docker" value="docker" />
          <el-option label="Script" value="script" />
          <el-option label="本地程序" value="binary" />
        </el-select>
      </el-form-item>
      <el-alert
        title="保存时会由所选节点 Agent 立即检查。版本优先由 Agent 返回；无法自动获取时会说明原因，也可手工填写/覆盖版本。"
        type="info"
        show-icon
        class="alert"
      />
      <el-form-item label="版本">
        <el-input v-model="form.version" placeholder="可选；无法自动获取时可手工填写" />
      </el-form-item>
      <el-form-item v-if="form.deploy_type === 'docker'" label="Docker Image">
        <el-input v-model="form.docker_image" placeholder="vllm/vllm-openai:latest" />
      </el-form-item>
      <el-form-item v-if="form.deploy_type === 'script'" label="脚本路径">
        <el-input v-model="form.binary_path" placeholder="/opt/lightai/scripts/start-vllm" />
      </el-form-item>
      <el-form-item v-if="form.deploy_type === 'binary'" label="程序路径">
        <el-input v-model="form.binary_path" placeholder="/usr/local/bin/ollama" />
      </el-form-item>
      <el-form-item label="工作目录">
        <el-input v-model="form.working_dir" placeholder="/opt/lightai" />
      </el-form-item>
      <el-form-item label="启用">
        <el-switch v-model="form.enabled" />
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
  checkRuntimeEnvironment,
  createRuntimeEnvironment,
  deleteRuntimeEnvironment,
  fetchNodes,
  fetchRuntimeEnvironments,
  updateRuntimeEnvironment
} from '../api'
import type { NodeStatus, RuntimeEnvironment } from '../types'

const backends = ['vllm', 'ollama', 'lmdeploy', 'mindie', 'llama_cpp', 'triton', 'custom']
const nodes = ref<NodeStatus[]>([])
const environments = ref<RuntimeEnvironment[]>([])
const loading = ref(false)
const error = ref('')
const dialogVisible = ref(false)
const editingId = ref('')
const form = ref({
  node_id: '',
  name: '',
  backend: 'ollama',
  deploy_type: 'binary',
  version: '',
  binary_path: '',
  docker_image: '',
  working_dir: '',
  enabled: true
})

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [nextNodes, nextEnvironments] = await Promise.all([
      fetchNodes(),
      fetchRuntimeEnvironments()
    ])
    nodes.value = nextNodes
    environments.value = nextEnvironments
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

function openCreate() {
  editingId.value = ''
  form.value = {
    node_id: nodes.value[0]?.id ?? '',
    name: '',
    backend: 'ollama',
    deploy_type: 'binary',
    version: '',
    binary_path: '',
    docker_image: '',
    working_dir: '',
    enabled: true
  }
  dialogVisible.value = true
}

function openEdit(row: RuntimeEnvironment) {
  editingId.value = row.id
  form.value = {
    node_id: row.node_id ?? '',
    name: row.name,
    backend: row.backend,
    deploy_type: row.deploy_type,
    version: row.version ?? '',
    binary_path: row.binary_path ?? '',
    docker_image: row.docker_image ?? '',
    working_dir: row.working_dir ?? '',
    enabled: row.enabled
  }
  dialogVisible.value = true
}

async function submit() {
  if (!form.value.node_id || !form.value.name) return
  const payload = {
    name: form.value.name,
    backend: form.value.backend,
    deploy_type: form.value.deploy_type,
    version: emptyToNull(form.value.version),
    base_url: null,
    health_url: null,
    endpoint_url: null,
    binary_path: emptyToNull(form.value.binary_path),
    docker_image: emptyToNull(form.value.docker_image),
    working_dir: emptyToNull(form.value.working_dir),
    enabled: form.value.enabled
  }
  if (editingId.value) {
    await updateRuntimeEnvironment(editingId.value, payload)
  } else {
    await createRuntimeEnvironment(form.value.node_id, payload)
  }
  dialogVisible.value = false
  await loadData()
}

async function check(row: RuntimeEnvironment) {
  const checked = await checkRuntimeEnvironment(row.id)
  ElNotification({
    title: `检查状态：${checked.check_status ?? 'unknown'}`,
    message: checked.check_message ?? formatTime(checked.last_checked_at),
    type: checked.check_status === 'available' ? 'success' : checked.check_status === 'agent_offline' ? 'error' : 'warning'
  })
  await loadData()
}

async function remove(row: RuntimeEnvironment) {
  await ElMessageBox.confirm(`删除运行环境 ${row.name}？`, '确认删除', {
    type: 'warning',
    confirmButtonText: '确认',
    cancelButtonText: '取消'
  })
  try {
    await deleteRuntimeEnvironment(row.id)
    ElMessage.success('已删除')
    await loadData()
  } catch (err) {
    ElMessage.error(toBusinessMessage(err))
  }
}

function nodeName(nodeId?: string | null) {
  if (!nodeId) return '-'
  return nodes.value.find((node) => node.id === nodeId)?.name ?? nodeId
}

function emptyToNull(value: string) {
  return value.trim() ? value.trim() : null
}

function checkType(status?: string | null) {
  if (status === 'available') return 'success'
  if (status === 'unavailable' || status === 'check_timeout' || status === 'agent_offline' || status === 'not_executable' || status === 'invalid_path') return 'danger'
  if (status === 'pending' || status === 'version_unavailable') return 'warning'
  return 'info'
}

function formatTime(value?: number | null) {
  if (!value) return '-'
  return new Date(value * 1000).toLocaleString()
}

function toBusinessMessage(err: unknown) {
  const message = err instanceof Error ? err.message : '操作失败'
  if (message.includes('runtime environment is used by model instances')) {
    return '运行环境已被模型实例引用，不能删除'
  }
  return message
}

onMounted(loadData)
defineExpose({ refresh: loadData })
</script>
