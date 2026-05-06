# Architecture

## 整体架构

```
Agent (GPU node) ──主动连接──> Server <── Web (console)
```

- Agent 主动向 Server 注册、心跳、拉取任务。Server 不主动直连 Agent。
- Web 只调用 Server HTTP API。
- 所有本地动作（模型验证、进程启停、文件清理）由 Agent 执行，通过平台定义的任务类型下发。

## 代码结构

```
lightai-platform/
  server/src/
    main.rs            # 启动入口
    lib.rs             # 模块声明
    routes.rs          # HTTP API 路由（~990 行）
    domain.rs          # 轻量 facade（43 行），re-export 业务模块
    domain/
      runtimes.rs        # 运行环境 CRUD + 检查（402 行）
      instances.rs       # 实例 CRUD + start/stop/test/check（682 行）
      model_catalog.rs   # 模型 CRUD（246 行）
      model_files.rs     # 模型文件 CRUD + 验证（426 行）
      model_trash.rs     # 模型文件垃圾箱 + 清理（264 行）
      instance_logs.rs   # 日志读取 + 错误摘要（253 行）
      support.rs         # 共享类型（Stage3Error）、验证函数、常量（238 行）
    agent_tasks.rs     # Agent 任务生命周期（494 行）
    repository.rs      # 数据库访问、节点注册、心跳、reconcile（~1260 行）
    models.rs          # 请求/响应/视图类型
    db.rs              # SQLite 迁移、schema 修正
    auth.rs            # token 生成与验证
    config.rs          # Server 配置加载
    http_check.rs      # HTTP 可达性检查
    platform_log.rs    # 日志写入/读取/脱敏/轮转
    util.rs            # now_unix_secs() 等共享函数

  agent/src/
    main.rs            # 启动入口
    tasks/
      mod.rs              # facade：re-export + run/run_once 调度 + 共享类型与 helper（535 行）
      process.rs          # 实例启停（start/stop）、受管进程监控、日志缓冲
      probe.rs            # 就绪探测配置、测试 URL 构建、失败摘要
      verify_model.rs     # 模型文件验证
      cleanup.rs          # 受控模型文件清理
      logs.rs             # 实例日志读取
      docker_backend.rs  # Docker 容器后端（run/stop/inspect/logs/check）
    heartbeat.rs       # 心跳、指标采集、配置同步
    managed_process.rs # 受管进程持久化记录与恢复
    platform_log.rs    # 日志写入/读取/脱敏/轮转
    client.rs          # Server HTTP 客户端
    models.rs          # Agent 侧请求/响应类型
    state.rs           # Agent 状态文件读写
    config.rs          # Agent 配置加载
    gpu.rs             # GPU 采集（nvidia-smi + custom collector）
    metrics.rs         # CPU/内存/磁盘指标采集

  web/src/
    utils/
      instance.ts         # 共享状态/标签/格式化 helper（61 行）
    main.ts            # Vue 应用入口，全局错误捕获
    api.ts             # Server API 客户端
    components/
      InstancesPanel.vue   # 实例管理（616 行，已提取 utils/instance.ts）
      LogsAuditPanel.vue   # 日志与审计
      NodesPanel.vue       # 节点监控
      ModelsPanel.vue      # 模型管理
      ...
```

## 关键模块职责

### server/src/agent_tasks.rs

Agent 任务生命周期的**唯一实现**。包含：
- `poll_agent_task` — Agent 长轮询获取任务
- `record_agent_task_result` — 记录任务结果并更新关联状态
- `mark_timed_out_tasks` / `mark_task_timed_out` — 超时标记
- `notify_agent_tasks` — 唤醒等待中的 Agent 长连接

### server/src/domain.rs + server/src/domain/

`domain.rs` 已变为 43 行轻量 facade，仅含 `mod` 声明和 `pub use` re-export。业务逻辑已拆入 7 个子模块：

| 模块 | 职责 | 行数 |
|------|------|------|
| `runtimes.rs` | 运行环境 CRUD、Agent 检查 | 402 |
| `instances.rs` | 实例 CRUD、start/stop/test/check、任务创建 | 682 |
| `model_catalog.rs` | 模型 CRUD | 246 |
| `model_files.rs` | 模型文件 CRUD、验证、路径检查 | 426 |
| `model_trash.rs` | 模型文件垃圾箱、清理 | 264 |
| `instance_logs.rs` | 实例日志读取、刷新、错误摘要 | 253 |
| `support.rs` | Stage3Error、验证函数、常量、guard helpers | 238 |

routes.rs 继续通过 `domain::function()` 调用（由 facade re-export 透明转发）。

### server/src/repository.rs

数据库访问层：
- `register_node` — 节点注册（事务 + 身份冲突检查 + 并发重试）
- `reconcile_managed_instances` — 心跳 reconcile 实例状态
- `record_heartbeat` — 心跳写入
- `effective_agent_config` — 配置策略合成
- `list_nodes` / `list_audit_events` — 查询

### server/src/routes.rs

Axum HTTP 路由定义和请求处理器。所有 handler 委托给 `domain::` 或 `repository::` 的具体函数。

### agent/src/tasks/mod.rs + agent/src/tasks/ 子模块

Agent 侧任务执行。包含：
- `start_model_instance_with_store` — 启动本地实例
- `stop_model_instance_with_store` — 停止本地实例
- `test_model_instance` — 测试实例
- `verify_model_file` — 模型文件验证
- `cleanup_model_file` — 受控文件清理
- `read_instance_log` — 读取实例日志

### web/src/components/InstancesPanel.vue

实例管理 UI（616 行）。包含：
- 实例列表、创建/编辑表单
- start / stop / test / check 操作 + 自动刷新
- Agent 离线自动检测：周期刷新时基于 `node_online` / `last_heartbeat_at` 展示 warning 标签
- 过渡态轮询（pollInstanceUntilStable）
- 周期刷新（15s）
- 探测配置面板
- 日志查看对话框 + 刷新按钮

辅助模块 `web/src/utils/instance.ts`（61 行）：statusType / statusLabel / instanceStatusLabel / isAgentOffline / formatTime 等。

### server/src/models.rs — ModelInstanceView

`ModelInstanceView` 包含 `node_online: bool` 和 `last_heartbeat_at: Option<i64>` 字段，从实例节点的心跳时间推算。Agent 离线时 `node_online=false`，但实例状态保持原值（不误改为 failed）。

## 数据流

### Agent 注册与心跳

```
Agent 启动 → POST /api/agent/register → Server 返回 node_id + token
Agent loop → POST /api/agent/heartbeat → Server reconcile 实例状态
```

### 本地实例启动

```
Web 点击启动 → POST /api/model-instances/{id}/start
  → domain::start_model_instance → 创建 agent_task
  → agent_tasks::notify_agent_tasks
  → Agent poll 获取任务 → tasks::start_model_instance_with_store
  → 端口检查 → spawn 进程 → 就绪探测 → 持久化 managed store
  → 上报结果 → agent_tasks::record_agent_task_result → 更新实例状态
```

### 状态恢复

```
Agent 重启 → managed_process::load → 逐条 /proc/{pid}/stat 校验
  → reports() → heartbeat managed_instances
  → Server reconcile_managed_instances → running 实例保持 running
  → 已退出实例标记为 failed + 原因
```

### Agent 离线检测（Web 自动感知）

```
Server list_model_instances → 查询 n.last_heartbeat_at
  → 计算 node_online（now - last_heartbeat_at <= 60s）
  → 返回 ModelInstanceView { node_online, last_heartbeat_at, status, last_error }
Web 周期刷新（15s）→ 检查 node_online
  → 离线 + running → warning 标签 "Agent 离线，运行状态无法确认"
  → 在线 + running → success 标签 "运行中"
  → 不误改 instance status 为 failed
```

### 进程隔离（Agent 退出不终止实例）

```
Agent 启动实例 → std::process::Command
  → stdin(Stdio::null())       # 脱离 Agent 控制终端
  → stdout/stderr → piped      # 写入受控日志文件
  → Unix: process_group(0)     # 独立进程组，不接收 Agent 进程组信号
  → spawn                      # 子进程独立于 Agent 存活
Agent 退出 → main.rs 日志"正在退出，不会终止受管实例"
  → managed store 保留 N 条记录
  → 不遍历、不 kill 受管进程
Agent 重启 → managed_process::load → reports()
  → 存活实例上报 running，已退出上报 failed
```

> **systemd 部署**：必须设置 `KillMode=process`（非默认的 `control-group`），否则 systemd 停止 Agent service 时会向整个 cgroup 发送 SIGTERM，导致模型实例进程也被终止。示例 service 文件见 `deploy/lightai-agent.service`。

### Docker 容器启动（三层配置合并）

```
Web 点击启动 → POST /api/model-instances/{id}/start
  → domain::start_model_instance → 创建 agent_task（payload 含 runtime_params + params + model_path + model_name）
  → Agent poll 获取任务 → tasks::start_model_instance_with_store
  → deploy_type == "docker" → resolve_docker_payload
    → 解析 runtime_params → DockerRuntimeConfig（image, gpu="all", ipc="host", container_port, defaults）
    → 解析 params → DockerInstanceOverrides（container_name, host_port, gpu?, ...）
    → merge_docker_config(model_path, model_name, runtime, overrides)
      → docker.gpu = instance.gpu ?? runtime.gpu
      → docker.image = runtime.image
      → port mapping = instance.host_port : runtime.container_port
      → vllm.port = runtime.defaults.port (= container_port)
    → build_docker_run_args → ["run", "--name", ..., "--gpus", ..., "--ipc", ..., "-p", ..., "-v", ..., "--detach", image]
    → build_vllm_args → ["--model", ..., "--served-model-name", ..., "--host", "0.0.0.0", "--port", ...]
    → agent.log 记录完整 command_summary（image, gpu, ipc, port, volumes）
    → docker run (argv-style, no shell) → container_id
    → ManagedProcessRecord { container_id, container_name, deploy_type: "docker", command }
    → upsert managed store
  → 上报结果 → running + base_url + command_summary
```

GPU 默认 "all"，ipc 默认 "host"（来自 `DockerRuntimeConfig` 自定义 Default）。Host 固定 "0.0.0.0"，不在 UI 配置。

### Docker 实例恢复

```
Agent 重启 → managed_process::load → 逐条 check_record
  → deploy_type == "docker" → docker_backend::check_docker_record
    → docker inspect <container> → parse State.Running/ExitCode/Error
    → running → 保持 running，清空 last_error
    → exited/not found → 上报 failed + 退出原因
  → deploy_type == "local" | None → 现有 pid + start_time 校验
```

> **Docker 部署验证**：本机使用 `vllm/vllm-openai:latest` 镜像 + `/data/models/qwen3-0.6b` 模型目录。Docker 容器默认不加 `--rm`，便于 Agent 在容器异常退出后仍能通过 `docker inspect` 获取 OOM、退出码等诊断信息。用户显式 stop instance 时才 `docker stop`；后续清理资源时才 `docker rm`。
>
> **Docker 日志审计**：Docker start/stop/inspect 操作均通过 `platform_log::append` 写入 agent.log（ISO 8601 时间戳、脱敏），记录 container_name, image, gpu, ipc, 端口映射, volumes, command_summary。失败时额外记录 stderr 摘要。Web 日志页面可见 command_summary。
