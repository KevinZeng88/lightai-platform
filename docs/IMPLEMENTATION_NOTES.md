# Implementation Notes

本文档记录较细的实现事实、数据流和注意事项。核心架构边界以 [ARCHITECTURE.md](ARCHITECTURE.md) 为准；本文随代码演进更新。

## API 路径

### 基础与 Agent

- `GET /health`：Server 健康检查。
- `POST /api/agent/register`：Agent 注册，返回 `node_id`、`agent_token` 和有效配置。
- `POST /api/agent/heartbeat`：Agent 心跳，Bearer token 鉴权。
- `POST /api/agent/tasks/poll`：Agent 轮询任务。
- `POST /api/agent/tasks/{id}/result`：Agent 上报任务结果。

### 节点、指标、配置

- `GET /api/nodes`
- `GET /api/nodes/{node_id}/metrics`
- `GET /api/nodes/{node_id}/gpus/{gpu_key}/metrics`
- `GET /api/nodes/{node_id}/gpu-metrics?gpu_key=...`
- `GET /api/config/agent`
- `GET|PUT /api/config/agent/global`
- `GET|PUT /api/nodes/{node_id}/config`

### Runtime、Model、Instance

- `GET /api/runtime-environments`
- `GET|POST /api/nodes/{node_id}/runtime-environments`
- `GET|PUT|DELETE /api/runtime-environments/{id}`
- `POST /api/runtime-environments/{id}/check`
- `GET|POST /api/models`
- `GET|PUT|DELETE /api/models/{id}`
- `GET|POST /api/models/{id}/files`
- `GET|PUT|DELETE /api/model-files/{id}`
- `POST /api/model-files/{id}/verify`
- `POST /api/model-files/{id}/trash`
- `GET|POST /api/model-instances`
- `GET|PUT|DELETE /api/model-instances/{id}`
- `POST /api/model-instances/{id}/check`
- `POST /api/model-instances/{id}/start`
- `POST /api/model-instances/{id}/stop`
- `POST /api/model-instances/{id}/test`
- `POST /api/model-instances/{id}/logs`

### Trash、日志、审计

- `GET /api/model-file-trash`
- `POST /api/model-file-trash/{id}/cleanup`
- `DELETE /api/model-file-trash/{id}`
- `GET /api/logs`
- `POST /api/frontend-errors`
- `GET /api/audit-events`
- `GET|PUT /api/config/server-logs`

## Agent 任务类型

Agent 在 `agent/src/tasks/mod.rs` 中分发以下任务：

- `verify_model_file`
- `cleanup_model_file`
- `read_agent_log`
- `check_runtime_environment`
- `start_model_instance`
- `stop_model_instance`
- `test_model_instance`
- `read_instance_log`

未知任务会以 failed 上报，不会执行任意命令。

## 数据库状态

启动时 `server/src/db.rs` 会：

- 创建 SQLite parent directory，开启 WAL。
- 执行 `0001_init.sql`、`0002_stage2_nodes.sql`、`0003_stage3a_models.sql`。
- 创建 `nodes.name` 和 `nodes.hostname` 唯一索引。
- 用幂等代码补齐 node status 配置字段、runtime endpoint 字段、model file `deleted_at/path_type`、instance process/log/command 字段、trash 字段等。
- 创建 `agent_config_policies`、`audit_events`、`platform_settings`。

注意：

- `migrations/0004_stage3a_corrections.sql` 是历史参考，不会自动执行。
- 当前没有 migration ledger；新增表结构变更时不要只追加 SQL 文件并假设会运行。

## 数据流

### 注册和心跳

```text
Agent state 不存在
  -> register(name, hostname, version, os, arch)
  -> Server 事务检查 name/hostname 唯一性
  -> 返回 token，Agent 写入 state file

Agent heartbeat
  -> 上报 NodeMetrics、GpuMetrics、collector_errors、agent_config、managed_instances
  -> Server 写 node_status/gpu_status/metric samples
  -> Server reconcile managed instance reports
  -> 返回 effective_agent_config
```

Server 判定在线使用 `ONLINE_THRESHOLD_SECS = 60`。

### Runtime 保存和检查

创建 Runtime 时 Server 会先检查节点在线并创建 `check_runtime_environment` 任务：

- `binary` / `script`：Agent 校验 `binary_path` 存在、不是目录、可执行，并尝试获取版本。
- `docker`：Agent 检查 `docker_image`，通过 Docker CLI inspect/version 路径确认镜像可用性。
- `available` 和 `version_unavailable` 都被 Server 视为可用。

### Model 和 Model File

- 新增 Model 必须带 `initial_file`，Server 会让对应节点 Agent 验证路径后才写入 Model 和 Model File。
- 新增/编辑 Model File 同样同步等待 Agent 验证。
- `POST /api/model-files/{id}/verify` 创建异步验证任务，Model File 状态变为 `verify_pending`。
- 验证只确认路径存在、基础类型和大小；不验证模型格式或推理可用性。
- **Model 元数据字段约定**：API/Wire 字段统一为 `params_json`（与 Instance 一致），DB `models` 表列保持 `config_json`，Server 在 `model_from_row` 和 `create/update` 中做透明映射。`ModelRequest`/`ModelView` 同时保留 `config_json` 字段仅用于旧调用方兼容，新接口不应依赖 `config_json`。

### Instance 生命周期

`external`：

- 创建/编辑时要求 `base_url`。
- `check` 使用 `health_url`、`endpoint_url` 或 `base_url` 做 HTTP 可达性检查。
- Server 不启动、不停止外部服务。

`local`：

- 创建时要求 Node、Runtime、verified Model File。
- `start` / `stop` / `test` / running 状态下 `check` 都创建 Agent 任务并等待结果。
- Server 在任务执行前设置 `starting`、`stopping` 或保留 `running` 状态。
- 任务超时后标记 task 为 `timed_out`，实例更新为 failed 或返回明确错误。

## 本地程序和脚本

local Runtime 的 `deploy_type` 可以是：

- `binary`：按 backend 构造启动参数，启动长进程。
- `script`：受控脚本入口，custom 脚本按 `start` / `stop` / `test` action 执行。
- `docker`：走 Docker 后端。

本地长进程启动逻辑：

- 使用 `std::process::Command` argv 执行。
- `stdin = null`，stdout/stderr 进入内存 buffer，并可写入受控 instance log 文件。
- Unix 下设置独立 process group。
- 启动前检查端口占用，启动后按 ProbeConfig 探测服务可用。
- 写入 managed store 后才认为受管成功。

## Docker 参数合并

Docker 运行参数来自三层：

```text
Model File path + model name
  + Runtime params_json
  + Instance params_json overrides
  -> DockerPayload { docker, vllm }
```

Runtime `params_json` 主要字段：

- `image`
- `gpu`，默认 `all`
- `ipc`，默认 `host`
- `container_port`，默认 `8000`
- `cache_host_path` / `cache_container_path`
- `defaults.host`，默认 `0.0.0.0`
- `defaults.port`
- `defaults.gpu_memory_utilization`
- `defaults.max_model_len`
- `defaults.max_num_seqs`
- `extra_docker_args`
- `extra_backend_args`

Instance `params_json` 主要字段：

- `container_name`
- `host_port`
- `model_container_path`
- `served_model_name`
- `gpu`
- `gpu_memory_utilization`
- `max_model_len`
- `max_num_seqs`
- `extra_docker_args`
- `extra_backend_args`

合并规则：

- Instance 覆盖 Runtime 默认。
- `host_port` 属于 Instance，`container_port` 属于 Runtime。
- 模型路径以只读 volume 挂入容器。
- `docker run` 不默认加 `--rm`。
- Docker 和 vLLM 参数均通过 argv 传递。

## 受管实例恢复

managed store 路径由 Agent state path 派生，形如 `agent-state.toml.managed-instances.json`。

记录字段包括：

- `instance_id`
- `process_id` / `process_start_time`
- `container_id` / `container_name`
- `deploy_type`
- `base_url` / `endpoint_url`
- `command`
- `log_path`

恢复规则：

- local：校验 `/proc/{pid}/stat` start_time。
- docker：`docker inspect` 判定 running/exited/not found。
- Agent 心跳携带恢复报告，Server reconcile 到 `running` 或 `failed`。
- 外部手工启动、未写入 managed store 的进程不会自动纳管。

## 日志与审计

- Server 和 Agent 日志都使用平台日志模块，支持脱敏、轮转、保留策略。
- Server 日志策略存在 `platform_settings`，Web 日志审计页可修改。
- Agent 日志策略通过 `AgentConfigPolicy` 下发。
- `/api/logs` 支持 `server`、`agent`、`instance`、`frontend`、`errors`。
- Agent 日志和实例日志读取也通过 Agent 任务，不读取任意远端路径。
- 审计记录覆盖配置、Runtime、Model、Model File、Instance、Trash 等主要操作的成功事件。

## Web 交互事实

- Nodes 页面展示节点当前状态、有效 Agent 配置、CPU/内存/磁盘/GPU 趋势。
- Config 页面维护全局和节点级 Agent 配置策略。
- Runtime 页面创建和检查 Runtime；编辑被运行中实例引用的 Runtime 会被拦截。
- Models 页面维护 Model 和节点文件路径；删除 Model/Model File 会进入 Trash，不直接删真实文件。
- Instances 页面维护 external/local 实例，支持 Docker Runtime 的结构化覆盖参数。
- Instances 页面每 15 秒刷新，过渡态操作后额外轮询稳定状态。
- LogsAudit 页面读取平台日志、实例日志、前端错误摘要和审计事件。

当前前后端差异：

- Runtime 的 `params_json` 在 Server 内部存入 `runtime_environments.config_json`，返回时映射为 `params_json`；这是当前兼容实现。
- Web Runtime 下拉只筛选 `check_status === "available"`，Server 对 `version_unavailable` 也认为可用。

## systemd 注意事项

Agent systemd 示例应使用 `KillMode=process`。如果使用默认 `control-group`，停止 Agent service 可能向整个 cgroup 发送信号，破坏“Agent 退出不终止模型实例”的约束。

## 后续开发注意

- 新增节点本地动作时，先定义明确任务类型和 payload/result schema，再实现 Server 创建任务和 Agent 执行分支。
- 不要把用户输入拼成 shell 命令。
- 不要让 Server 直接删除节点文件。
- 不要在 Agent 离线时把运行中实例自动改为 failed。
- 修改 schema 前先处理 migration ledger 缺失问题，至少保持启动时幂等。
