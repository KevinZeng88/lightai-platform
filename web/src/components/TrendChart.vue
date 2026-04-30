<template>
  <div ref="chartEl" class="trend-chart" />
</template>

<script setup lang="ts">
import * as echarts from 'echarts'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

const props = defineProps<{
  title: string
  series: Array<{
    name: string
    data: Array<[number, number | null]>
  }>
}>()

const chartEl = ref<HTMLElement>()
let chart: echarts.ECharts | undefined

function renderChart() {
  if (!chartEl.value) return
  chart ??= echarts.init(chartEl.value)
  chart.setOption({
    title: {
      text: props.title,
      left: 0,
      textStyle: {
        fontSize: 13,
        fontWeight: 600
      }
    },
    tooltip: {
      trigger: 'axis'
    },
    legend: {
      top: 24,
      left: 0
    },
    grid: {
      top: 64,
      right: 16,
      bottom: 28,
      left: 44
    },
    xAxis: {
      type: 'time'
    },
    yAxis: {
      type: 'value'
    },
    series: props.series.map((item) => ({
      name: item.name,
      type: 'line',
      showSymbol: false,
      connectNulls: false,
      data: item.data
    }))
  })
}

function resizeChart() {
  chart?.resize()
}

onMounted(() => {
  renderChart()
  window.addEventListener('resize', resizeChart)
})

watch(() => props.series, renderChart, { deep: true })
watch(() => props.title, renderChart)

onBeforeUnmount(() => {
  window.removeEventListener('resize', resizeChart)
  chart?.dispose()
})
</script>
