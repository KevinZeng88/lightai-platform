<template>
  <el-config-provider :locale="zhCn">
    <main v-if="setupRequired || !currentUser" class="login-shell">
      <el-card class="login-card">
        <template #header>
          <div class="login-title">{{ setupRequired ? '初始化管理员' : 'LightAI 登录' }}</div>
        </template>
        <el-alert
          v-if="authMessage"
          :title="authMessage"
          type="warning"
          show-icon
          class="alert"
        />
        <el-form v-if="setupRequired" label-position="top" @submit.prevent>
          <el-form-item label="管理员用户名">
            <el-input v-model="setupForm.username" autocomplete="username" @keyup.enter="submitSetup" />
          </el-form-item>
          <el-form-item label="管理员密码">
            <el-input
              v-model="setupForm.password"
              type="password"
              show-password
              autocomplete="new-password"
              @keyup.enter="submitSetup"
            />
          </el-form-item>
          <el-button
            type="primary"
            :loading="authLoading"
            :disabled="!setupForm.username.trim() || !setupForm.password"
            @click="submitSetup"
          >
            创建管理员
          </el-button>
        </el-form>
        <el-form v-else label-position="top" @submit.prevent>
          <el-form-item label="用户名">
            <el-input v-model="loginForm.username" autocomplete="username" @keyup.enter="submitLogin" />
          </el-form-item>
          <el-form-item label="密码">
            <el-input
              v-model="loginForm.password"
              type="password"
              show-password
              autocomplete="current-password"
              @keyup.enter="submitLogin"
            />
          </el-form-item>
          <el-button
            type="primary"
            :loading="authLoading"
            :disabled="!loginForm.username.trim() || !loginForm.password"
            @click="submitLogin"
          >
            登录
          </el-button>
        </el-form>
      </el-card>
    </main>
    <main v-else-if="currentUser.must_change_password" class="login-shell">
      <el-card class="login-card">
        <template #header>
          <div class="login-title">修改密码</div>
        </template>
        <el-alert
          title="管理员已重置你的密码，请先修改密码后继续使用控制台。"
          type="warning"
          show-icon
          class="alert"
        />
        <el-alert
          v-if="authMessage"
          :title="authMessage"
          type="error"
          show-icon
          class="alert"
        />
        <el-form label-position="top" @submit.prevent>
          <el-form-item label="当前密码">
            <el-input
              v-model="passwordForm.currentPassword"
              type="password"
              show-password
              autocomplete="current-password"
              @keyup.enter="submitPasswordChange"
            />
          </el-form-item>
          <el-form-item label="新密码">
            <el-input
              v-model="passwordForm.newPassword"
              type="password"
              show-password
              autocomplete="new-password"
              @keyup.enter="submitPasswordChange"
            />
          </el-form-item>
          <el-form-item label="确认新密码">
            <el-input
              v-model="passwordForm.confirmPassword"
              type="password"
              show-password
              autocomplete="new-password"
              @keyup.enter="submitPasswordChange"
            />
          </el-form-item>
          <div class="login-actions">
            <el-button
              type="primary"
              :loading="authLoading"
              :disabled="!passwordForm.currentPassword || !passwordForm.newPassword || !passwordForm.confirmPassword"
              @click="submitPasswordChange"
            >
              修改密码
            </el-button>
            <el-button :disabled="authLoading" @click="submitLogout">退出</el-button>
          </div>
        </el-form>
      </el-card>
    </main>
    <main v-else class="page-shell">
      <div class="topbar">
        <span>
          {{ currentUser.username }} ·
          {{ roleLabel(currentUser.effective_role) }}
        </span>
        <el-button size="small" @click="submitLogout">退出</el-button>
      </div>
      <el-tabs v-model="activeTab" class="main-tabs" @tab-change="refreshActiveTab">
        <el-tab-pane label="节点监控" name="nodes">
          <NodesPanel ref="nodesPanel" :role="currentUser.effective_role" />
        </el-tab-pane>
        <el-tab-pane label="配置" name="config">
          <ConfigPanel ref="configPanel" :role="currentUser.effective_role" />
        </el-tab-pane>
        <el-tab-pane label="运行环境" name="runtime">
          <RuntimeEnvironmentsPanel ref="runtimePanel" :role="currentUser.effective_role" />
        </el-tab-pane>
        <el-tab-pane label="模型" name="models">
          <ModelsPanel ref="modelsPanel" :role="currentUser.effective_role" />
        </el-tab-pane>
        <el-tab-pane label="实例" name="instances">
          <InstancesPanel ref="instancesPanel" :role="currentUser.effective_role" />
        </el-tab-pane>
        <el-tab-pane label="模型垃圾箱" name="trash">
          <TrashPanel ref="trashPanel" :role="currentUser.effective_role" />
        </el-tab-pane>
        <el-tab-pane label="日志审计" name="logs">
          <LogsAuditPanel ref="logsPanel" :role="currentUser.effective_role" />
        </el-tab-pane>
        <el-tab-pane label="采集器登记" name="collectors">
          <CollectorRegistryPanel ref="collectorsPanel" :role="currentUser.effective_role" />
        </el-tab-pane>
        <el-tab-pane v-if="currentUser.effective_role === 'admin'" label="用户与组" name="users">
          <UsersPanel ref="usersPanel" />
        </el-tab-pane>
      </el-tabs>
    </main>
  </el-config-provider>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ElMessage } from 'element-plus'
import zhCn from 'element-plus/es/locale/lang/zh-cn'
import ConfigPanel from './components/ConfigPanel.vue'
import InstancesPanel from './components/InstancesPanel.vue'
import ModelsPanel from './components/ModelsPanel.vue'
import NodesPanel from './components/NodesPanel.vue'
import RuntimeEnvironmentsPanel from './components/RuntimeEnvironmentsPanel.vue'
import TrashPanel from './components/TrashPanel.vue'
import LogsAuditPanel from './components/LogsAuditPanel.vue'
import CollectorRegistryPanel from './components/CollectorRegistryPanel.vue'
import UsersPanel from './components/UsersPanel.vue'
import {
  changePassword,
  fetchCurrentUser,
  fetchSetupStatus,
  login,
  logout,
  setupAdmin,
  type AuthUser,
  type Role
} from './api'

const activeTab = ref('nodes')
const currentUser = ref<AuthUser | null>(null)
const setupRequired = ref(false)
const authLoading = ref(false)
const authMessage = ref('')
const loginForm = ref({
  username: '',
  password: ''
})
const setupForm = ref({
  username: 'admin',
  password: ''
})
const passwordForm = ref({
  currentPassword: '',
  newPassword: '',
  confirmPassword: ''
})
const nodesPanel = ref<InstanceType<typeof NodesPanel> | null>(null)
const configPanel = ref<InstanceType<typeof ConfigPanel> | null>(null)
const runtimePanel = ref<InstanceType<typeof RuntimeEnvironmentsPanel> | null>(null)
const modelsPanel = ref<InstanceType<typeof ModelsPanel> | null>(null)
const instancesPanel = ref<InstanceType<typeof InstancesPanel> | null>(null)
const trashPanel = ref<InstanceType<typeof TrashPanel> | null>(null)
const logsPanel = ref<InstanceType<typeof LogsAuditPanel> | null>(null)
const collectorsPanel = ref<InstanceType<typeof CollectorRegistryPanel> | null>(null)
const usersPanel = ref<InstanceType<typeof UsersPanel> | null>(null)

function refreshActiveTab(name: string | number) {
  const refreshers: Record<string, (() => void | Promise<void>) | undefined> = {
    nodes: nodesPanel.value?.refresh,
    config: configPanel.value?.refresh,
    runtime: runtimePanel.value?.refresh,
    models: modelsPanel.value?.refresh,
    instances: instancesPanel.value?.refresh,
    trash: trashPanel.value?.refresh,
    logs: logsPanel.value?.refresh,
    collectors: collectorsPanel.value?.refresh,
    users: usersPanel.value?.refresh
  }
  void refreshers[String(name)]?.()
}

function roleLabel(role: Role) {
  if (role === 'admin') return '管理员'
  if (role === 'operator') return '运维'
  return '只读'
}

async function submitLogin() {
  authLoading.value = true
  authMessage.value = ''
  try {
    currentUser.value = await login(loginForm.value.username, loginForm.value.password)
    loginForm.value.password = ''
    if (!currentUser.value.must_change_password) {
      refreshActiveTab(activeTab.value)
    }
  } catch (error) {
    authMessage.value = error instanceof Error ? error.message : '登录失败'
  } finally {
    authLoading.value = false
  }
}

async function submitSetup() {
  authLoading.value = true
  authMessage.value = ''
  try {
    currentUser.value = await setupAdmin(setupForm.value.username, setupForm.value.password)
    setupForm.value.password = ''
    setupRequired.value = false
    refreshActiveTab(activeTab.value)
  } catch (error) {
    authMessage.value = error instanceof Error ? error.message : '初始化失败'
  } finally {
    authLoading.value = false
  }
}

async function submitPasswordChange() {
  authMessage.value = ''
  if (passwordForm.value.newPassword !== passwordForm.value.confirmPassword) {
    authMessage.value = '两次输入的新密码不一致'
    return
  }
  authLoading.value = true
  try {
    await changePassword(passwordForm.value.currentPassword, passwordForm.value.newPassword)
    passwordForm.value = {
      currentPassword: '',
      newPassword: '',
      confirmPassword: ''
    }
    currentUser.value = null
    activeTab.value = 'nodes'
    ElMessage.success('密码已修改，请重新登录')
  } catch (error) {
    authMessage.value = error instanceof Error ? error.message : '修改密码失败'
  } finally {
    authLoading.value = false
  }
}

async function submitLogout() {
  await logout().catch(() => {})
  currentUser.value = null
  activeTab.value = 'nodes'
  ElMessage.info('已退出登录')
}

onMounted(async () => {
  try {
    setupRequired.value = await fetchSetupStatus()
    if (setupRequired.value) {
      authMessage.value = '请创建第一个管理员账号'
      return
    }
    currentUser.value = await fetchCurrentUser()
    if (!currentUser.value.must_change_password) {
      refreshActiveTab(activeTab.value)
    }
  } catch {
    authMessage.value = '请登录后继续'
  }
})
</script>

<style scoped>
.login-shell {
  min-height: 100vh;
  display: grid;
  place-items: center;
  background: #f5f7fa;
}

.login-card {
  width: min(420px, calc(100vw - 32px));
}

.login-title {
  font-weight: 600;
}

.topbar {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 12px;
  margin-bottom: 8px;
  color: #606266;
  font-size: 13px;
}

.alert {
  margin-bottom: 12px;
}

.login-actions {
  display: flex;
  gap: 8px;
}
</style>
