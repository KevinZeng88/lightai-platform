import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  build: {
    chunkSizeWarningLimit: 520,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('/node_modules/vue/')) return 'vue'
          if (id.includes('/node_modules/echarts/') || id.includes('/node_modules/zrender/')) {
            return 'echarts'
          }
          if (id.includes('/node_modules/element-plus/')) {
            if (id.includes('/components/table/')) return 'element-plus-table'
            if (
              id.includes('/components/select/') ||
              id.includes('/components/option/') ||
              id.includes('/components/date-picker/')
            ) {
              return 'element-plus-inputs'
            }
            return 'element-plus-core'
          }
        }
      }
    }
  },
  server: {
    host: '127.0.0.1',
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:8080'
    }
  }
})
