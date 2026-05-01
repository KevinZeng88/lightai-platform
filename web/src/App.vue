<template>
  <el-config-provider :locale="zhCn">
    <main class="page-shell">
      <el-tabs v-model="activeTab" class="main-tabs" @tab-change="refreshActiveTab">
        <el-tab-pane label="节点监控" name="nodes">
          <NodesPanel ref="nodesPanel" />
        </el-tab-pane>
        <el-tab-pane label="配置" name="config">
          <ConfigPanel ref="configPanel" />
        </el-tab-pane>
        <el-tab-pane label="运行环境" name="runtime">
          <RuntimeEnvironmentsPanel ref="runtimePanel" />
        </el-tab-pane>
        <el-tab-pane label="模型" name="models">
          <ModelsPanel ref="modelsPanel" />
        </el-tab-pane>
        <el-tab-pane label="实例" name="instances">
          <InstancesPanel ref="instancesPanel" />
        </el-tab-pane>
        <el-tab-pane label="模型垃圾箱" name="trash">
          <TrashPanel ref="trashPanel" />
        </el-tab-pane>
      </el-tabs>
    </main>
  </el-config-provider>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import zhCn from 'element-plus/es/locale/lang/zh-cn'
import ConfigPanel from './components/ConfigPanel.vue'
import InstancesPanel from './components/InstancesPanel.vue'
import ModelsPanel from './components/ModelsPanel.vue'
import NodesPanel from './components/NodesPanel.vue'
import RuntimeEnvironmentsPanel from './components/RuntimeEnvironmentsPanel.vue'
import TrashPanel from './components/TrashPanel.vue'

const activeTab = ref('nodes')
const nodesPanel = ref<InstanceType<typeof NodesPanel> | null>(null)
const configPanel = ref<InstanceType<typeof ConfigPanel> | null>(null)
const runtimePanel = ref<InstanceType<typeof RuntimeEnvironmentsPanel> | null>(null)
const modelsPanel = ref<InstanceType<typeof ModelsPanel> | null>(null)
const instancesPanel = ref<InstanceType<typeof InstancesPanel> | null>(null)
const trashPanel = ref<InstanceType<typeof TrashPanel> | null>(null)

function refreshActiveTab(name: string | number) {
  const refreshers: Record<string, (() => void | Promise<void>) | undefined> = {
    nodes: nodesPanel.value?.refresh,
    config: configPanel.value?.refresh,
    runtime: runtimePanel.value?.refresh,
    models: modelsPanel.value?.refresh,
    instances: instancesPanel.value?.refresh,
    trash: trashPanel.value?.refresh
  }
  void refreshers[String(name)]?.()
}
</script>
