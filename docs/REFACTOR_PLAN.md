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

## 剩余 TODO

- `tests/instance_lifecycle_api.rs` 仍 2805 行，可后续按测试域拆分
- `repository.rs` / `routes.rs` 后续可拆分
- 未引入新依赖、未改 API/DB/行为

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
