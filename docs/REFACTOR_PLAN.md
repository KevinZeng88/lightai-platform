# Refactor Plan — domain.rs 拆分

本文件供 Codex / Claude Code 读取，用于指导 `server/src/domain.rs` 的继续拆分。

## 当前状态

- `server/src/stage3a.rs` 已删除。已改名为 `domain.rs`。
- `server/src/agent_tasks.rs` 已提取：Agent 任务生命周期（poll / record / timeout / notify）的唯一实现。
- `server/src/util.rs` 已提取：`now_unix_secs()`。
- `server/src/domain.rs` 当前约 **2444 行**，承载运行环境、模型、模型文件、实例、垃圾箱、日志、验证等全部业务逻辑。
- `domain.rs` 的 agent task 函数（poll_agent_task 等）**已移除**，仅通过 `use crate::agent_tasks;` 调用。
- `update_trash_failure` 和 `update_instance_check` 已提升为 `pub(crate)`（agent_tasks.rs 需要调用）。

## 最终目标

`domain.rs` 变为很薄的 facade（≤50 行），只做 `pub use` re-export。或者完全删除，由 `routes.rs` 直接引用各业务模块。

## 严格边界

每次拆分必须遵守以下约束：

- **不修改 API 路径、请求/响应结构、数据库 schema、状态语义**
- **不修改前端代码**
- **不引入新依赖**
- **不更新业务逻辑、不顺手优化**
- **不按行号切割**——只移动完整的 Rust top-level item（`pub fn`、`fn`、`struct`、`enum`、`impl` 等）
- **每轮只拆一个业务模块**
- **每轮必须运行完整检查**

## 建议拆分顺序（从依赖最少到最多）

### 1. runtimes.rs

目标模块：`server/src/domain/runtimes.rs`

函数清单：
- `create_runtime_environment`
- `list_runtime_environments`
- `runtime_environment`
- `update_runtime_environment`
- `delete_runtime_environment`
- `check_runtime_environment`
- `update_runtime_environment_check`
- `check_runtime_environment_before_save`
- `runtime_check_message`
- `runtime_environment_usable`
- `CheckedRuntimeEnvironment`（struct）
- `runtime_environment_from_row`

依赖：`agent_tasks`、`validation`（validate_*）、`error`（Stage3Error）

### 2. instance_logs.rs 或 logs.rs

目标模块：`server/src/domain/logs.rs`

函数清单：
- `read_agent_log`
- `refresh_instance_logs`
- `frontend_error_summary`
- `recent_error_summary`

依赖：`agent_tasks`、`instances`、`runtimes`、`error`

### 3. model_trash.rs

目标模块：`server/src/domain/model_trash.rs`

函数清单：
- `create_model_file_trash`
- `ensure_model_file_trash`
- `list_model_file_trash`
- `cleanup_model_file_trash`
- `delete_model_file_trash`
- `model_file_trash_item`
- `wait_for_model_file_cleanup`
- `update_trash_failure`（已经是 pub(crate)）
- `model_file_trash_from_row`

依赖：`agent_tasks`、`model_files`、`validation`、`error`

### 4. model_files.rs

目标模块：`server/src/domain/model_files.rs`

函数清单：
- `create_model_file`、`list_model_files`、`model_file`、`update_model_file`、`delete_model_file`
- `queue_model_file_verification`、`verify_model_file_before_save`、`wait_for_model_file_verification`
- `verification_error_message`
- `VerifiedModelFile`（struct）
- `ModelFileSummary`（struct）
- `model_file_summary`、`model_file_rows`、`model_file_from_row`

依赖：`agent_tasks`、`validation`、`repository`、`error`

### 5. model_catalog.rs

目标模块：`server/src/domain/model_catalog.rs`

函数清单：
- `create_model`、`list_models`、`model`、`update_model`、`delete_model`
- `model_from_row`

依赖：`agent_tasks`、`validation`、`model_files`、`error`

### 6. instances.rs

目标模块：`server/src/domain/instances.rs`

函数清单：
- `create_model_instance`、`create_local_model_instance`
- `list_model_instances`、`model_instance`
- `update_model_instance`、`delete_model_instance`
- `check_model_instance`、`start_model_instance`、`stop_model_instance`、`test_model_instance`
- `run_local_instance_task`、`wait_for_model_instance_task`
- `update_instance_check`（已经是 pub(crate)）
- `verified_model_file_for_instance`
- `InstanceModelFile`（struct）
- `model_instance_from_row`

依赖：`agent_tasks`、`runtimes`、`model_catalog`、`model_files`、`validation`、`error`

> instances.rs 依赖最多，**必须最后拆分**。

## 每轮操作流程

1. 从 `domain.rs` 中提取目标模块的所有 top-level item，创建 `server/src/domain/<module>.rs`。
2. 在 `server/src/domain/mod.rs` 中添加 `mod <module>;` 和 `pub use`。
3. 从 `domain.rs` 中删除已移动的 item。
4. 修复跨模块引用（如 `runtime_environment()` → `runtimes::runtime_environment()`）。
5. 运行完整检查：
   ```bash
   cargo fmt --all --check
   cargo build --workspace
   cargo test --workspace
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cd web && npm run build
   ```
6. 所有检查通过后，输出：
   - 移动了哪些函数/struct
   - domain.rs 剩余行数
   - 是否存在重复逻辑
   - 测试结果
   - clippy 是否 0 warning
   - git diff 摘要

## 关键注意事项

- **不要按行号切割**。Rust 嵌套大括号、多行签名、raw string 导致行号提取不可靠。必须通过完整 item 边界（brace counting）识别要移动的代码段。
- **保留 agent_tasks.rs 的唯一性**。不要向 domain.rs 或其他模块复制 agent task 函数。
- **validation 模块**：验证函数（validate_*、ensure_*、node_online 等）可暂留 domain.rs，等所有业务模块拆分完毕后再统一提取。
- **model_files 和 model_catalog 的边界**：`model_file_from_row`、`model_instance_from_row`、`model_file_trash_from_row` 等 row mapper 函数应跟随其对应的业务模块。
- **不要创建空模块或预留文件**。

## 验收标准

最终状态：
- `domain.rs`（或 `domain/mod.rs`）≤ 100 行，仅含 re-exports + 共享常量
- 所有业务模块独立，职责清晰
- 92 项测试全部通过
- clippy 0 warning
- 无重复逻辑
