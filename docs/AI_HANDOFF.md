# AI Handoff

## 当前真实状态

- 仓库是 Rust workspace + Vue/Vite Web monorepo，主要目录为 `server/`、`agent/`、`web/`、`migrations/`、`deploy/`、`docs/`。
- Server 使用 Axum + SQLite，提供 Agent 注册/心跳、节点与 GPU 指标、配置策略、Runtime、Model、Model File、Instance、Trash、日志、前端错误和审计 API。
- Agent 运行在 GPU 节点，主动注册 Server，按心跳上报 CPU/内存/磁盘/GPU 指标和受管实例状态，并通过任务轮询执行受控动作。
- Web 是 Vue 3 + Vite + Element Plus 控制台，包含节点监控、Agent 配置、运行环境、模型、实例、垃圾箱、日志审计页面。
- Instance 顶层类型是 `external` 或 `local`；`local` 实例的启动方式来自 Runtime 的 `deploy_type`：`binary`、`script` 或 `docker`。
- Docker 代码路径已实现，包括三层参数合并、`docker run --detach`、`docker stop`、`docker inspect`、`docker logs` 和 managed store 恢复；仍需真实 GPU 环境端到端验证。
- 平台日志已实现脱敏、级别过滤、轮转和保留策略；Server 日志策略可在 Web 更新，Agent 日志策略通过 Agent 配置下发。

## 必守开发约束

1. Agent 是唯一节点本地执行者；Server 不直连 Agent，Web 不直连 Agent 或节点服务。
2. 本地执行必须使用 argv，不构造 shell 命令字符串，不接受前端任意命令。
3. Agent 退出不终止模型实例；只有用户显式 stop 才能停止受管进程或容器。
4. Agent 离线不能把 running 实例误标为 failed；只展示“运行状态无法确认”。
5. running / starting / stopping 的 Instance 及其引用的 Runtime、Model 不能修改。
6. 文档和代码都应保持小改动、低抽象、无不必要依赖。

## 代码地图

```text
server/src/
  routes.rs              # Axum 路由和 HTTP handler
  models.rs              # API 请求/响应类型
  repository.rs          # 节点注册、心跳、指标、配置、审计、reconcile
  agent_tasks.rs         # Agent task poll/result/timeout/notify
  db.rs                  # SQLite 连接、SQL 迁移、幂等 schema 修正
  domain/
    runtimes.rs          # Runtime CRUD 和 Agent 检查
    instances.rs         # Instance CRUD、start/stop/test/check
    model_catalog.rs     # Model CRUD
    model_files.rs       # Model File CRUD 和验证任务
    model_trash.rs       # Trash 和受控物理删除任务
    instance_logs.rs     # Agent/实例日志读取和错误摘要

agent/src/
  main.rs                # Agent HTTP health、heartbeat loop、task loop 并行启动
  heartbeat.rs           # 注册、心跳、指标/GPU/managed report 上报、配置应用
  managed_process.rs     # 受管进程/容器记录持久化和恢复检查
  gpu/                   # NVIDIA nvidia-smi 和 custom collector
  metrics.rs             # CPU/内存/磁盘采集
  tasks/
    mod.rs               # 任务分发
    runtime_check.rs     # Runtime 检查
    process*.rs          # 本地程序/脚本启停、日志、命令构造
    docker_backend.rs    # Docker 启停、inspect、logs、参数合并
    verify_model.rs      # 模型路径验证
    cleanup.rs           # 受控文件删除
    logs.rs              # 实例日志读取

web/src/
  api.ts                 # Server API client
  types.ts               # 前端 API 类型
  components/            # Nodes/Config/Runtime/Models/Instances/Trash/LogsAudit
  components/instances/  # 实例参数和刷新 helper
  utils/templates.ts     # Runtime/Model 模板和兼容性 helper
  utils/instance.ts      # 实例状态、标签和格式化 helper
```

## 数据库与迁移

- `migrations/0001_init.sql` 是占位。
- `0002_stage2_nodes.sql` 创建节点、当前指标和历史指标表。
- `0003_stage3a_models.sql` 创建 Runtime、Model、Model Instance、Model File、Agent Task、Trash 基础表。
- `0004_stage3a_corrections.sql` 是历史修正参考，不由 `db.rs` 自动执行。
- `server/src/db.rs` 启动时执行 0001-0003，并用代码内幂等逻辑补齐后续表/列、唯一索引、审计表、配置策略表和平台设置表。
- 当前没有正式 migration ledger，新增 schema 变更需要谨慎设计幂等升级路径。

## 已知限制和风险

- Docker/vLLM 未在真实 GPU 环境完成完整验收。
- 模型文件验证只证明路径存在并可读基础信息，不证明模型格式正确或推理服务可用。
- Runtime 列表只把 `check_status === "available"` 作为本地实例可选项，Server 也接受 `version_unavailable`；这里存在前端选择范围偏窄。
- 手工 kill local 受管进程后，状态同步到 Web 最坏约 33 秒（Agent monitor 3s + heartbeat 15s + Web refresh 15s）。
- 模型垃圾箱不支持批量清理、定时清理或目录递归删除。
- 前端错误上报是 fire-and-forget，网络失败时静默丢失。
- 审计页面是基础列表和筛选，没有分页、详情展开或导出。
- 历史指标没有自动清理、聚合或降采样。

## 后续建议优先级

1. 在真实 NVIDIA GPU 环境验证 Docker vLLM 端到端：创建 Runtime、模型目录、实例启动、健康检查、日志、停止、Agent 重启恢复、异常退出诊断。
2. 引入正式 migration ledger 或明确 schema 版本策略，减少 `db.rs` 中不断追加的修正逻辑。
3. 缩短受管进程异常退出到 Server/Web 的同步延迟，例如心跳携带更明确的退出事件或任务结果。
4. 增加历史指标清理和基础聚合，避免 SQLite 长期膨胀。
5. 在本地运行层稳定后，再推进 OpenAI-compatible Gateway、API Key 和用量统计。

## 常用验证

```bash
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd web && npm run build
```

本地 GPU 环境额外运行：

```bash
bash scripts/dev_check_nvidia.sh
```

实现细节见 [IMPLEMENTATION_NOTES.md](IMPLEMENTATION_NOTES.md)；真实环境步骤见 [LOCAL_TEST_ENV.md](LOCAL_TEST_ENV.md)。
