# Implementation Notes

本文档记录较细的实现事实、数据流和注意事项。核心架构边界以 [ARCHITECTURE.md](ARCHITECTURE.md) 为准；本文随代码演进更新。

当前实现属于第一阶段：GPU 服务器统一纳管、基础模型实例管理、Web 控制台、本地用户与用户组。统一模型调用 API、API Key、额度、计量、调用统计、GPU 调度优先级、扩缩容、降级和费用归集是后续阶段目标，本文件只记录当前已落地的实现事实。

## API 路径

### 基础与 Agent

- `GET /health`：Server 健康检查。
- `POST /api/agent/register`：Agent 注册，返回 `node_id`、`agent_token` 和有效配置。
- `POST /api/agent/heartbeat`：Agent 心跳，Bearer token 鉴权。
- `POST /api/agent/tasks/poll`：Agent 轮询任务。
- `POST /api/agent/tasks/{id}/result`：Agent 上报任务结果。

除 `/health`、`/api/setup/*`、`/api/auth/login` 与 `/api/agent/*` 外，所有 `/api/*` 控制面接口都需要本地用户登录会话。Web 使用 HttpOnly `lightai_session` cookie，不在前端构建产物中预置管理密钥。空库首次管理员只能通过 Web setup 创建，生产配置不支持 `initial_admin_password` 或 `LIGHTAI_ADMIN_PASSWORD`。

### 用户与会话

- `POST /api/auth/login`：本地用户名/密码登录，成功后设置 HttpOnly session cookie。
- `POST /api/auth/logout`：撤销当前 session。
- `GET /api/auth/me`：读取当前登录用户。
- `POST /api/auth/change-password`：当前用户修改自己的密码。管理员重置密码后，用户登录只能访问 `me`、`change-password`、`logout` 等必要接口，修改成功后旧 session 会失效并需要重新登录。
- `GET /api/setup/status`：空库初始化状态。
- `POST /api/setup/admin`：仅无用户时创建第一个管理员，成功后关闭 setup 入口并登录。
- `GET|POST /api/users`：管理员列出/创建本地用户。
- `PUT /api/users/{id}`：管理员修改用户角色、启用状态或密码。
- `GET|POST /api/groups`：管理员列出/创建用户组。
- `PUT|DELETE /api/groups/{id}`：管理员修改用户组或删除空组。
- `PUT /api/groups/{id}/members`：管理员替换用户组成员列表。

密码使用 Argon2 哈希存储；数据库中不保存明文密码或 session token，只保存 hash。当前角色保持极简，仅 `admin`、`operator` 与 `viewer`。

用户可以有直接角色，也可以通过启用状态的用户组继承角色。后端在登录、会话认证、用户列表和组成员展示时统一计算最高权限 `effective_role`；控制面权限判断使用 `effective_role`，不能只依赖前端隐藏按钮。`admin` 可管理用户、用户组、系统配置和 Trash/清理类危险操作；`operator` 可执行 Runtime、模型、模型文件、实例等日常运维写操作；`viewer` 只能执行 GET 查看。禁用用户组后，该组不再提供角色继承。删除用户组要求先移除所有成员。

密码策略可通过 `[auth.password]` 配置：`min_length` 默认 12，`complexity_required` 默认 false，`expires_days = 0` 表示关闭过期，`force_change_after_reset` 默认 true。session 策略可通过 `[auth.session]` 配置：`ttl_secs` 默认 43200，`idle_timeout_secs` 默认 7200，`secure_cookie` 默认 false。生产 HTTPS 部署应开启 `secure_cookie`，并推荐 Web 与 API 同源或通过同一反向代理访问。

管理员忘记密码时，不提供公开远程恢复 API；在服务器本机执行 `lightai-server --reset-password <USERNAME> <PASSWORD>`。重置密码会撤销该用户现有 session，并按策略标记用户必须修改密码。


用户组当前只承载成员关系和组角色，是后续部门、项目、业务系统、API Key、额度、计量和优先级归属的基础对象；当前不做资源级授权、多租户隔离、组管理员、审批流、菜单权限或复杂权限表达式。

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

- 创建 SQLite parent directory，开启 WAL，并开启 SQLite foreign key 约束。
- 执行 `0001_init.sql`、`0002_stage2_nodes.sql`、`0003_stage3a_models.sql`。
- 创建 `nodes.name` 和 `nodes.hostname` 唯一索引。
- 用幂等代码补齐 node status 配置字段、runtime endpoint 字段、model file `deleted_at/path_type`、instance process/log/command 字段、trash 字段等。
- 创建 `agent_config_policies`、`audit_events`、`users`、`user_sessions`、`user_groups`、`user_group_members`、`platform_settings`。

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
- 审计记录覆盖控制面所有写操作：用户管理、用户组管理、登录/退出、密码修改、配置更新、Runtime CRUD/check、Model/Model File CRUD/verify、Instance CRUD/start/stop/test/check/logs-refresh、Trash create/cleanup/delete、collector registry register/update。操作失败也会记录（带 error_message）。
- 审计查询 `GET /api/audit-events` 支持可选 `limit`（默认 500，最大 1000）和 `offset`（默认 0）参数。

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

## GPU Collector 架构（脚本化）

### 概述

Agent 只支持脚本化 GPU/加速卡 collector。未配置 `[gpu_collectors]` 时，Agent 不执行 GPU collector 脚本，也不会回退到旧的内置 `nvidia-smi` 或 custom JSON collector。

Agent 启动时会输出 GPU collector 配置诊断日志（`agent.log`），明确显示配置状态、发现的 collector 目录和 hash。首次 heartbeat 周期会输出 GPU probe 日志，区分四种状态：

| 状态 | 含义 | agent.log 级别 |
|------|------|---------------|
| `no_collector_configured` | 未配置 collector_root | WARN |
| `collector_configured_but_failed` | collector 配置了但全部失败 | ERROR（含错误摘要） |
| `collector_ok_no_devices` | collector 执行成功但未发现 GPU | WARN |
| `collector_ok_devices_found` | collector 执行成功并发现 GPU | INFO（含每张卡摘要） |

`collector_status` 和 `collector_errors` 通过 heartbeat 上报到 Server，存储在 `node_status` 表，并由 `GET /api/nodes` 返回，Web 节点页 GPU 列表区域会据此展示原因。

| 路径 | 触发条件 | 说明 |
|------|----------|------|
| **Collector 框架** | `collector_root` 已配置 | 脚本化 collector 目录，需要 Server registry 登记 |

### Agent 配置

Agent 本地 TOML 配置文件支持 `[gpu_collectors]` 节：

```toml
[gpu_collectors]
root = "/opt/lightai/collectors/gpu"
mode = "explicit"         # "explicit" | "auto"
enabled = ["nvidia"]
disabled = []
```

配置加载优先级：
1. `--config <PATH>` 命令行参数
2. `LIGHTAI_AGENT_CONFIG` 环境变量
3. 程序目录 `agent.toml`
4. 程序目录 `lightai-agent.toml`
5. 内嵌默认值

生成配置模板：
```bash
lightai-agent config init ./agent.toml
```

### 启用流程

1. 将 collector 目录放到 Agent 本地，例如 `/opt/lightai/collectors/gpu/nvidia-wsl`
2. 运行 `lightai-agent collector inspect /opt/lightai/collectors/gpu/nvidia-wsl`
3. 将输出的 JSON 粘贴到 Web「采集器登记」页面
4. 在 Agent 配置中设置 `[gpu_collectors]` root + enabled
5. 启动 Agent

### collector 目录结构

```
/opt/lightai/collectors/gpu/
  nvidia/                  # 示例：deploy/collectors/gpu/nvidia-wsl/
    collector.toml
    discover.sh
    metrics.sh
```

每套 collector 一个独立目录，固定包含三个文件：
- `collector.toml` — id, vendor, name, version, discover, metrics
- `discover.sh` — 设备发现（输出 STATUS + DEVICE TSV 行）
- `metrics.sh` — 指标采集（输出 STATUS + METRIC TSV 行）

### 安全机制（fail-closed）

Collector 脚本执行前必须通过所有校验：

1. 本地 collector 目录存在且包含三个必需文件
2. Agent 配置启用（enabled 列表 / disabled 不排除）
3. Server registry 中存在匹配的 `(id, version)` 且 `enabled=true`
4. `discover_sha256` 与 registry 一致
5. `metrics_sha256` 与 registry 一致

任一条件不满足 → 脚本不执行，错误原因写入 `collector_errors`：
- `collector not registered`
- `collector disabled in Server registry`
- `discover.sh hash mismatch`
- `metrics.sh hash mismatch`
- `collector registry is empty`

registry 为空时所有 collector 不执行。

### inspect 命令

```bash
lightai-agent collector inspect /opt/lightai/collectors/gpu/nvidia-wsl
```

- 读取 `collector.toml`
- 计算 `discover.sh` / `metrics.sh` 的 SHA-256（进程内，不依赖系统 sha256sum）
- 输出 JSON 供 Web 登记
- **不登记、不批准、不执行脚本**

### Server registry

Agent 通过 heartbeat 从 Server 拉取 `collector_registry` 列表（`id + version` 唯一键）。
Web「采集器登记」页面支持查看已登记采集器；登记新采集器和启用/禁用切换限制为 **admin only**。

### 当前 NVIDIA 实现

**示例目录**：`deploy/collectors/gpu/nvidia-wsl/`

- `collector.toml`：id="nvidia", vendor="nvidia", version="1.0.0"
- `discover.sh`：通过 `nvidia-smi --query-gpu=index,name,uuid,pci.bus_id,driver_version` 输出 DEVICE TSV 行
- `metrics.sh`：通过 `nvidia-smi --query-gpu=uuid,memory.total,...` 输出 METRIC TSV 行

### 后续支持国产卡

**不写 Rust Collector。** 新增厂商只需新增 collector 目录：

1. 创建 `deploy/collectors/gpu/<vendor>/` 目录
2. 编写 `collector.toml` + `discover.sh` + `metrics.sh`（TSV 输出）
3. `lightai-agent collector inspect` → Web 登记 → 配置 enabled
4. **不需要修改 Rust 代码，不需要重新编译 Agent**

| 厂商 | 预期命令 | 状态 |
|------|----------|------|
| NVIDIA | `nvidia-smi` | 已实现（脚本 collector） |
| 沐曦 | `mx-smi` | 未实现，需真实硬件 |
| 昇腾 | `npu-smi` | 未实现，需真实硬件 |
| 海光 | `hy-smi` | 未实现，需真实硬件 |

所有未验证的 collector 不得默认启用，不得在文档中宣称已支持。未接入真实硬件前，可以新增脚本 collector 目录并通过 registry/hash 流程验证。

### GPU key 约定

- 优先使用设备稳定唯一标识（NVIDIA: UUID）
- 格式：`<vendor>:<stable_id>`
- 不长期依赖 index（重启可能漂移）

## systemd 注意事项

Agent systemd 示例应使用 `KillMode=process`。如果使用默认 `control-group`，停止 Agent service 可能向整个 cgroup 发送信号，破坏"Agent 退出不终止模型实例"的约束。

## 后续开发注意

- 新增节点本地动作时，先定义明确任务类型和 payload/result schema，再实现 Server 创建任务和 Agent 执行分支。
- 不要把用户输入拼成 shell 命令。
- 不要让 Server 直接删除节点文件。
- 不要在 Agent 离线时把运行中实例自动改为 failed。
- 修改 schema 前先处理 migration ledger 缺失问题，至少保持启动时幂等。
- 新增 GPU collector 时，创建 collector 目录 + 脚本（collector.toml + discover.sh + metrics.sh），通过 Web 登记 registry。**不要写 Rust Collector**。
- 未经验证的厂商 collector 不默认启用，不在文档中宣称已支持。
