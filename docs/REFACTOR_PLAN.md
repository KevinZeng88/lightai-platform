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

## 后续计划

### 1. agent/src/tasks.rs 拆分（建议优先）

当前约 1750 行，混合了实例启停、模型验证、文件清理、日志读取、启动参数构建等多种逻辑。

建议按功能拆分为：
- `agent/src/tasks/start_instance.rs`
- `agent/src/tasks/verify_model.rs`
- `agent/src/tasks/cleanup.rs`
- `agent/src/tasks/logs.rs`
- `agent/src/tasks/params.rs`（InstanceLaunchParams、ProbeConfig 等）

严格边界同 domain 拆分（不改 API/行为/前端/DB，只移动完整 item，每轮一个模块，每轮完整检查）。

### 2. server/tests/stage3a_api.rs 整理

- 重命名为 `instance_lifecycle_api.rs` 或按测试域拆分
- 补充 Web 层关键逻辑的单元测试（checkFailedReason、statusType 等）

### 3. web/src/components/InstancesPanel.vue 整理

当前约 680 行，可提取 composables：
- `useInstanceForm.ts`
- `useInstanceOperations.ts`
- `useProbeConfig.ts`

### 4. server/src/repository.rs / routes.rs（低优先级）

repository.rs 约 1260 行，routes.rs 约 990 行。功能稳定，接口明确。可后续拆分，非紧急。

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
