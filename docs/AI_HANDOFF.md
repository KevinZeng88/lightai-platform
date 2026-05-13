# AI Handoff

Language convention:
- CLI help, CLI operational output, server logs, and agent logs must be English.
- Web UI text should be Chinese.
- Documentation may be Chinese, but command examples, config keys, status values, and log examples should remain English.

## 当前真实状态

- 仓库是 Rust workspace + Vue/Vite Web monorepo，主要目录为 `server/`、`agent/`、`web/`、`migrations/`、`deploy/`、`docs/`。
- 产品最终目标是企业级模型服务与 GPU 资源调度平台。当前代码处于第一阶段（v0.1）：GPU 服务器统一纳管、Agent 心跳/GPU 状态上报、基础模型/Runtime/实例管理、Web 控制台、本地用户、用户组和基础权限。
- Server 使用 Axum + SQLite，提供 HTTPS（自签 CA）+ 本地用户/用户组登录与权限基础、Agent 注册/心跳、节点与 GPU 指标、配置策略、Runtime、Model、Model File、Instance、Trash、日志、前端错误和审计 API。
- Agent 运行在 GPU 节点，主动注册 Server，按心跳上报 CPU/内存/磁盘/GPU 指标和受管实例状态，并通过任务轮询执行受控动作。
- Web 是 Vue 3 + Vite + Element Plus 控制台，包含节点监控、Agent 配置、运行环境、模型、实例、垃圾箱、日志审计、用户与组页面。
- v0.1 发布为**单包**（`lightai-platform-v0.1.0-linux-x86_64-glibc2.28.tar.gz`），包含 Server + Agent + Web + 脚本 + 配置 + systemd + collector。不是 server/agent 分包。
- Instance 顶层类型是 `external` 或 `local`；`local` 实例的启动方式来自 Runtime 的 `deploy_type`：`binary`、`script` 或 `docker`。
- Docker 代码路径已实现，包括三层参数合并、`docker run --detach`、`docker stop`、`docker inspect`、`docker logs` 和 managed store 恢复；仍需真实 GPU 环境端到端验证。
- 平台日志已实现脱敏、级别过滤、轮转和保留策略。

## Backend 生命周期差异

v0.1 支持多种后端，生命周期语义不同，切勿混淆：

### llama.cpp / vLLM（local binary/script）
- 每个 Instance 对应一个独立进程。
- start → Agent 启动二进制进程；stop → kill 进程。
- Agent 通过 managed_process 记录跟踪进程存活。
- heartbeat reconcile：Agent 未上报 managed process status 时，Server 可能将实例标记 failed。

### vLLM（Docker）
- 每个 Instance 对应一个 Docker 容器。
- start → `docker run --detach`；stop → `docker stop`。
- managed store 记录 container_id/name。

### Ollama
- **完全不同的语义**，务必理解：
  - Runtime = Ollama daemon 配置（host/port/env vars）。
  - Instance = 某个 Ollama model name 的逻辑实例。
  - 多个 Instance 共享同一个 daemon。
  - start = 加载/预热模型（POST /api/generate warmup），**不**启动新进程。
  - stop = 卸载模型（keep_alive=0），**不**停止 daemon。
  - check/test 通过 Ollama API（/api/tags、/api/generate）判断。
  - logs = 共享 Ollama daemon 日志。
  - **不强制绑定 model_file_id**。模型来源为节点本地 Ollama 模型列表（/api/tags），也允许手工输入。
  - **heartbeat reconcile 必须跳过 Ollama Instance**：Ollama 是逻辑实例，不依赖 managed process 心跳。Server 不应因为 managed_instances 中没有该 instance 而标记 failed。
  - **test 不应因 DB status 非 running 被拦截**：Ollama test 由 Agent 调用 API 判断实际可用性。
- 如果已有可用 Ollama daemon，平台复用；如果没有，启动 Instance 时 Agent 尝试启动 ollama serve。
- Runtime 保存只校验格式，不检查 daemon 是否在线。刷新模型/启动实例时才访问 host:port。

### Ollama Runtime 配置字段
- binary_path（默认 ollama）、host（默认 127.0.0.1）、port（默认 11434）
- models_dir（可空）、max_loaded_models（默认 2）、num_parallel（默认 1）
- max_queue（默认 512）、keep_alive（默认 30m）、context_length（默认 4096）
- 保存到 runtime_params_json.defaults

### Ollama 暂缓项
- 不自动 ollama pull；不支持 Modelfile 管理；不支持模型删除
- 不控制 CUDA_VISIBLE_DEVICES / ROCR_VISIBLE_DEVICES
- 不做多 daemon 调度；不做 GPU 自动调度

## 必守开发约束

1. Agent 是唯一节点本地执行者；Server 不直连 Agent，Web 不直连 Agent 或节点服务。
2. 本地执行必须使用 argv，不构造 shell 命令字符串，不接受前端任意命令。
3. Agent 退出不终止模型实例；只有用户显式 stop 才能停止受管进程或容器。
4. **Ollama 例外**：Agent 离线时，Ollama daemon（如果由 Agent 启动）可能退出，但平台不会主动 kill 模型 runner。
5. running / starting / stopping 的 Instance 及其引用的 Runtime、Model 不能修改。
6. 文档和代码都应保持小改动、低抽象、无不必要依赖。
7. 当前不要实现 API Gateway、API Key、配额、计量、调度优先级或计费。
8. 用户组只做成员关系和组角色继承；不要扩展成复杂 IAM。
9. 当前角色只有 `admin`、`operator`、`viewer`；后端统一计算 `effective_role`。这是轻量内置角色，不是完整 RBAC。忘记密码通过 `lightai-server --reset-password <USERNAME> <PASSWORD>` 恢复。后端已实现最后一个 admin 保护。当前无用户删除功能。

## 代码地图

```text
server/src/
  routes.rs              # Axum 路由和 HTTP handler
  models.rs              # API 请求/响应类型
  repository.rs          # 用户、用户组、会话、节点注册、心跳、指标、配置、审计、reconcile
  agent_tasks.rs         # Agent task poll/result/timeout/notify
  db.rs                  # SQLite 连接、SQL 迁移
  domain/
    runtimes.rs          # Runtime CRUD 和 Agent 检查
    instances.rs         # Instance CRUD、start/stop/test/check
    model_catalog.rs     # Model CRUD
    model_files.rs       # Model File CRUD 和验证
    model_trash.rs       # Trash 和受控物理删除
    instance_logs.rs     # Agent/实例日志读取、Ollama 模型列表查询

agent/src/
  main.rs                # Agent HTTP health、heartbeat loop、task loop
  heartbeat.rs           # 注册、心跳、指标/GPU/managed report 上报
  managed_process.rs     # 受管进程/容器记录持久化和恢复
  gpu/                   # 脚本化 GPU collector 调度与 registry/hash 校验
  metrics.rs             # CPU/内存/磁盘采集
  tasks/
    mod.rs               # 任务分发（含 Ollama 专用分支）
    ollama.rs            # Ollama daemon 管理、模型 warmup/unload/check/test、模型列表
    runtime_check.rs     # Runtime 检查
    process*.rs          # 本地程序/脚本启停、日志、命令构造
    docker_backend.rs    # Docker 启停、inspect、logs、参数合并
    verify_model.rs      # 模型路径验证
    cleanup.rs           # 受控文件删除
    logs.rs              # 实例日志读取（含 Docker + Ollama daemon 日志）

web/src/
  api.ts                 # Server API client（含 Ollama 模型查询）
  types.ts               # 前端 API 类型
  components/            # Nodes/Config/Runtime/Models/Instances/Trash/LogsAudit
  components/instances/  # 实例参数（含 Ollama 字段）
  utils/templates.ts     # Runtime/Model 模板和兼容性 helper
```

## 数据库与迁移

- `migrations/0001_init.sql` 是占位。
- `0002_stage2_nodes.sql` — 节点、gpu_status、指标采样表。
- `0003_stage3a_models.sql` — Runtime、Model、Model Instance、Model File、Agent Task、Trash 表。
- `0005_platform.sql` — 用户、session、用户组、审计、配置策略、collector registry 表。
- `server/src/db.rs` 启动时按序执行上述 SQL 文件。不兼容历史数据库，旧数据库删除后重建。

## 已知限制和风险

- Docker/vLLM 未在真实 GPU 环境完成完整验收。
- Ollama 暂不支持自动 pull、GPU 可见性控制、多 daemon 调度。
- 暂无 OpenAI-compatible API Gateway、API Key、额度、计量、GPU 调度。
- 手工 kill local 受管进程后，状态同步到 Web 最坏约 33 秒（Agent monitor 3s + heartbeat 15s + Web refresh 15s）。
- 模型垃圾箱不支持批量清理、定时清理或目录递归删除。
- 前端错误上报是 fire-and-forget，网络失败时静默丢失。
- 审计页面是基础列表和筛选，有默认 limit 500（最大 1000）和 offset 分页。
- 历史指标采样数据会自动清理（默认保留 7 天），但不做小时聚合或降采样。
- `model_file_id` 在数据库中是可 null 字段（Ollama Instance 不需要），但 Rust 层面对非 Ollama 后端仍强制校验。
- 权限体系为 admin/operator/viewer 基础模型，不是完整 RBAC。

## 后续阶段方向

1. 第二阶段：OpenAI-compatible API Gateway、模型路由和调用认证。
2. 第三阶段：API Key、部门/项目归属、额度、限流、调用统计。
3. 第四阶段：GPU 资源调度、优先级、扩缩容和降级。
4. 第五阶段：费用归集、SLA、审计分析、运营报表。

## 后续建议优先级

1. 在真实 NVIDIA GPU 环境验证 collector + Docker vLLM + Ollama 端到端。
2. 缩短受管进程异常退出到 Web 的同步延迟。
3. 在运行层稳定后推进统一模型调用入口。

## 本地部署验证

`scripts/verify-local-deployment.sh` 用于在本机快速验证整个部署链路。

- **工作目录**：`.local/lightai-deployment`（可通过 `LIGHTAI_VERIFY_DIR` 环境变量或 `--workdir` 覆盖）。**不要依赖 `/tmp`**。
- **默认行为**：自动判断 fresh（首次初始化）或 update（停止服务 → 更新二进制 → 重启 → 验证）。
- **fresh 模式**：构建 → 组装 → 初始化 certs → 配置 → collector sync → Agent CA 下载 → 启动 → 全链路验证。
- **update 模式**：**先停止 lightai-server + lightai-agent** → 再覆盖 bin/web/dist → 重新启动 → 验证。update 不重新生成 certs/config/data/db/token。
- **CA 同步**：update 和 fresh 都会检查 agent/certs/ca.crt 与 server/certs/ca.crt 一致性，不一致时自动同步。Agent 启动前有 TLS preflight 检查。
- **`--stop`**：只停止 lightai-server 和 lightai-agent，不构建、不验证、不删除数据。
- **`--stop-after-verify`**：完整验证后停止服务。
- **`--clean`**：停止服务后删除整个部署目录，需要确认或 --yes。
- **`--fresh`**：强制全新初始化。
- **`--no-restart`**：只同步文件，不启动服务。
- **`--auto`**：自动创建 admin 并测试登录。

```bash
# 日常开发：首次 auto-fresh，后续 auto-update
bash scripts/verify-local-deployment.sh

# 强制重建
bash scripts/verify-local-deployment.sh --fresh --yes

# CI / 自动化完整测试
bash scripts/verify-local-deployment.sh --fresh --yes --auto --stop-after-verify

# 只停止服务
bash scripts/verify-local-deployment.sh --stop
```

## Release 打包

v0.1 发布为**单包**，不是 server/agent 分包。

- `lightai-platform-v0.1.0-linux-x86_64-glibc2.28.tar.gz`（glibc2.28，推荐）
- `lightai-platform-v0.1.0-linux-x86_64-native.tar.gz`（native，仅本机测试）

包内包含：bin/lightai-server、bin/lightai-agent、web/dist、scripts/（init-server.sh、init-agent.sh、start-server.sh、start-agent.sh、stop.sh）、config/（server.example.toml、agent.example.toml）、systemd/、collectors/gpu/nvidia-wsl/、INSTALL.md、ldd-check.txt、glibc-symbols.txt。

构建命令：
```bash
bash scripts/package-release-docker.sh v0.1.0   # glibc2.28（推荐）
bash scripts/package-release.sh v0.1.0 native     # 本机构建
```

release 包用户不需要 Node.js、cargo 或从源码构建。Server 内置 Web 静态资源托管，SQLite 本地数据库。但 GPU collector、Ollama、Docker、nvidia-smi 等仍可能依赖外部组件。

## 常用验证

```bash
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd web && npm run build
```

## 安全

- **默认 HTTPS 18443**：HTTP 18080 默认关闭（本地排障可开启）。
- **证书**：`lightai-server cert init`（纯 Rust rcgen）生成自签 CA + Server 证书。ca.crt 可分发 Agent，ca.key/server.key 不可分发。
- **setup token**：`lightai-server cert setup-token` 生成。
- **Agent TLS**：Agent 使用 ca.crt 校验 Server 证书；`lightai-agent ca fetch` 下载 CA 并显示指纹确认。`insecure_skip_tls_verify` 默认 false，**不应作为推荐做法**。
- **CA 同步**：verify-local-deployment.sh 保证 agent/certs/ca.crt 与 server/certs/ca.crt 一致。如果 Agent 无法注册，优先检查 CA 一致性。
- **per-agent token**：Agent 注册后使用 Bearer token；Server 只存 hash/prefix。当前不自动轮换。
- **适用场景**：适合可信内网/客户测试网段；不建议公网暴露 18443。
