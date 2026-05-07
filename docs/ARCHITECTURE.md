# Architecture

## 总览

```text
Agent (GPU node) ──register / heartbeat / poll tasks──> Server <── Web
```

LightAI Platform 当前是单控制面、单节点单副本实例管理模型：

- **Server** 是唯一 HTTP API 入口和状态持久化位置。
- **Agent** 是节点本地事实采集和本地动作执行者。
- **Web** 是 Server API 的前端控制台，不直接访问 Agent 或节点本地服务。
- **SQLite** 保存节点、指标采样、运行环境、模型、模型文件、实例、Agent 任务、配置策略、日志策略和审计记录。

## 通信边界

- Agent 主动注册、心跳、轮询任务和上报任务结果；Server 不主动直连 Agent。
- 所有节点本地动作都通过 `agent_tasks` 表和任务轮询执行，包括模型文件验证、Runtime 检查、实例启停/测试/日志读取、文件清理。
- Web 只调用 Server API；节点离线时 Web 展示状态不可确认，而不是绕过 Server 直连节点。

## 安全边界

- Agent 只执行平台定义的任务类型，不接受任意 shell 命令。
- 本地程序、脚本、Docker 均使用 argv 方式执行，不拼接 shell 命令字符串。
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
| Runtime Environment | 某节点上的运行模板，含 backend 与 `deploy_type`（`binary` / `script` / `docker`） |
| Node | Agent 注册后的节点身份和心跳状态 |
| Model Instance | `external` 外部服务，或 `local` 受 Agent 管理实例 |

关键边界：

- `external` 实例只记录 HTTP 地址并做可达性检查，不由平台启动/停止。
- `local` 实例绑定 Node、Runtime Environment 和 verified Model File。
- Runtime 是默认模板；Instance 只保存本次覆盖参数，不修改 Runtime。
- Docker 不是实例顶层类型，而是 Runtime 的一种 `deploy_type`。

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
  -> Agent 检查二进制/脚本路径或 Docker 镜像
  -> Server 保存 check_status / check_message
```

Runtime 必须绑定在线节点。`binary`/`script` 需要受控入口路径，`docker` 需要镜像配置。

### 本地实例生命周期

```text
Web start/stop/test/check
  -> Server 校验实例、节点、Runtime、Model File
  -> Server 创建 Agent 任务并设置 starting/stopping 等过渡态
  -> Agent 执行本地程序、脚本或 Docker 操作
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

## Docker 原则

- Docker 容器由 Agent 通过 `docker run --detach` 启动，不默认加 `--rm`，保留异常退出后的 inspect/logs 诊断能力。
- Agent 退出不停止容器；用户显式 stop 才执行 `docker stop`。
- Docker 参数由 Model File 路径、Runtime `params_json` 和 Instance `params_json` 合并得到。
- Docker 操作写入 agent.log 的 command summary，并进行脱敏。

## 配置模型

Agent 本地 TOML 主要是 bootstrap：Server 地址、节点名、监听地址、state 路径等。运行期策略由 Server 合成：

```text
内置默认 + 全局策略 + 节点覆盖 -> effective_agent_config
```

当前在线下发字段包括心跳/采样间隔、命令和检查超时、allowed dirs、GPU collector、日志策略等。

## MVP 范围

已实现：

- Server / Agent / Web 基础闭环。
- 单节点单副本模型实例管理。
- 外部服务接入和本地实例生命周期。
- Runtime、Model、Model File、Trash、日志审计和基础配置页面。
- 系统/GPU 指标当前状态和历史趋势。

部分完成：

- Docker/vLLM 后端已有实现和测试，但仍缺真实 GPU 环境端到端验证。
- 模型元数据 UI 已有表单，但前后端字段尚未统一，兼容性判断不能作为强约束依据。
- SQLite schema 有迁移 SQL 和代码内幂等修正，但还没有正式 migration ledger。

未完成：

- OpenAI-compatible API Gateway、API Key、用量统计和计费。
- 多节点调度、自动 GPU 调度、高可用、IAM/SSO。
- 指标清理/聚合/降采样后台任务。
- 厂商 GPU SDK collector。

更多 API、表结构、任务类型和参数合并细节见 [IMPLEMENTATION_NOTES.md](IMPLEMENTATION_NOTES.md)。
