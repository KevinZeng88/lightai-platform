<template>
  <el-config-provider>
    <main class="page-shell">
      <section class="overview">
        <div>
          <p class="eyebrow">LightAI Platform</p>
          <h1>节点与 GPU 运行状态</h1>
          <p class="summary">
            展示节点注册、心跳状态、基础资源指标和最近时间窗口趋势。
          </p>
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
      </section>

      <section class="toolbar">
        <el-segmented v-model="selectedRange" :options="rangeOptions" @change="onRangeChange" />
        <el-date-picker
          v-if="selectedRange === 'custom'"
          v-model="customRange"
          type="datetimerange"
          range-separator="至"
          start-placeholder="开始时间"
          end-placeholder="结束时间"
          value-format="x"
          @change="refreshMetrics"
        />
      </section>

      <el-alert v-if="error" :title="error" type="error" show-icon class="alert" />

      <el-table :data="nodes" row-key="id" border>
        <el-table-column type="expand">
          <template #default="{ row }">
            <div class="node-detail">
              <section class="trend-block">
                <div class="trend-meta">
                  <span>选择范围：{{ selectedRangeLabel }}</span>
                  <span>请求范围：{{ formatRange(nodeMetricPayload(row.id)) }}</span>
                  <span>实际数据：{{ formatActualRange(nodeMetricPayload(row.id)) }}</span>
                  <span>采样点：{{ nodeMetricPayload(row.id)?.sample_count ?? 0 }}</span>
                </div>
                <el-alert
                  v-if="historyNotice(nodeMetricPayload(row.id))"
                  :title="historyNotice(nodeMetricPayload(row.id))"
                  :type="nodeMetricPayload(row.id)?.sample_count ? 'warning' : 'info'"
                  show-icon
                  class="history-alert"
                />
                <TrendChart title="CPU / 内存 / 磁盘趋势" :series="nodeSeries(row.id)" />
              </section>
              <el-table :data="row.gpus" size="small" border>
                <el-table-column prop="gpu_index" label="#" width="70" />
                <el-table-column prop="vendor" label="厂商" width="110" />
                <el-table-column prop="name" label="GPU" min-width="180" />
                <el-table-column label="使用率" width="110">
                  <template #default="{ row: gpu }">
                    {{ formatPercent(gpu.utilization_percent) }}
                  </template>
                </el-table-column>
                <el-table-column label="显存" width="150">
                  <template #default="{ row: gpu }">
                    {{ formatBytes(gpu.memory_used_bytes) }} /
                    {{ formatBytes(gpu.memory_total_bytes) }}
                  </template>
                </el-table-column>
              </el-table>
              <div class="gpu-charts">
                <section v-for="gpu in row.gpus" :key="gpu.gpu_key" class="trend-block">
                  <div class="trend-meta">
                    <span>选择范围：{{ selectedRangeLabel }}</span>
                    <span>请求范围：{{ formatRange(gpuMetricPayload(row.id, gpu.gpu_key)) }}</span>
                    <span>实际数据：{{ formatActualRange(gpuMetricPayload(row.id, gpu.gpu_key)) }}</span>
                    <span>采样点：{{ gpuMetricPayload(row.id, gpu.gpu_key)?.sample_count ?? 0 }}</span>
                  </div>
                  <el-alert
                    v-if="historyNotice(gpuMetricPayload(row.id, gpu.gpu_key))"
                    :title="historyNotice(gpuMetricPayload(row.id, gpu.gpu_key))"
                    :type="gpuMetricPayload(row.id, gpu.gpu_key)?.sample_count ? 'warning' : 'info'"
                    show-icon
                    class="history-alert"
                  />
                  <TrendChart :title="`${gpu.name} 趋势`" :series="gpuSeries(row.id, gpu.gpu_key)" />
                </section>
              </div>
            </div>
          </template>
        </el-table-column>
        <el-table-column prop="name" label="节点" min-width="160" />
        <el-table-column prop="hostname" label="Hostname" min-width="160" />
        <el-table-column label="状态" width="100">
          <template #default="{ row }">
            <el-tag :type="row.status === 'online' ? 'success' : 'info'">
              {{ row.status }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="CPU" width="100">
          <template #default="{ row }">
            {{ formatPercent(row.metrics?.cpu_usage_percent) }}
          </template>
        </el-table-column>
        <el-table-column label="内存" width="170">
          <template #default="{ row }">
            {{ formatBytes(row.metrics?.memory_used_bytes) }} /
            {{ formatBytes(row.metrics?.memory_total_bytes) }}
          </template>
        </el-table-column>
        <el-table-column label="磁盘" width="170">
          <template #default="{ row }">
            {{ formatBytes(row.metrics?.disk_used_bytes) }} /
            {{ formatBytes(row.metrics?.disk_total_bytes) }}
          </template>
        </el-table-column>
        <el-table-column label="最后心跳" width="190">
          <template #default="{ row }">
            {{ formatTime(row.last_heartbeat_at) }}
          </template>
        </el-table-column>
      </el-table>
    </main>
  </el-config-provider>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import TrendChart from './components/TrendChart.vue'
import { fetchGpuMetrics, fetchNodeMetrics, fetchNodes } from './api'
import type {
  GpuMetricSample,
  MetricSampleResponse,
  NodeMetricSample,
  NodeStatus
} from './types'

const nodes = ref<NodeStatus[]>([])
const nodeMetrics = ref<Record<string, MetricSampleResponse<NodeMetricSample>>>({})
const gpuMetrics = ref<Record<string, MetricSampleResponse<GpuMetricSample>>>({})
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

const onlineCount = computed(() => nodes.value.filter((node) => node.status === 'online').length)
const gpuCount = computed(() => nodes.value.reduce((count, node) => count + node.gpus.length, 0))
const selectedRangeLabel = computed(
  () => rangeOptions.find((option) => option.value === selectedRange.value)?.label ?? selectedRange.value
)

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

async function refreshAll() {
  loading.value = true
  error.value = ''
  try {
    nodes.value = await fetchNodes()
    await refreshMetrics()
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载失败'
  } finally {
    loading.value = false
  }
}

async function refreshMetrics() {
  const { from, to } = timeWindow()
  const nextNodeMetrics: Record<string, MetricSampleResponse<NodeMetricSample>> = {}
  const nextGpuMetrics: Record<string, MetricSampleResponse<GpuMetricSample>> = {}

  await Promise.all(
    nodes.value.map(async (node) => {
      nextNodeMetrics[node.id] = await fetchNodeMetrics(node.id, from, to)
      await Promise.all(
        node.gpus.map(async (gpu) => {
          nextGpuMetrics[gpuMetricKey(node.id, gpu.gpu_key)] = await fetchGpuMetrics(
            node.id,
            gpu.gpu_key,
            from,
            to
          )
        })
      )
    })
  )

  nodeMetrics.value = nextNodeMetrics
  gpuMetrics.value = nextGpuMetrics
}

async function onRangeChange() {
  if (selectedRange.value !== 'custom') {
    await refreshMetrics()
  }
}

type ChartSeries = Array<{
  name: string
  data: Array<[number, number | null]>
}>

function point(time: number, value: number | null): [number, number | null] {
  return [time, value]
}

function nodeSeries(nodeId: string): ChartSeries {
  const samples = nodeMetrics.value[nodeId]?.samples ?? []
  return [
    {
      name: 'CPU %',
      data: samples.map((sample) => point(sample.sampled_at * 1000, sample.cpu_usage_percent ?? null))
    },
    {
      name: '内存 %',
      data: samples.map((sample) => point(
        sample.sampled_at * 1000,
        percent(sample.memory_used_bytes, sample.memory_total_bytes)
      ))
    },
    {
      name: '磁盘 %',
      data: samples.map((sample) => point(
        sample.sampled_at * 1000,
        percent(sample.disk_used_bytes, sample.disk_total_bytes)
      ))
    }
  ]
}

function gpuSeries(nodeId: string, gpuKey: string): ChartSeries {
  const samples = gpuMetrics.value[gpuMetricKey(nodeId, gpuKey)]?.samples ?? []
  return [
    {
      name: 'GPU %',
      data: samples.map((sample) => point(
        sample.sampled_at * 1000,
        sample.utilization_percent ?? null
      ))
    },
    {
      name: '显存 %',
      data: samples.map((sample) => point(
        sample.sampled_at * 1000,
        percent(sample.memory_used_bytes, sample.memory_total_bytes)
      ))
    }
  ]
}

function gpuMetricKey(nodeId: string, gpuKey: string) {
  return `${nodeId}:${gpuKey}`
}

function nodeMetricPayload(nodeId: string) {
  return nodeMetrics.value[nodeId]
}

function gpuMetricPayload(nodeId: string, gpuKey: string) {
  return gpuMetrics.value[gpuMetricKey(nodeId, gpuKey)]
}

function historyNotice(payload?: MetricSampleResponse<unknown>) {
  if (!payload) return ''
  if (payload.sample_count === 0) {
    return '暂无历史采样数据，请确认 Agent 正在运行并已完成 heartbeat 上报'
  }
  if (payload.actual_from != null && payload.actual_from > payload.requested_from) {
    return '系统运行时间较短，当前仅展示已采集数据'
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

function percent(used?: number | null, total?: number | null) {
  if (!used || !total) return null
  return Number(((used / total) * 100).toFixed(2))
}

function formatPercent(value?: number | null) {
  return value == null ? '-' : `${value.toFixed(1)}%`
}

function formatBytes(value?: number | null) {
  if (value == null) return '-'
  if (value >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)} GB`
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)} MB`
  return `${value} B`
}

function formatTime(value?: number | null) {
  if (!value) return '-'
  return new Date(value * 1000).toLocaleString()
}

onMounted(refreshAll)
</script>
