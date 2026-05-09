# AI Handoff
Language convention:
- CLI help, CLI operational output, server logs, and agent logs must be English.
- Web UI text should be Chinese.
- Documentation may be Chinese, but command examples, config keys, status values, and log examples should remain English.

## 当前真实状态

- 仓库是 Rust workspace + Vue/Vite Web monorepo，主要目录为 `server/`、`agent/`、`web/`、`migrations/`、`deploy/`、`docs/`。
- 产品最终目标是企业级模型服务与 GPU 资源调度平台，不是单纯 GPU 服务器管理工具。当前代码处于第一阶段：GPU 服务器统一纳管、Agent 心跳/GPU 状态上报、基础模型/Runtime/实例管理、Web 控制台、本地用户、用户组和基础权限。
- Server 使用 Axum + SQLite，提供本地用户/用户组登录与权限基础、Agent 注册/心跳、节点与 GPU 指标、配置策略、Runtime、Model、Model File、Instance、Trash、日志、前端错误和审计 API。
- 控制面 API 使用本地用户会话保护；除 `/health`、`/api/setup/*`、`/api/auth/login` 与 `/api/agent/*` 外，所有 `/api/*` 请求都需要登录 cookie。空库首次访问 Web 进入 setup 页面，生产配置不支持 `initial_admin_password` 或 `LIGHTAI_ADMIN_PASSWORD`。
- Agent 运行在 GPU 节点，主动注册 Server，按心跳上报 CPU/内存/磁盘/GPU 指标和受管实例状态，并通过任务轮询执行受控动作。
- Web 是 Vue 3 + Vite + Element Plus 控制台，包含节点监控、Agent 配置、运行环境、模型、实例、垃圾箱、日志审计、用户与组页面。
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
7. 当前不要实现 API Gateway、API Key、配额、计量、调度优先级或计费；这些是后续阶段目标，不应在文档中写成永久范围外。
8. 用户组只做成员关系和组角色继承，作为后续部门、项目、业务系统和 API Key 归属基础；不要扩展成复杂 IAM。
9. 当前角色只有 `admin`、`operator`、`viewer`；后端统一计算 `effective_role`。`admin` 管理用户/组、配置和 Trash 清理，`operator` 管理 Runtime/模型/实例，`viewer` 只读。这是轻量内置角色，不是完整 RBAC。后续不要轻易把当前三角色扩展成可配置权限矩阵；如需扩展权限，应先设计 API Key、租户和计费边界。忘记密码通过服务器本机 `lightai-server --reset-password <USERNAME> <PASSWORD>` 恢复，重置后用户必须修改密码。后端已实现最后一个 admin 保护（不能禁用或降级最后一个启用的管理员），当前无用户删除功能。

## 代码地图

```text
server/src/
  routes.rs              # Axum 路由和 HTTP handler
  models.rs              # API 请求/响应类型
  repository.rs          # 用户、用户组、会话、节点注册、心跳、指标、配置、审计、reconcile
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
  gpu/                   # 脚本化 GPU collector 调度与 registry/hash 校验
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
- `0002_stage2_nodes.sql` — 节点、node_status（含所有配置字段）、gpu_status、指标采样表。
- `0003_stage3a_models.sql` — Runtime、Model、Model Instance、Model File、Agent Task、Trash 表。
- `0005_platform.sql` — 用户、session、用户组、审计、配置策略、平台设置、collector registry 表。
- `server/src/db.rs` 启动时按序执行上述 SQL 文件并创建唯一索引。不兼容历史数据库，旧数据库删除后重建。

## 已知限制和风险

- Docker/vLLM 未在真实 GPU 环境完成完整验收。
- 模型文件验证只证明路径存在并可读基础信息，不证明模型格式正确或推理服务可用。
- 手工 kill local 受管进程后，状态同步到 Web 最坏约 33 秒（Agent monitor 3s + heartbeat 15s + Web refresh 15s）。
- 模型垃圾箱不支持批量清理、定时清理或目录递归删除。
- 前端错误上报是 fire-and-forget，网络失败时静默丢失。
- 审计页面是基础列表和筛选，有默认 limit 500（最大 1000）和 offset 分页，但没有详情展开或导出。
- 历史指标采样数据会自动清理（默认保留 7 天），但不做小时聚合或降采样。趋势图展示的是原始采样点。
- 统一模型调用 API、API Key、额度、计量、调用延迟/错误率/吞吐统计、GPU 调度优先级、自动扩缩容、降级和费用归集仍未实现。

## 后续阶段方向

1. 第二阶段：模型服务管理与统一调用入口，包括 OpenAI-compatible Gateway、模型路由和调用认证。
2. 第三阶段：API Key、部门/项目/业务系统归属、额度、限流、调用统计和基础计量。
3. 第四阶段：GPU 资源调度、关键模型优先级、扩缩容和降级策略。
4. 第五阶段：费用归集、SLA、审计分析、运营报表和企业级治理能力。

## 后续建议优先级

1. 在真实 NVIDIA GPU 环境验证脚本 collector + Docker vLLM 端到端：登记 collector、创建 Runtime、模型目录、实例启动、健康检查、日志、停止、Agent 重启恢复、异常退出诊断。
2. 缩短受管进程异常退出到 Server/Web 的同步延迟，例如心跳携带更明确的退出事件或任务结果。
3. 在本地运行层稳定后，再推进统一模型调用入口、API Key 和用量统计。

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

## Release 打包

- **glibc2.28 包**（推荐）：`bash scripts/package-release-docker.sh v0.1.0` — 在 Rocky Linux 8 容器内编译，生成企业兼容包。
- **native 包**：`bash scripts/package-release.sh v0.1.0 native` — 宿主机直接编译，仅限本机测试。
- 不建议直接分发 native 包给老系统。跨服务器测试优先使用 glibc2.28 包。
- 如果目标服务器仍报 `GLIBC_x.xx not found`，先记录 `getconf GNU_LIBC_VERSION` 和 `ldd --version`，不要建议升级 glibc。
- 详见 [IMPLEMENTATION_NOTES.md](IMPLEMENTATION_NOTES.md) 的 "Release 包类型" 章节。

实现细节见 [IMPLEMENTATION_NOTES.md](IMPLEMENTATION_NOTES.md)；真实环境步骤见 [LOCAL_TEST_ENV.md](LOCAL_TEST_ENV.md)。
