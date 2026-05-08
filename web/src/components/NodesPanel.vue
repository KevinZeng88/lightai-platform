<template>
  <section class="overview">
    <div>
      <p class="eyebrow">LightAI Platform</p>
      <h1>节点与 GPU 运行状态</h1>
      <p class="summary">选择一个节点后查看其 Agent 配置、趋势和 GPU 详情。</p>
    </div>
    <el-button :loading="loading" type="primary" @click="refreshAll">刷新</el-button>
  </section>

  <section class="summary-grid">
    <el-card shadow="never">
      <p class="metric-label">节点</p>
      <p class="metric-value">{{ nodes.length }}</p>
    </el-card>
    <el-card shadow="never">
      <p class="metric-label">在线</p>
      <p class="metric-value">{{ onlineCount }}</p>
    </el-card>
    <el-card shadow="never">
      <p class="metric-label">GPU</p>
      <p class="metric-value">{{ gpuCount }}</p>
    </el-card>
    <el-card shadow="never">
      <p class="metric-label">平均 GPU</p>
      <p class="metric-value">{{ formatPercent(avgGpuUtilization) }}</p>
    </el-card>
  </section>

  <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

  <!-- 节点列表 -->
  <el-card shadow="never" class="section-card">
    <template #header>
      <div class="card-header-row">
        <span>节点列表</span>
        <span v-if="!selectedNode" class="muted">点击"查看"选择一个节点以查看监控详情</span>
        <span v-else class="muted">当前选中：{{ selectedNode.name }}</span>
      </div>
    </template>
    <el-table
      :data="nodes"
      row-key="id"
      border
      highlight-current-row
      :current-row-key="selectedNodeId || undefined"
      @current-change="selectNode"
    >
      <el-table-column prop="name" label="节点" min-width="150" fixed="left" />
      <el-table-column label="状态" width="100">
        <template #default="{ row }">
          <el-tag :type="row.status === 'online' ? 'success' : 'info'">{{ row.status }}</el-tag>
        </template>
      </el-table-column>
      <el-table-column label="CPU" width="100">
        <template #default="{ row }">{{ formatPercent(row.metrics?.cpu_usage_percent) }}</template>
      </el-table-column>
      <el-table-column label="内存" width="110">
        <template #default="{ row }">
          {{ formatPercent(percent(row.metrics?.memory_used_bytes, row.metrics?.memory_total_bytes)) }}
        </template>
      </el-table-column>
      <el-table-column label="磁盘" width="110">
        <template #default="{ row }">
          {{ formatPercent(percent(row.metrics?.disk_used_bytes, row.metrics?.disk_total_bytes)) }}
        </template>
      </el-table-column>
      <el-table-column label="最后心跳" width="190">
        <template #default="{ row }">{{ formatTime(row.last_heartbeat_at) }}</template>
      </el-table-column>
      <el-table-column label="心跳间隔" width="110">
        <template #default="{ row }">{{ row.agent_config?.heartbeat_interval_secs ?? '-' }}s</template>
      </el-table-column>
      <el-table-column label="采样间隔" width="110">
        <template #default="{ row }">{{ row.agent_config?.metrics_sample_interval_secs ?? '-' }}s</template>
      </el-table-column>
      <el-table-column label="配置版本" width="110">
        <template #default="{ row }">{{ row.effective_agent_config.config_version }}</template>
      </el-table-column>
      <el-table-column label="同步状态" width="110">
        <template #default="{ row }">
          <el-tag :type="syncType(row.config_sync_status)">{{ syncLabel(row.config_sync_status) }}</el-tag>
        </template>
      </el-table-column>
      <el-table-column label="操作" width="90" fixed="right">
        <template #default="{ row }">
          <el-button size="small" type="primary" @click.stop="selectNode(row)">查看</el-button>
        </template>
      </el-table-column>
    </el-table>
  </el-card>

  <!-- Agent 生效配置 -->
  <el-card v-if="selectedNode" shadow="never" class="section-card">
    <template #header>Agent 生效配置 · {{ selectedNode.name }}</template>
    <div class="detail-grid">
      <div>
        <span class="muted">Server 生效版本</span>
        <p>{{ selectedNode.effective_agent_config.config_version }}</p>
      </div>
      <div>
        <span class="muted">Agent 上报版本</span>
        <p>{{ selectedNode.agent_config?.config_version ?? '-' }}</p>
      </div>
      <div>
        <span class="muted">同步状态</span>
        <p>{{ syncLabel(selectedNode.config_sync_status) }}</p>
      </div>
      <div>
        <span class="muted">心跳 / 采样</span>
        <p>{{ selectedNode.effective_agent_config.heartbeat_interval_secs }}s / {{ selectedNode.effective_agent_config.metrics_sample_interval_secs }}s</p>
      </div>
      <div>
        <span class="muted">命令 / 环境检查超时</span>
        <p>{{ selectedNode.effective_agent_config.command_timeout_secs }}s / {{ selectedNode.effective_agent_config.environment_check_timeout_secs }}s</p>
      </div>
      <div>
        <span class="muted">采集超时 / 输出上限</span>
        <p>{{ selectedNode.effective_agent_config.collector_timeout_secs }}s / {{ formatBytes(selectedNode.effective_agent_config.collector_max_output_bytes) }}</p>
      </div>
      <div class="wide-detail">
        <span class="muted">Allowed dirs</span>
        <p>{{ selectedNode.effective_agent_config.allowed_model_dirs.join(', ') || '未配置，物理删除会被拒绝' }}</p>
      </div>
    </div>
    <el-alert
      title="GPU 由 Agent 脚本化 collector 自动发现并随心跳上报，Web 不手工添加 GPU。collector 必须先登记 registry 并通过 hash 校验。"
      type="info"
      show-icon
      class="history-alert"
    />
    <el-alert
      title="本页策略字段可在线生效；Agent bootstrap 字段（Server 地址、节点标识、state、health 监听）需要重启 Agent。"
      type="info"
      show-icon
      class="history-alert"
    />
  </el-card>
  <el-card v-else shadow="never" class="section-card">
    <template #header>Agent 生效配置</template>
    <el-empty description="请选择节点查看 Agent 生效配置" :image-size="80" />
  </el-card>

  <!-- 节点趋势 -->
  <el-card v-if="selectedNode" shadow="never" class="section-card">
    <template #header>
      <div class="card-header-row">
        <span>节点趋势 · {{ selectedNode.name }}</span>
        <div class="trend-toolbar">
          <el-segmented v-model="selectedRange" :options="rangeOptions" @change="refreshMetrics" size="small" />
          <el-date-picker
            v-if="selectedRange === 'custom'"
            v-model="customRange"
            type="datetimerange"
            range-separator="至"
            start-placeholder="开始时间"
            end-placeholder="结束时间"
            value-format="x"
            size="small"
            @change="refreshMetrics"
          />
        </div>
      </div>
    </template>
    <div class="trend-meta">
      <span>范围：{{ selectedRangeLabel }}</span>
      <span>请求：{{ formatRange(nodeMetrics) }}</span>
      <span>实际：{{ formatActualRange(nodeMetrics) }}</span>
      <span>采样点：{{ nodeMetrics?.sample_count ?? 0 }}</span>
    </div>
    <el-alert
      v-if="historyNotice(nodeMetrics, '暂无节点历史数据，请确认 Agent 正在运行并已完成 heartbeat 上报')"
      :title="historyNotice(nodeMetrics, '暂无节点历史数据，请确认 Agent 正在运行并已完成 heartbeat 上报')"
      :type="nodeMetrics?.sample_count ? 'warning' : 'info'"
      show-icon
      class="history-alert"
    />
    <TrendChart title="CPU / 内存 / 磁盘趋势" :series="nodeSeries" />
  </el-card>
  <el-card v-else shadow="never" class="section-card">
    <template #header>节点趋势</template>
    <el-empty description="请选择节点查看节点趋势" :image-size="80" />
  </el-card>

  <!-- GPU 列表 -->
  <el-card v-if="selectedNode" shadow="never" class="section-card">
    <template #header>
      <div class="card-header-row">
        <span>GPU 列表 · {{ selectedNode.name }}</span>
        <span v-if="selectedGpu" class="muted">当前趋势：{{ selectedGpu.name }}</span>
      </div>
    </template>
    <el-table
      v-if="selectedNode.gpus.length > 0"
      :data="selectedNode.gpus"
      row-key="gpu_key"
      size="small"
      border
      highlight-current-row
      :current-row-key="selectedGpuKey || undefined"
      @current-change="selectGpu"
    >
      <el-table-column prop="name" label="GPU" min-width="180" fixed="left" />
      <el-table-column prop="vendor" label="厂商" width="110" />
      <el-table-column label="利用率" width="110">
        <template #default="{ row }">{{ formatPercent(row.utilization_percent) }}</template>
      </el-table-column>
      <el-table-column label="显存" width="130">
        <template #default="{ row }">
          {{ formatPercent(percent(row.memory_used_bytes, row.memory_total_bytes)) }}
        </template>
      </el-table-column>
      <el-table-column label="温度" width="100">
        <template #default="{ row }">{{ row.temperature_celsius == null ? '-' : `${row.temperature_celsius.toFixed(0)}°C` }}</template>
      </el-table-column>
      <el-table-column label="功耗" width="100">
        <template #default="{ row }">{{ row.power_watts == null ? '-' : `${row.power_watts.toFixed(0)}W` }}</template>
      </el-table-column>
      <el-table-column prop="collector" label="采集器" width="110" />
      <el-table-column label="状态" width="140">
        <template #default="{ row }">
          <template v-if="row.status === 'missing'">
            <el-tooltip :content="row.last_error ?? '设备曾被发现，但最近一次成功采集中未出现'">
              <el-tag type="danger" size="small">设备消失</el-tag>
            </el-tooltip>
          </template>
          <template v-else-if="row.last_error">
            <el-tooltip :content="row.last_error">
              <el-tag type="warning" size="small">异常</el-tag>
            </el-tooltip>
          </template>
          <template v-else>
            <el-tag type="success" size="small">正常</el-tag>
          </template>
        </template>
      </el-table-column>
      <el-table-column label="操作" width="90" fixed="right">
        <template #default="{ row }">
          <el-button size="small" type="primary" @click.stop="selectGpu(row)">趋势</el-button>
        </template>
      </el-table-column>
    </el-table>
    <el-empty v-else description="当前节点无 GPU 数据" :image-size="80" />
  </el-card>
  <el-card v-else shadow="never" class="section-card">
    <template #header>GPU 列表</template>
    <el-empty description="请选择节点查看 GPU 列表" :image-size="80" />
  </el-card>

  <!-- GPU 趋势 -->
  <el-card v-if="selectedNode && selectedGpu" shadow="never" class="section-card">
    <template #header>GPU 趋势 · {{ selectedGpu.name }}</template>
    <el-alert
      :title="`当前展示 ${selectedGpu.name} 的 GPU 利用率、显存和温度历史趋势`"
      type="info"
      show-icon
      class="history-alert"
    />
    <div class="trend-meta">
      <span>范围：{{ selectedRangeLabel }}</span>
      <span>请求：{{ formatRange(gpuMetrics) }}</span>
      <span>实际：{{ formatActualRange(gpuMetrics) }}</span>
      <span>采样点：{{ gpuMetrics?.sample_count ?? 0 }}</span>
    </div>
    <el-alert
      v-if="historyNotice(gpuMetrics, '暂无 GPU 历史数据，请确认 Agent 正在运行并已完成 GPU 指标上报。')"
      :title="historyNotice(gpuMetrics, '暂无 GPU 历史数据，请确认 Agent 正在运行并已完成 GPU 指标上报。')"
      :type="gpuMetrics?.sample_count ? 'warning' : 'info'"
      show-icon
      class="history-alert"
    />
    <TrendChart :title="`${selectedGpu.name} 利用率 / 显存趋势`" :series="gpuSeries" />
  </el-card>
  <el-card v-else-if="selectedNode" shadow="never" class="section-card">
    <template #header>GPU 趋势</template>
    <el-empty description="请选择一个 GPU 查看趋势" :image-size="80" />
  </el-card>
  <el-card v-else shadow="never" class="section-card">
    <template #header>GPU 趋势</template>
    <el-empty description="请选择节点后选择一个 GPU 查看趋势" :image-size="80" />
  </el-card>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import TrendChart from './TrendChart.vue'
import { fetchGpuMetrics, fetchNodeMetrics, fetchNodes } from '../api'
import type {
  GpuMetricSample,
  GpuStatus,
  MetricSampleResponse,
  NodeMetricSample,
  NodeStatus
} from '../types'

const nodes = ref<NodeStatus[]>([])
const selectedNodeId = ref('')
const selectedGpuKey = ref('')
const nodeMetrics = ref<MetricSampleResponse<NodeMetricSample> | undefined>()
const gpuMetrics = ref<MetricSampleResponse<GpuMetricSample> | undefined>()
const loading = ref(false)
const error = ref('')
const selectedRange = ref('1h')
const customRange = ref<[number, number] | null>(null)

const rangeOptions = [
  { label: '最近 1 小时', value: '1h' },
  { label: '最近 6 小时', value: '6h' },
  { label: '最近 24 小时', value: '24h' },
  { label: '最近 7 天', value: '7d' },
  { label: '自定义', value: 'custom' }
]

const selectedNode = computed(() => nodes.value.find((node) => node.id === selectedNodeId.value))
const selectedGpu = computed(() =>
  selectedNode.value?.gpus.find((gpu) => gpu.gpu_key === selectedGpuKey.value)
)
const onlineCount = computed(() => nodes.value.filter((node) => node.status === 'online').length)
const gpuCount = computed(() => nodes.value.reduce((count, node) => count + node.gpus.length, 0))
const avgGpuUtilization = computed(() => {
  const values = nodes.value.flatMap((node) =>
    node.gpus
      .map((gpu) => gpu.utilization_percent)
      .filter((value): value is number => value != null)
  )
  if (!values.length) return null
  return values.reduce((sum, value) => sum + value, 0) / values.length
})
const selectedRangeLabel = computed(
  () => rangeOptions.find((option) => option.value === selectedRange.value)?.label ?? selectedRange.value
)

const nodeSeries = computed(() => {
  const samples = nodeMetrics.value?.samples ?? []
  return [
    {
      name: 'CPU %',
      data: samples.map((sample) => point(sample.sampled_at * 1000, sample.cpu_usage_percent ?? null))
    },
    {
      name: '内存 %',
      data: samples.map((sample) =>
        point(sample.sampled_at * 1000, percent(sample.memory_used_bytes, sample.memory_total_bytes))
      )
    },
    {
      name: '磁盘 %',
      data: samples.map((sample) =>
        point(sample.sampled_at * 1000, percent(sample.disk_used_bytes, sample.disk_total_bytes))
      )
    }
  ]
})

const gpuSeries = computed(() => {
  const samples = gpuMetrics.value?.samples ?? []
  return [
    {
      name: 'GPU %',
      data: samples.map((sample) => point(sample.sampled_at * 1000, sample.utilization_percent ?? null))
    },
    {
      name: '显存 %',
      data: samples.map((sample) =>
        point(sample.sampled_at * 1000, percent(sample.memory_used_bytes, sample.memory_total_bytes))
      )
    },
    {
      name: '温度',
      data: samples.map((sample) => point(sample.sampled_at * 1000, sample.temperature_celsius ?? null))
    }
  ]
})

async function refreshAll() {
  loading.value = true
  error.value = ''
  try {
    nodes.value = await fetchNodes()
    // 如果当前已选中节点，重新加载指标；否则不自动选择
    if (selectedNodeId.value && selectedNode.value) {
      await refreshMetrics()
    } else {
      // 节点列表变更后，之前选中的节点可能已不存在，清除选中
      selectedNodeId.value = ''
      selectedGpuKey.value = ''
      nodeMetrics.value = undefined
      gpuMetrics.value = undefined
    }
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

async function refreshMetrics() {
  if (!selectedNodeId.value) return
  const { from, to } = timeWindow()
  nodeMetrics.value = await fetchNodeMetrics(selectedNodeId.value, from, to)
  if (selectedGpuKey.value) {
    gpuMetrics.value = await fetchGpuMetrics(selectedNodeId.value, selectedGpuKey.value, from, to)
  } else {
    gpuMetrics.value = undefined
  }
}

async function selectNode(row?: NodeStatus) {
  if (!row) return
  selectedNodeId.value = row.id
  // 切换节点时重置 GPU 选择
  selectedGpuKey.value = ''
  gpuMetrics.value = undefined
  await refreshMetrics()
}

async function selectGpu(row?: GpuStatus) {
  if (!row) return
  selectedGpuKey.value = row.gpu_key
  await refreshMetrics()
}

function timeWindow() {
  const now = Math.floor(Date.now() / 1000)
  if (selectedRange.value === 'custom' && customRange.value) {
    return {
      from: Math.floor(customRange.value[0] / 1000),
      to: Math.floor(customRange.value[1] / 1000)
    }
  }
  const seconds =
    selectedRange.value === '6h'
      ? 6 * 3600
      : selectedRange.value === '24h'
        ? 24 * 3600
        : selectedRange.value === '7d'
          ? 7 * 24 * 3600
          : 3600
  return { from: now - seconds, to: now }
}

function historyNotice(payload: MetricSampleResponse<unknown> | undefined, emptyText: string) {
  if (!payload) return ''
  if (payload.sample_count === 0) return emptyText
  if (payload.sample_count < 3) return '当前采样点较少，趋势会随 Agent 运行时间逐步形成。'
  if (payload.actual_from != null && payload.actual_from > payload.requested_from) {
    return '系统运行时间较短，当前仅展示已采集数据。'
  }
  return ''
}

function formatRange(payload?: MetricSampleResponse<unknown>) {
  if (!payload) return '-'
  return `${formatTime(payload.requested_from)} - ${formatTime(payload.requested_to)}`
}

function formatActualRange(payload?: MetricSampleResponse<unknown>) {
  if (!payload || payload.actual_from == null || payload.actual_to == null) return '-'
  return `${formatTime(payload.actual_from)} - ${formatTime(payload.actual_to)}`
}

function point(time: number, value: number | null): [number, number | null] {
  return [time, value]
}

function percent(used?: number | null, total?: number | null) {
  if (used == null || total == null || total <= 0) return null
  return Number(((used / total) * 100).toFixed(2))
}

function formatPercent(value?: number | null) {
  return value == null ? '-' : `${value.toFixed(1)}%`
}

function formatBytes(value?: number | null) {
  if (value == null) return '-'
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MiB`
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`
  return `${value} B`
}

function formatTime(value?: number | null) {
  if (value == null) return '-'
  return new Date(value * 1000).toLocaleString()
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

onMounted(refreshAll)
defineExpose({ refresh: refreshAll })
</script>
