<template>
  <div ref="chartEl" class="trend-chart" />
</template>

<script setup lang="ts">
import { LineChart } from 'echarts/charts'
import {
  GridComponent,
  LegendComponent,
  TitleComponent,
  TooltipComponent
} from 'echarts/components'
import * as echarts from 'echarts/core'
import type { EChartsType } from 'echarts/core'
import { CanvasRenderer } from 'echarts/renderers'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'

echarts.use([
  LineChart,
  GridComponent,
  LegendComponent,
  TitleComponent,
  TooltipComponent,
  CanvasRenderer
])

const props = defineProps<{
  title: string
  series: Array<{
    name: string
    data: Array<[number, number | null]>
  }>
}>()

const chartEl = ref<HTMLElement>()
let chart: EChartsType | undefined

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
      top: 36,
      left: 0
    },
    grid: {
      top: 86,
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
