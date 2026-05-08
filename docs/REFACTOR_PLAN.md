# Refactor Plan

本文件是历史执行记录，用于了解已完成的结构整理。不要把其中的旧阶段约束当作当前必须遵守的开发计划；当前任务以 README、ARCHITECTURE、IMPLEMENTATION_NOTES 和用户请求为准。

## 已完成

### ✅ server/src/domain.rs 拆分

`domain.rs` 已变为 43 行轻量 facade，仅含 `mod` 声明和 `pub use` re-export。业务逻辑已拆入 7 个子模块：

| 模块 | 职责 | 行数 |
|------|------|------|
| `domain/runtimes.rs` | 运行环境 CRUD、Agent 检查 | 402 |
| `domain/instances.rs` | 实例 CRUD、start/stop/test/check、任务创建 | 682 |
| `domain/model_catalog.rs` | 模型 CRUD | 246 |
| `domain/model_files.rs` | 模型文件 CRUD、验证、路径检查 | 426 |
| `domain/model_trash.rs` | 模型文件垃圾箱、清理 | 264 |
| `domain/instance_logs.rs` | 实例日志读取、刷新、错误摘要 | 253 |
| `domain/support.rs` | Stage3Error、验证函数、常量、guard helpers | 238 |

routes.rs 继续通过 `domain::function()` 透明调用各子模块。

### ✅ server/src/stage3a.rs → domain.rs 重命名

`stage3a.rs` 已删除，`lib.rs` 和 `routes.rs` 已更新。

### ✅ server/src/agent_tasks.rs 提取

Agent 任务生命周期（poll / record / timeout / notify）的唯一实现已独立为 `agent_tasks.rs`（494 行）。domain 模块通过 re-export 保持兼容。

## 后续计划（已执行）

### ✅ 1. agent/src/tasks.rs 拆分（已完成）

原 1749 行已拆为 `agent/src/tasks/` 目录：

| 模块 | 职责 |
|------|------|
| `tasks/mod.rs` | facade：module 声明 + re-export + run/run_once 调度 + 共享类型 + helper（535 行） |
| `tasks/probe.rs` | 就绪探测配置、测试 URL 构建、失败摘要 |
| `tasks/process.rs` | 实例启停、进程管理、监控、日志缓冲 |
| `tasks/verify_model.rs` | 模型文件验证 |
| `tasks/cleanup.rs` | 受控模型文件清理 |
| `tasks/logs.rs` | 实例日志读取 |

### ✅ 2. server/tests/stage3a_api.rs 整理（已完成）

- 重命名为 `tests/instance_lifecycle_api.rs`（2805 行，去掉 stage3a 代号）
- 测试覆盖未减少（仍 49 项测试全部通过）

### ✅ 3. web/src/components/InstancesPanel.vue 整理（已完成）

- 原 ~678 行 → 616 行
- 提取 `web/src/utils/instance.ts`（61 行）：statusType / statusLabel / deployTypeLabel / runtimeDeployTypeLabel / backendLabel / checkFailedReason / formatTime / emptyToNull
- ModelsPanel.vue 也已统一使用 `utils/instance.ts` 的 `emptyToNull` / `formatTime`

### ⏭️ 4. server/src/repository.rs / routes.rs（跳过后继续）

repository.rs（1255 行）、routes.rs（981 行）功能稳定，非紧急，本轮跳过。

### ✅ 5. Docker 后端（已完成）

- `agent/src/tasks/docker_backend.rs` 新增（~1000 行）
- Docker run/stop/inspect/logs 命令构造、容器管理、状态解析
- 三层配置模型：model + runtime + instance → docker run
- `merge_docker_config()` 合并优先级：instance override > runtime default
- managed store 扩展支持 container_id/container_name
- Docker 与 local 统一生命周期（start/stop/check/logs/recover）
- Docker 容器默认不加 --rm；Agent 退出不停止容器
- 23 项 Docker 相关测试

### ✅ 6. Web 产品模型落地（已完成）

- ModelsPanel：结构化模型元数据配置（path_type/model_format/supported_backends）
- RuntimeEnvironmentsPanel：Docker vLLM 结构化表单 + 可选参数开关
- InstancesPanel：按 Runtime deploy_type 切换 Docker/local 表单
- Runtime 默认值自动带入 Instance 表单；Instance 覆盖参数保存不回写 Runtime
- 模板按钮替换为结构化字段填充
- extra args 逐行输入（linesToArgs/argsToLines）
- params_json 作为内部承载自动生成和反解析
- Runtime params_json 保存/回显链路修复（Server 端补齐字段）

### ✅ 6. 可观测性与 Agent 离线展示（已完成）

- 日志时间 ISO 8601；关键路径日志补齐
- 进程隔离（Agent 退出不终止模型实例）
- Agent 离线 Web 自动展示（warning 标签，不误改状态）
- 测试 +3 项

### ✅ 7. Docker 实例完整体验（已完成）

**Docker Runtime 结构化表单：**
- GPU/IPC 始终显示（移除 toggle），默认值 "all"/"host"
- container_port 统一为 "容器内服务端口"，defaults.port 自动同步
- Host 不在 UI 配置，内部固定 0.0.0.0
- 旧数据兼容：parseDockerRuntimeParams 容错回填

**Instance Docker 参数覆盖表单：**
- Runtime 默认值显示具体数值 + "来自运行环境" 标签
- 覆盖字段显示 "实例覆盖" 标签 + "恢复默认" 按钮
- 保存时只写真正覆盖的字段，不重复 Runtime 默认值
- container_port 只读来自 Runtime，host_port 实例可编辑

**运行中资源锁定：**
- running/starting/stopping 的 Instance 不能编辑配置（Server 409 + Web disabled）
- 被此类 Instance 引用的 Runtime/Model 不能修改
- 允许 status-only update（管理员强制修改状态）

**Agent 三层配置合并：**
- `resolve_docker_payload` → `merge_docker_config`：Model + Runtime + Instance
- GPU 默认 "all"，ipc 默认 "host"（修复 CUDA 不可用 bug）
- port 映射：host_port(Instance) : container_port(Runtime)，vLLM --port = container_port

**Docker 命令日志：**
- start/stop/inspect 操作记录 command_summary 到 agent.log
- 失败时额外记录 stderr 摘要，command_summary 写入 last_error
- Web 日志页面可见

**按钮状态：**
- stopped/failed/created/unknown → 可启动
- starting/stopping → 都不可点
- running + node_online → 可停止
- running + node_offline → 不可操作

## 剩余 TODO

- Docker 实例端到端真实环境验证（vllm/vllm-openai:latest + qwen3-0.6b）
- 未来多节点通过 Deployment / Replica 扩展
- `repository.rs` / `routes.rs` 后续可拆分（低优先级）
- 未引入新依赖、未改 API/DB schema（params_json 复用 config_json 列）

## 每轮验收标准

```bash
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd web && npm run build
```

## 不可逾越的红线

- 不修改 API 路径、请求/响应结构、数据库 schema、状态语义
- 不修改前端代码（除非明确拆分 Web 组件）
- 不引入新依赖
- 不更新业务逻辑、不顺手优化
- 不按行号切割 Rust 代码
- 不创建空模块或预留文件
