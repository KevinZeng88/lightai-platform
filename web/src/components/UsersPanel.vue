<template>
  <section>
    <el-alert
      title="本地用户与用户组用于控制面登录和基础权限；用户组是后续部门、项目、API Key、额度和优先级归属的基础对象。"
      type="info"
      show-icon
      class="panel-alert"
    />
    <el-tabs v-model="activeTab">
      <el-tab-pane label="用户" name="users">
    <el-form :inline="true" class="user-form" @submit.prevent>
      <el-form-item label="用户名">
        <el-input v-model="form.username" placeholder="operator" />
      </el-form-item>
      <el-form-item label="密码">
        <el-input v-model="form.password" type="password" show-password placeholder="至少 12 位" />
      </el-form-item>
      <el-form-item label="确认密码">
        <el-input v-model="form.confirmPassword" type="password" show-password placeholder="请再次输入密码" />
      </el-form-item>
      <el-form-item label="角色">
        <el-select v-model="form.role" style="width: 180px">
          <el-option label="管理员 admin" value="admin" />
          <el-option label="运维 operator" value="operator" />
          <el-option label="只读 viewer" value="viewer" />
        </el-select>
        <span class="muted">{{ roleDesc(form.role) }}</span>
      </el-form-item>
      <el-form-item>
        <el-button type="primary" :loading="saving" @click="submitCreate">新增用户</el-button>
      </el-form-item>
    </el-form>
    <el-table :data="users" v-loading="loading" border>
      <el-table-column prop="username" label="用户名" min-width="160" />
      <el-table-column label="角色" width="120">
        <template #default="{ row }">
          <el-tag :type="row.effective_role === 'admin' ? 'danger' : 'info'">
            {{ roleLabel(row.role) }}
            <span v-if="row.effective_role !== row.role"> / 继承管理员</span>
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column label="状态" width="110">
        <template #default="{ row }">
          <el-tag :type="row.enabled ? 'success' : 'warning'">
            {{ row.enabled ? '启用' : '禁用' }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column label="操作" width="260">
        <template #default="{ row }">
          <el-button size="small" @click="toggleUser(row)">
            {{ row.enabled ? '禁用' : '启用' }}
          </el-button>
          <el-button size="small" @click="toggleRole(row)">
            {{ row.role === 'admin' ? '设为只读' : '设为管理员' }}
          </el-button>
        </template>
      </el-table-column>
    </el-table>
      </el-tab-pane>
      <el-tab-pane label="用户组" name="groups">
        <el-form :inline="true" class="user-form" @submit.prevent>
          <el-form-item label="组名">
            <el-input v-model="groupForm.name" placeholder="platform-team" />
          </el-form-item>
          <el-form-item label="组角色">
            <el-select v-model="groupForm.role" style="width: 120px">
              <el-option label="只读" value="viewer" />
              <el-option label="运维" value="operator" />
              <el-option label="管理员" value="admin" />
            </el-select>
          </el-form-item>
          <el-form-item>
            <el-button type="primary" :loading="savingGroup" @click="submitCreateGroup">
              新增用户组
            </el-button>
          </el-form-item>
        </el-form>
        <el-table :data="groups" v-loading="loadingGroups" border>
          <el-table-column prop="name" label="组名" min-width="150" />
          <el-table-column label="编辑组名" min-width="180">
            <template #default="{ row }">
              <el-input
                :model-value="row.name"
                size="small"
                @change="(name: string) => renameGroup(row, name)"
              />
            </template>
          </el-table-column>
          <el-table-column label="角色" width="120">
            <template #default="{ row }">
              <el-select
                :model-value="row.role"
                size="small"
                style="width: 100px"
                @change="setGroupRole(row, $event)"
              >
                <el-option label="只读" value="viewer" />
                <el-option label="运维" value="operator" />
                <el-option label="管理员" value="admin" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column label="状态" width="110">
            <template #default="{ row }">
              <el-tag :type="row.enabled ? 'success' : 'warning'">
                {{ row.enabled ? '启用' : '禁用' }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column label="成员" min-width="260">
            <template #default="{ row }">
              <el-select
                :model-value="groupMemberIds(row)"
                multiple
                collapse-tags
                collapse-tags-tooltip
                placeholder="选择成员"
                style="width: 100%"
                @change="setGroupMembers(row, $event)"
              >
                <el-option
                  v-for="user in users"
                  :key="user.id"
                  :label="user.username"
                  :value="user.id"
                />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column label="操作" width="190">
            <template #default="{ row }">
              <el-button size="small" @click="toggleGroup(row)">
                {{ row.enabled ? '禁用' : '启用' }}
              </el-button>
              <el-button size="small" type="danger" plain @click="removeGroup(row)">
                删除
              </el-button>
            </template>
          </el-table-column>
        </el-table>
      </el-tab-pane>
    </el-tabs>
  </section>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ElMessage } from 'element-plus'
import {
  createGroup,
  createUser,
  deleteGroup,
  fetchGroups,
  fetchUsers,
  updateGroup,
  updateGroupMembers,
  updateUser,
  type AuthUser,
  type Role,
  type UserGroup
} from '../api'

const activeTab = ref('users')
const users = ref<AuthUser[]>([])
const groups = ref<UserGroup[]>([])
const loading = ref(false)
const loadingGroups = ref(false)
const saving = ref(false)
const savingGroup = ref(false)
const form = ref({
  username: '',
  password: '',
  confirmPassword: '',
  role: 'viewer' as Role
})
const groupForm = ref({
  name: '',
  role: 'viewer' as Role
})

async function refresh() {
  loading.value = true
  try {
    users.value = await fetchUsers()
  } finally {
    loading.value = false
  }
}

async function refreshGroups() {
  loadingGroups.value = true
  try {
    groups.value = await fetchGroups()
  } finally {
    loadingGroups.value = false
  }
}

function roleLabel(role: Role) {
  if (role === 'admin') return '管理员 admin'
  if (role === 'operator') return '运维 operator'
  return '只读 viewer'
}

function roleDesc(role: Role) {
  if (role === 'admin') return '可管理用户、配置、节点、模型、实例、审计'
  if (role === 'operator') return '可管理模型、Runtime、实例启停和状态检查'
  return '只读查看节点、GPU、模型、实例、日志和配置'
}

function groupMemberIds(group: UserGroup) {
  return group.members.map((member) => member.id)
}

async function submitCreate() {
  if (form.value.password !== form.value.confirmPassword) {
    ElMessage.error('两次输入的密码不一致')
    return
  }
  saving.value = true
  try {
    await createUser({
      username: form.value.username,
      password: form.value.password,
      role: form.value.role
    })
    form.value.username = ''
    form.value.password = ''
    form.value.confirmPassword = ''
    form.value.role = 'viewer'
    ElMessage.success('用户已创建')
    await Promise.all([refresh(), refreshGroups()])
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '创建用户失败')
  } finally {
    saving.value = false
  }
}

async function toggleUser(user: AuthUser) {
  try {
    await updateUser(user.id, { enabled: !user.enabled })
    await Promise.all([refresh(), refreshGroups()])
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '更新用户失败')
  }
}

async function toggleRole(user: AuthUser) {
  try {
    await updateUser(user.id, { role: user.role === 'admin' ? 'viewer' : 'admin' })
    await Promise.all([refresh(), refreshGroups()])
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '更新用户失败')
  }
}

async function submitCreateGroup() {
  savingGroup.value = true
  try {
    await createGroup({ name: groupForm.value.name, role: groupForm.value.role })
    groupForm.value.name = ''
    groupForm.value.role = 'viewer'
    ElMessage.success('用户组已创建')
    await refreshGroups()
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '创建用户组失败')
  } finally {
    savingGroup.value = false
  }
}

async function toggleGroup(group: UserGroup) {
  try {
    await updateGroup(group.id, { enabled: !group.enabled })
    await Promise.all([refresh(), refreshGroups()])
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '更新用户组失败')
  }
}

async function setGroupRole(group: UserGroup, value: unknown) {
  const role: Role = value === 'admin' ? 'admin' : value === 'operator' ? 'operator' : 'viewer'
  try {
    await updateGroup(group.id, { role })
    await Promise.all([refresh(), refreshGroups()])
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '更新用户组失败')
  }
}

async function renameGroup(group: UserGroup, name: string) {
  try {
    await updateGroup(group.id, { name })
    await refreshGroups()
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '更新组名失败')
  }
}

async function setGroupMembers(group: UserGroup, value: unknown) {
  const userIds = Array.isArray(value) ? value.filter((id): id is string => typeof id === 'string') : []
  try {
    await updateGroupMembers(group.id, userIds)
    await Promise.all([refresh(), refreshGroups()])
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '更新成员失败')
  }
}

async function removeGroup(group: UserGroup) {
  try {
    await deleteGroup(group.id)
    ElMessage.success('用户组已删除')
    await refreshGroups()
  } catch (error) {
    ElMessage.error(error instanceof Error ? error.message : '删除用户组失败')
  }
}

async function refreshAll() {
  await Promise.all([refresh(), refreshGroups()])
}

onMounted(refreshAll)

defineExpose({ refresh: refreshAll })
</script>

<style scoped>
.panel-alert {
  margin-bottom: 12px;
}

.user-form {
  margin-bottom: 12px;
}
</style>
