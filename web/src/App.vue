<template>
  <el-config-provider :locale="zhCn">
    <main class="page-shell">
      <el-tabs v-model="activeTab" class="main-tabs">
        <el-tab-pane label="节点监控" name="nodes">
          <NodesPanel />
        </el-tab-pane>
        <el-tab-pane label="运行环境" name="runtime">
          <RuntimeEnvironmentsPanel />
        </el-tab-pane>
        <el-tab-pane label="模型" name="models">
          <ModelsPanel />
        </el-tab-pane>
        <el-tab-pane label="实例" name="instances">
          <InstancesPanel />
        </el-tab-pane>
        <el-tab-pane label="文件垃圾箱" name="trash">
          <TrashPanel />
        </el-tab-pane>
      </el-tabs>
    </main>
  </el-config-provider>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import zhCn from 'element-plus/es/locale/lang/zh-cn'
import InstancesPanel from './components/InstancesPanel.vue'
import ModelsPanel from './components/ModelsPanel.vue'
import NodesPanel from './components/NodesPanel.vue'
import RuntimeEnvironmentsPanel from './components/RuntimeEnvironmentsPanel.vue'
import TrashPanel from './components/TrashPanel.vue'

const activeTab = ref('nodes')
</script>
