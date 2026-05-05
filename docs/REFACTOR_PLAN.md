# Refactor Plan

本文件供 Codex / Claude Code 读取，用于指导后续代码结构整理。

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

- `agent/src/tasks/docker_backend.rs` 新增（~950 行）
- Docker run/stop/inspect/logs 命令构造、容器管理、状态解析
- 三层配置模型：model + runtime + instance → docker run
- managed store 扩展支持 container_id/container_name
- Docker 与 local 统一生命周期（start/stop/check/logs/recover）
- Web 第一版通过高级 JSON params_json 配置
- Docker 容器默认不加 --rm
- Agent 退出不停止容器
- 23 项 Docker 相关测试

### ✅ 6. 可观测性与 Agent 离线展示（已完成）

**日志时间人可读化：**
- Server/Agent 的 `platform_log::append` 日志时间戳从 Unix timestamp 改为 ISO 8601（如 `2026-05-05T10:23:11Z`）
- 无外部依赖，使用纯 std 日历算法

**关键路径日志补齐：**
- Agent 进程监控：instance_id、pid、exit_status、managed store 保留状态
- Agent 心跳：running/failed 计数 + 失败详情
- Agent 退出：保留 N 条受管进程记录
- Server reconcile：running→failed 写 server log
- Server check_instance：Agent 离线检查写 server log
- Server heartbeat reconcile "未上报" 的 instance 写 server log

**进程隔离（Agent 退出不终止模型实例）：**
- stdin 设为 null
- Unix `process_group(0)` — 子进程进入独立进程组
- Agent 退出时不遍历、不 kill 受管进程
- 环境限制文档说明：systemd KillMode=control-group 或 Docker 容器内需额外注意

**Agent 离线 Web 自动展示：**
- `ModelInstanceView` 新增 `node_online: bool` + `last_heartbeat_at: Option<i64>`
- `web/src/utils/instance.ts` 新增 `instanceStatusLabel` / `isAgentOffline`
- Agent 离线时 running 实例显示 warning 标签 "Agent 离线，运行状态无法确认"
- 周期刷新自动更新，不误改实例状态为 failed

**新增测试（+3 项 → 95 项总计）：**
- `running_instance_on_offline_node_shows_node_online_false`
- `instance_list_includes_node_online_when_agent_offline`
- `running_instance_on_online_node_shows_node_online_true`

## 剩余 TODO

- `tests/instance_lifecycle_api.rs` 仍 ~2900 行，可后续按测试域拆分
- `repository.rs` / `routes.rs` 后续可拆分
- systemd KillMode / Docker 容器生命周期对模型实例进程的影响需真实环境验证
- 未引入新依赖、未改 API/DB/行为（ModelInstanceView 为向后兼容追加字段）

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
