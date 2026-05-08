<template>
  <section class="panel-header">
    <div>
      <h2>Agent 配置</h2>
      <p>全局默认填写具体默认值；节点级覆盖按节点配置，空值继承全局，有值覆盖全局。</p>
    </div>
    <el-button :loading="loading" type="primary" @click="loadData">刷新</el-button>
  </section>

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <el-alert
    title="Server 地址、节点标识、state 文件和本地 health 监听属于 Agent bootstrap，修改后需要重启 Agent；本页策略支持在线重新下发。"
    type="info"
    show-icon
    class="alert"
  />

  <el-alert
    title="GPU 采集只通过 Agent 本地 [gpu_collectors] 脚本目录和 Server registry/hash 校验启用；本页只配置在线策略，不下发任意采集脚本路径。"
    type="info"
    show-icon
    class="alert"
  />

  <el-card shadow="never" class="section-card">
    <template #header>全局默认策略</template>
    <el-form label-width="170px" class="config-form">
      <PolicyFields v-model="globalForm" :allow-inherit="false" />
      <el-form-item>
        <el-button type="primary" :loading="savingGlobal" @click="saveGlobal">保存全局策略</el-button>
        <span class="muted">版本：{{ policies?.global.version ?? '-' }}</span>
      </el-form-item>
    </el-form>
  </el-card>

  <el-card shadow="never" class="section-card">
    <template #header>选择节点</template>
    <el-select v-model="selectedNodeId" filterable placeholder="选择节点" class="node-config-select">
      <el-option v-for="node in nodes" :key="node.id" :label="node.name" :value="node.id" />
    </el-select>
  </el-card>

  <el-card v-if="selectedNode" shadow="never" class="section-card">
    <template #header>节点覆盖策略 · {{ selectedNode.name }}</template>
    <el-form label-width="170px" class="config-form">
      <PolicyFields v-model="nodeForm" :allow-inherit="true" />
      <el-form-item>
        <el-button type="primary" :loading="savingNode" @click="saveNode">保存节点覆盖</el-button>
        <span class="muted">留空表示继承全局默认；保存后 Agent 会通过主动控制通道获取最新有效配置。</span>
      </el-form-item>
    </el-form>
  </el-card>

  <el-card v-if="selectedNode" shadow="never" class="section-card">
    <template #header>最终生效配置与同步状态 · {{ selectedNode.name }}</template>
    <div class="detail-grid">
      <div><span class="muted">同步状态</span><p>{{ syncLabel(selectedNode.config_sync_status) }}</p></div>
      <div><span class="muted">生效版本</span><p>{{ selectedNode.effective_agent_config.config_version }}</p></div>
      <div><span class="muted">Agent 上报版本</span><p>{{ selectedNode.agent_config?.config_version ?? '-' }}</p></div>
      <div><span class="muted">心跳 / 采样</span><p>{{ selectedNode.effective_agent_config.heartbeat_interval_secs }}s / {{ selectedNode.effective_agent_config.metrics_sample_interval_secs }}s</p></div>
      <div><span class="muted">命令 / 环境检查</span><p>{{ selectedNode.effective_agent_config.command_timeout_secs }}s / {{ selectedNode.effective_agent_config.environment_check_timeout_secs }}s</p></div>
      <div class="wide-detail"><span class="muted">Allowed dirs</span><p>{{ selectedNode.effective_agent_config.allowed_model_dirs.join(', ') || '未配置' }}</p></div>
      <div><span class="muted">日志级别</span><p>{{ selectedNode.effective_agent_config.log_level }}</p></div>
      <div class="wide-detail"><span class="muted">日志目录</span><p>{{ selectedNode.effective_agent_config.log_dir }}</p></div>
      <div><span class="muted">日志轮转</span><p>{{ selectedNode.effective_agent_config.log_max_file_bytes }} bytes / {{ selectedNode.effective_agent_config.log_retention_files }} 个 / {{ selectedNode.effective_agent_config.log_retention_days }} 天</p></div>
    </div>
  </el-card>
</template>

<script setup lang="ts">
import { computed, defineComponent, h, onMounted, ref, watch } from 'vue'
import { ElFormItem } from 'element-plus/es/components/form/index'
import { ElInput } from 'element-plus/es/components/input/index'
import { ElInputNumber } from 'element-plus/es/components/input-number/index'
import { ElMessage } from 'element-plus/es/components/message/index'
import {
  fetchAgentConfigPolicies,
  fetchNodes,
  updateGlobalAgentConfigPolicy,
  updateNodeAgentConfigPolicy
} from '../api'
import type { AgentConfigPoliciesResponse, AgentConfigPolicy, NodeStatus } from '../types'

const emptyPolicy = (): AgentConfigPolicy => ({
  heartbeat_interval_secs: null,
  metrics_sample_interval_secs: null,
  command_timeout_secs: null,
  environment_check_timeout_secs: null,
  allowed_model_dirs: null,
  collector_timeout_secs: null,
  collector_max_output_bytes: null,
  log_dir: null,
  log_level: null,
  log_max_file_bytes: null,
  log_retention_files: null,
  log_retention_days: null
})

const PolicyFields = defineComponent({
  props: {
    modelValue: { type: Object, required: true },
    allowInherit: { type: Boolean, default: true }
  },
  emits: ['update:modelValue'],
  setup(props, { emit }) {
    function update(key: keyof AgentConfigPolicy, value: unknown) {
      emit('update:modelValue', { ...(props.modelValue as AgentConfigPolicy), [key]: value })
    }
    function numberField(label: string, key: keyof AgentConfigPolicy) {
      return h(ElFormItem, { label }, () =>
        h(ElInputNumber, {
          modelValue: (props.modelValue as AgentConfigPolicy)[key] as number | null,
          min: 1,
          controlsPosition: 'right',
          placeholder: props.allowInherit ? '继承' : '填写默认值',
          onChange: (value: number | undefined) => update(key, value ?? null)
        })
      )
    }
    return () => [
      numberField('心跳间隔（秒）', 'heartbeat_interval_secs'),
      numberField('指标采样间隔（秒）', 'metrics_sample_interval_secs'),
      numberField('命令超时（秒）', 'command_timeout_secs'),
      numberField('环境检查超时（秒）', 'environment_check_timeout_secs'),
      h(ElFormItem, { label: 'Allowed dirs' }, () =>
        h(ElInput, {
          modelValue: ((props.modelValue as AgentConfigPolicy).allowed_model_dirs ?? []).join('\n'),
          type: 'textarea',
          rows: 3,
          placeholder: props.allowInherit ? '每行一个绝对路径；留空表示继承' : '每行一个绝对路径',
          onInput: (value: string) =>
            update(
              'allowed_model_dirs',
              value
                .split('\n')
                .map((item) => item.trim())
                .filter(Boolean)
            )
        })
      ),
      numberField('采集器超时（秒）', 'collector_timeout_secs'),
      numberField('采集器输出上限', 'collector_max_output_bytes'),
      h(ElFormItem, { label: '日志目录' }, () =>
        h(ElInput, {
          modelValue: (props.modelValue as AgentConfigPolicy).log_dir ?? '',
          placeholder: props.allowInherit ? '默认 logs；留空继承' : '默认 logs',
          onInput: (value: string) => update('log_dir', value.trim() || null)
        })
      ),
      h(ElFormItem, { label: '日志级别' }, () =>
        h(ElInput, {
          modelValue: (props.modelValue as AgentConfigPolicy).log_level ?? '',
          placeholder: 'error / warn / info / debug / trace',
          onInput: (value: string) => update('log_level', value.trim() || null)
        })
      ),
      numberField('日志文件上限（字节）', 'log_max_file_bytes'),
      numberField('日志保留文件数', 'log_retention_files'),
      numberField('日志保留天数', 'log_retention_days')
    ]
  }
})

const nodes = ref<NodeStatus[]>([])
const policies = ref<AgentConfigPoliciesResponse>()
const selectedNodeId = ref('')
const globalForm = ref<AgentConfigPolicy>(emptyPolicy())
const nodeForm = ref<AgentConfigPolicy>(emptyPolicy())
const loading = ref(false)
const savingGlobal = ref(false)
const savingNode = ref(false)
const error = ref('')

const selectedNode = computed(() => nodes.value.find((node) => node.id === selectedNodeId.value))

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [nextNodes, nextPolicies] = await Promise.all([fetchNodes(), fetchAgentConfigPolicies()])
    nodes.value = nextNodes
    policies.value = nextPolicies
    globalForm.value = policyFromConfig(nextPolicies.global.effective_config)
    if (!selectedNodeId.value) selectedNodeId.value = nextNodes[0]?.id ?? ''
    syncNodeForm()
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载配置失败'
  } finally {
    loading.value = false
  }
}

function syncNodeForm() {
  const policy = policies.value?.nodes.find((item) => item.node_id === selectedNodeId.value)?.policy
  nodeForm.value = { ...emptyPolicy(), ...(policy ?? {}) }
}

async function saveGlobal() {
  savingGlobal.value = true
  try {
    await updateGlobalAgentConfigPolicy(normalizePolicy(globalForm.value))
    ElMessage.success('全局策略已保存')
    await loadData()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '保存失败')
  } finally {
    savingGlobal.value = false
  }
}

function policyFromConfig(config: NodeStatus['effective_agent_config']): AgentConfigPolicy {
  return {
    heartbeat_interval_secs: config.heartbeat_interval_secs,
    metrics_sample_interval_secs: config.metrics_sample_interval_secs,
    command_timeout_secs: config.command_timeout_secs,
    environment_check_timeout_secs: config.environment_check_timeout_secs,
    allowed_model_dirs: config.allowed_model_dirs,
    collector_timeout_secs: config.collector_timeout_secs,
    collector_max_output_bytes: config.collector_max_output_bytes,
    log_dir: config.log_dir,
    log_level: config.log_level,
    log_max_file_bytes: config.log_max_file_bytes,
    log_retention_files: config.log_retention_files,
    log_retention_days: config.log_retention_days
  }
}

async function saveNode() {
  if (!selectedNodeId.value) return
  savingNode.value = true
  try {
    await updateNodeAgentConfigPolicy(selectedNodeId.value, normalizePolicy(nodeForm.value))
    ElMessage.success('节点覆盖策略已保存')
    await loadData()
  } catch (err) {
    ElMessage.error(err instanceof Error ? err.message : '保存失败')
  } finally {
    savingNode.value = false
  }
}

function normalizePolicy(policy: AgentConfigPolicy) {
  return Object.fromEntries(
    Object.entries(policy).filter(([, value]) => {
      if (Array.isArray(value)) return value.length > 0
      return value !== null && value !== ''
    })
  )
}

function syncLabel(status: string) {
  if (status === 'synced') return '已同步'
  if (status === 'out_of_sync') return '待同步'
  return '待上报'
}

function syncType(status: string) {
  if (status === 'synced') return 'success'
  if (status === 'out_of_sync') return 'warning'
  return 'info'
}

watch(selectedNodeId, syncNodeForm)
onMounted(loadData)
defineExpose({ refresh: loadData })
</script>
