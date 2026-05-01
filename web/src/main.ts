import { createApp } from 'vue'
import './styles.css'
import App from './App.vue'
import { registerElementPlus } from './element-plus'

const app = createApp(App)
registerElementPlus(app)
app.mount('#app')
