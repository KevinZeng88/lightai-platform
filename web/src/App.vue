<template>
  <el-config-provider>
    <main class="page-shell">
      <section class="overview">
        <div>
          <p class="eyebrow">LightAI Platform</p>
          <h1>轻量 GPU 模型服务管理平台</h1>
          <p class="summary">
            Stage 1 提供 Server、Agent、Web 控制台占位和基础工程结构。
          </p>
        </div>
        <el-tag type="success" effect="plain" size="large">Stage 1</el-tag>
      </section>

      <section class="status-grid" aria-label="Service placeholders">
        <el-card v-for="item in services" :key="item.name" shadow="never">
          <div class="service-card">
            <div>
              <p class="service-name">{{ item.name }}</p>
              <p class="service-desc">{{ item.description }}</p>
            </div>
            <el-tag :type="item.type" effect="light">{{ item.status }}</el-tag>
          </div>
        </el-card>
      </section>
    </main>
  </el-config-provider>
</template>

<script setup lang="ts">
const services = [
  {
    name: 'Server',
    description: '管理 API、网关和数据层的预留入口。',
    status: '/health',
    type: 'success' as const
  },
  {
    name: 'Agent',
    description: 'GPU 节点侧进程的预留入口。',
    status: '/health',
    type: 'success' as const
  },
  {
    name: 'Web',
    description: '控制台首页占位，不包含业务交互。',
    status: 'placeholder',
    type: 'info' as const
  }
]
</script>
