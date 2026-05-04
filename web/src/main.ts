import { createApp } from 'vue'
import './styles.css'
import App from './App.vue'
import { registerElementPlus } from './element-plus'
import { reportFrontendError } from './api'

const app = createApp(App)
registerElementPlus(app)

app.config.errorHandler = (err, _instance, info) => {
  const message = err instanceof Error ? err.message : String(err)
  const stack = err instanceof Error ? err.stack : undefined
  reportFrontendError({
    message: `[${info}] ${message}`,
    stack: stack?.slice(0, 2048),
    url: window.location.href
  })
  console.error('Unhandled Vue error:', err)
}

window.addEventListener('error', (event) => {
  if (event.error) {
    reportFrontendError({
      message: event.error.message || String(event.error),
      stack: event.error.stack?.slice(0, 2048),
      url: window.location.href
    })
  }
})

window.addEventListener('unhandledrejection', (event) => {
  const message = event.reason instanceof Error ? event.reason.message : String(event.reason)
  const stack = event.reason instanceof Error ? event.reason.stack : undefined
  reportFrontendError({
    message: `[unhandledrejection] ${message}`,
    stack: stack?.slice(0, 2048),
    url: window.location.href
  })
})

app.mount('#app')
