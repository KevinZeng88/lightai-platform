# Architecture

## 总览

```text
Agent (GPU node) ──register / heartbeat / poll tasks──> Server <── Web
```

LightAI Platform 的定位是轻量企业级 GPU / 模型服务管理平台：统一纳管多台 GPU 服务器，让模型实例能够上线、下线和被控制台管理；后续在此基础上补充独立 Gateway 数据面、API Key、Usage、Quota、Cost、报表和治理能力。

当前代码处于第一阶段，重点是 GPU 服务器统一纳管与基础模型实例管理：

- **Server** 是管理 API 入口和控制面状态持久化位置，不默认承载业务模型流量。
- **Agent** 是节点执行面 / 本地 supervisor，负责本地事实采集和本地动作执行。
- **Web** 是 Server API 的前端控制台，不直接访问 Agent 或节点本地服务。
- **SQLite** 保存节点、指标采样、运行环境、模型、模型文件、实例、Agent 任务、配置策略、日志策略、审计记录、用户、用户组和会话。

## 阶段规划

1. 第一阶段：GPU 服务器统一纳管、Agent 心跳/GPU 状态上报、基础模型/Runtime/实例管理、Web 控制台、本地用户与用户组。
2. 第二阶段：模型服务管理与独立 Gateway 数据面，包括 OpenAI-compatible API、模型路由、统一调用认证和基础调用状态。
3. 第三阶段：API Key、部门/项目/业务系统归属、额度、限流、调用统计和基础计量。
4. 第四阶段：GPU 资源调度、关键模型优先级、扩缩容、降级策略和资源紧张时的保障策略。
5. 第五阶段：费用归集、SLA、审计分析、运营报表和企业级治理能力。

后续阶段是产品方向，不表示当前阶段已经实现。当前实现应为这些方向保留清晰归属基础，但不提前实现复杂调度、计费、网关或多租户隔离。

## 通信边界

- Agent 主动注册、心跳、轮询任务和上报任务结果；Server 不主动直连 Agent。
- 所有节点本地动作都通过 `agent_tasks` 表和任务轮询执行，包括模型文件验证、Runtime 检查、实例启停/测试/日志读取、文件清理。
- Web 只调用 Server 管理 API；节点离线时 Web 展示状态不可确认，而不是绕过 Server 直连节点。
- 后续业务模型流量路径是“应用系统 -> Gateway -> 模型实例”，不是“应用系统 -> Server -> 模型实例”或“应用系统 -> Agent -> 模型实例”。

## 安全边界

- 除 `/health`、`/api/setup/*`、`/api/auth/login` 与 Agent 专用 `/api/agent/*` 外，控制面 API 必须携带已登录用户会话 cookie。
- 空数据库首次访问 Web 时进入 setup 页面创建第一个管理员；setup 入口由后端保证只能成功一次。
- 不支持通过生产配置文件或 `LIGHTAI_ADMIN_PASSWORD` 初始化管理员密码。
- 服务器本机可通过 `lightai-server --reset-password <USERNAME> <PASSWORD>` 重置管理员或其他本地用户密码，重置后该用户旧 session 会失效，并要求用户登录后修改密码。
- 当前角色为 `admin`、`operator`、`viewer`。用户可以拥有直接角色，也可以通过启用状态的用户组继承角色；后端统一计算最高权限 `effective_role`，权限判断不能只依赖前端隐藏按钮。`admin` 负责系统管理、配置、collector registry 写操作和危险清理；`operator` 负责日常模型/Runtime/实例运维；`viewer` 只读查看。Web 前端会根据 effective_role 隐藏不具备权限的按钮。
- 用户组当前只承载成员关系和组角色，是后续部门、项目、业务系统、API Key、额度、计量和优先级归属的基础对象。
- Cookie 使用 HttpOnly、SameSite=Lax，可通过配置在 HTTPS 部署时启用 Secure；推荐 Web 与 API 同源部署或通过同一反向代理访问。
- Agent 只执行平台定义的任务类型，不接受任意 shell 命令。
- 本地程序、兼容保留的脚本后端、Docker 均使用 argv 方式执行，不拼接 shell 命令字符串。
- GPU collector 只走本地 `[gpu_collectors]` 目录 + Server registry/hash 校验机制；未登记或 hash 不匹配的脚本不会执行。
- 路径需要校验；模型文件物理删除只能由 Agent 在 Server 下发的 allowed dirs 内执行。
- 日志写入和读取做敏感信息脱敏，不允许前端指定任意日志文件路径。
- Agent 是管理进程，不是模型进程宿主；Agent 退出不主动 kill 受管实例。

## 产品模型

```text
Model + Runtime Environment + Node + Instance Overrides = Model Instance
```

| 概念 | 当前实现 |
|------|----------|
| Model | 模型定义，含名称、类型、默认后端、描述和配置 JSON |
| Model File | 某节点上的模型文件或目录路径，需由 Agent 验证 |
| Runtime Environment | 某节点上的运行模板，含 backend 与 `deploy_type`（v0.1 用户可见主流程为 `binary` / `docker`；`script` 仅保留兼容） |
| Node | Agent 注册后的节点身份和心跳状态 |
| Model Instance | `external` 外部服务，或 `local` 受 Agent 管理实例 |
| User Group | 当前阶段的最小组织归属对象，后续可承接部门、项目、业务系统和 API Key 归属 |

关键边界：

- `external` 实例只记录 HTTP 地址并做可达性检查，不由平台启动/停止。
- `local` 实例绑定 Node、Runtime Environment 和 verified Model File。
- Runtime 是默认模板；Instance 只保存本次覆盖参数，不修改 Runtime。
- Docker 不是实例顶层类型，而是 Runtime 的一种 `deploy_type`。

## 下一阶段服务化能力边界

当前 `Model Instance` 是节点上的运行对象：它描述某个模型文件、某个 Runtime 和某个节点上的一次本地进程、容器或 Ollama 逻辑实例。它不是对业务系统暴露的服务对象，也不承载 API Key、调用配额、费用归属或业务路由语义。

后续模型服务化应在当前模型之上增加清晰的业务访问层，概念边界如下：

| 概念 | 边界 |
|------|------|
| Model | 模型定义和元数据 |
| Runtime | 具体模型运行方式，例如 vLLM、llama.cpp、Ollama，以及某节点上的默认运行参数 |
| Instance | 节点上的实际运行对象，负责承载推理进程、容器或 Ollama 逻辑模型 |
| Service | 未来对外业务访问对象，可关联一个或多个 Instance；API Key、Quota、Usage 和路由策略应围绕 Service 建立 |
| Gateway | 未来独立数据面组件，宜作为独立进程或独立二进制运行，负责业务 API、API Key 本地校验、限流、服务路由、流式转发和请求级 Usage 采集 |
| API Key | 未来业务调用认证凭据，独立于 Web 登录 session，可与用户组、项目或业务系统归属关联 |
| Usage | 未来业务请求级调用记录，包括 API Key、Service、模型、状态、耗时、输入/输出 token 和错误摘要等必要统计 |

未来业务模型调用应经过 LightAI 管控的数据面入口，但该入口不应默认由 Server 承载。Server 应保持控制面定位，负责用户、权限、Service 定义、API Key、Quota/Cost 策略、路由策略、Usage 汇总、审计、配置管理和状态管理；Gateway 负责数据面处理和异步上报。

Agent 可以在节点侧管理 Gateway 的本地生命周期，包括部署、启动、停止、重启、健康检查、日志读取、配置落地和状态上报；Agent 不承担 API Key 策略、计费、租户判断和业务路由决策。Gateway 可以由 Agent 托管生命周期，但不应做成 Agent 本体的业务模块。Web 仍只调用 Server 管理 API，不直连 Agent 或节点本地模型服务。

后续路径应保持分离：

```text
业务流量：应用系统 -> Gateway -> 模型实例
管理路径：Web -> Server -> agent_tasks -> Agent -> Runtime / Instance / Gateway
```

Usage 与节点/GPU 指标不是一类数据。Usage 是业务请求级调用记录；节点/GPU 指标是资源监控数据，包括 GPU、CPU、内存、磁盘和实例状态等。Usage 采集不应阻塞主请求链路，不应默认记录完整 prompt / response，也不应同步记录每个 token 到数据库；后续可考虑异步上报、批量写入、聚合统计和缓存策略。

Usage、Quota 和 Cost 应按依赖顺序分阶段建设：先围绕 Service 和 API Key 建立 Gateway 请求级 Usage 采集与 Server 汇总，再基于 Usage 聚合做配额、限流和治理策略，最后再做成本归集、报表和审计分析。Cost 不应先于 Usage 建模，也不应在早期扩展成完整商业账单、支付、套餐或复杂多租户系统。

这部分属于后续阶段方向，不表示当前 v0.1 已经实现 OpenAI-compatible Gateway、API Key、Usage、Quota 或 Cost，也不要求立即实现完整 Gateway、完整商业账单、复杂多租户、复杂调度或高可用。

### 后续代码落地路线

后续代码落地应按组件边界小步推进：

1. 先固化边界，不改现有实例启停、Agent 心跳、日志审计、GPU collector 和 Runtime 检查主路径。
2. 新增独立 Gateway workspace/package 骨架，确认其作为独立进程或独立二进制的启动、配置、日志和健康检查边界；该阶段不实现业务模型转发、API Key、Usage、Quota 或 Cost。
3. 由 Agent 以本地 supervisor 方式托管 Gateway 生命周期，通过 `agent_tasks` 接收 Server 控制面任务；Agent 只负责部署、启动、停止、重启、健康检查、日志读取、配置落地和状态上报。
4. Server 再增加 Gateway 配置、策略、状态和审计等控制面管理能力，并保持业务流量不经过 Server。
5. 在 Gateway 边界稳定后，先引入 Service、API Key 和 Usage，再分阶段推进 Quota、Cost 和报表治理能力。

第一轮最小代码改动应止于独立 Gateway 进程骨架和 Agent 侧托管边界验证，不新增数据库迁移、Server 控制面 API、Web 页面或具体业务策略模型。

## 主要数据流

### Agent 注册与心跳

```text
Agent 启动
  -> POST /api/agent/register
  -> Server 返回 node_id、agent_token、有效配置
Agent 循环
  -> POST /api/agent/heartbeat (Bearer token)
  -> Server 保存节点/GPU/指标/受管实例状态并返回最新配置
```

Server 用 name 和 hostname 的唯一约束维护节点身份。Agent token 失效时，Agent 会重新注册并更新本地 state。

### Runtime 检查

```text
Web 创建/检查 Runtime
  -> Server 创建 check_runtime_environment 任务
  -> Agent 检查本地入口路径或 Docker 镜像
  -> Server 保存 check_status / check_message
```

Runtime 必须绑定在线节点。`binary` 需要受控入口路径，`docker` 需要镜像配置。`script` 后端路径保留兼容，但不作为 v0.1 用户可见主流程。

### 本地实例生命周期

```text
Web start/stop/test/check
  -> Server 校验实例、节点、Runtime、Model File
  -> Server 创建 Agent 任务并设置 starting/stopping 等过渡态
  -> Agent 执行本地程序或 Docker 操作
  -> Agent 上报结果
  -> Server 更新实例状态、地址、进程/容器引用、日志摘要和错误信息
```

`running` / `starting` / `stopping` 的 Instance 不能修改配置或删除。被运行中实例引用的 Runtime 和 Model 也不能修改。

### 状态恢复

- Agent 启动后读取 managed store，只恢复平台曾启动并持久化的受管记录。
- local 进程通过 pid + start_time 校验，降低 PID 复用误判。
- Docker 容器通过 `docker inspect` 校验。
- Server 重启后依赖下一次 Agent 心跳 reconcile 实例状态。
- Agent 离线不等于实例失败；Server 保留原实例状态，Web 使用 `node_online=false` 展示 warning。

## Backend 生命周期模型

不同 backend 有根本不同的生命周期语义，不可混淆：

### Binary（llama.cpp 等本地进程）
- 每个 Instance 对应一个独立进程。
- start → Agent 启动二进制；stop → Agent kill 进程。
- 通过 managed_process 跟踪存活，心跳上报 managed_instances。
- heartbeat reconcile：未上报时可能标记 failed。
- `script` 后端的解析、校验和 Agent task 兼容逻辑暂时保留，用于既有数据；Web v0.1 不再把它作为新增推荐路径展示。

### Docker（vLLM 等容器）
- 每个 Instance 对应一个 `docker run --detach` 容器。
- start → `docker run`；stop → `docker stop`。
- Docker 参数由 Runtime `params_json` + Instance `params_json` 三层合并。
- 支持 gpu_memory_utilization、max_model_len、tensor_parallel_size 等参数。

### Ollama（共享 daemon）
- Runtime = Ollama daemon 配置；Instance = 某个 model name 的逻辑实例。
- 多个 Instance 共享同一个 daemon。
- start = 加载/预热模型（POST /api/generate warmup），不启动新进程。
- stop = 卸载模型（keep_alive=0），不停止 daemon。
- **不依赖 managed process 心跳**；heartbeat reconcile 必须跳过。
- check/test 通过 Ollama API 判断；test 不应因 DB status 非 running 被拦截。
- 模型来源为节点本地 Ollama 模型列表（/api/tags），不强制绑定 model_file_id。
- Runtime 保存只校验格式，不检查 daemon 是否在线。
- 暂缓：自动 pull、GPU 可见性控制、多 daemon 调度。

## Docker 原则

- Docker 容器由 Agent 通过 `docker run --detach` 启动，不默认加 `--rm`，保留异常退出后的 inspect/logs 诊断能力。
- Agent 退出不停止容器；用户显式 stop 才执行 `docker stop`。
- Docker 参数由 Model File 路径、Runtime `params_json` 和 Instance `params_json` 合并得到。
- Docker 操作写入 lightai-agent.log 的 command summary，并进行脱敏。

## 配置模型

Agent 本地 TOML 主要是 bootstrap：Server 地址、节点名、监听地址、state 路径等。运行期策略由 Server 合成：

```text
内置默认 + 全局策略 + 节点覆盖 -> effective_agent_config
```

当前在线下发字段包括心跳/采样间隔、命令和检查超时、allowed dirs、collector 执行超时/输出上限、日志策略等。GPU collector 的本地目录、启用列表和禁用列表属于 Agent bootstrap 配置。

## 当前阶段范围

已实现：

- Server / Agent / Web 基础闭环。
- 单节点单副本模型实例管理。
- 外部服务接入和本地实例生命周期。
- Runtime、Model、Model File、Trash、日志审计、用户/用户组和基础配置页面。
- 系统/GPU 指标当前状态和历史趋势。
- Ollama v0.1 最小可用（共享 daemon 模式）。
- vLLM Docker 参数配置（含 tensor_parallel_size）。
- llama.cpp gpu_layers 参数治理（默认不传参，CPU-only 可显式设置）。

部分完成：

- Docker/vLLM 后端已有实现和测试，但仍缺真实 GPU 环境端到端验证。
- Ollama 暂不支持自动 pull、GPU 可见性控制、多 daemon 调度。

当前阶段暂未实现，作为后续阶段目标保留：

- 独立 Gateway 数据面、API Key、Service、Usage、Quota、Cost、报表和治理能力。
- GPU 资源调度、关键模型优先级、自动扩缩容、降级策略、高可用和复杂 IAM/SSO。
- 指标清理/聚合/降采样后台任务。
- 厂商 GPU SDK collector。

更多 API、表结构、任务类型和参数合并细节见 [IMPLEMENTATION_NOTES.md](IMPLEMENTATION_NOTES.md)。
