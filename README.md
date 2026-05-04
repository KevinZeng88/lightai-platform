# LightAI Platform

轻量级私有 GPU 模型服务管理平台。Rust workspace + Vue/Vite Web monorepo，包含 Server、Agent、Web 控制台。

## 平台架构

```
Agent (GPU 节点) ──主动连接──> Server (中央控制面) <── Web (控制台)
```

- **Server**：Rust 服务，SQLite 存储，提供 HTTP API。负责 Agent 注册、心跳鉴权、节点/GPU 状态、配置策略合成、任务下发、日志和审计。
- **Agent**：Rust 服务，运行在 GPU 节点。主动注册 Server、上报心跳和指标、通过任务通道执行受控操作（模型验证、实例启停、文件清理等）。
- **Web**：Vue 3 + Vite + Element Plus。只调用 Server API，不直接连接 Agent。提供节点监控、配置、模型、实例、垃圾箱、日志审计页面。

## 核心能力

### Agent 注册与身份规则

- Agent 首次启动向 Server 注册，Server 返回 `node_id` 和一次性 `agent_token`。
- 后续心跳使用 `Authorization: Bearer <agent_token>` 鉴权。
- 身份规则（数据库 UNIQUE 约束保障）：
  - name 全局唯一，hostname 全局唯一。
  - same name + same hostname 视为同一节点重注册，复用 node_id，更新 token（幂等）。
  - same name + different hostname、different name + same hostname 均拒绝（400）。
- 事务保证注册原子性；并发冲突时自动重试复用已有 node_id。

### 节点监控与指标

- Agent 采集 CPU、内存、磁盘基础指标，支持 NVIDIA `nvidia-smi` 和受控 custom collector 脚本。
- Server 保存节点当前状态、GPU 状态和历史采样。
- Web 显示节点列表、GPU 状态、自定义时间段趋势图。

### 模型、模型文件与运行环境

- **模型**：平台中的模型定义，组织模型配置和文件状态。
- **模型文件**：具体节点上的文件或目录路径。创建或重新验证时由对应节点 Agent 检查路径存在性和基础信息。验证不代表模型格式正确或服务可用。
- **运行环境**：节点具备的本地运行能力（ollama / llama_cpp / vllm / custom），绑定节点，描述 backend、运行方式、入口路径、工作目录、日志目录等。

### 实例生命周期

- **External 实例**：接入已有外部模型服务。平台只记录和 HTTP 可达性检查，不负责启动/停止。
- **本地实例**：绑定节点、运行环境和已验证模型文件，由 Agent 负责启动/停止/测试。
  - 启动前端口占用检查，失败时返回中文原因。
  - 启动后按后端区分服务就绪探测路径（可通过实例参数自定义）。
  - 就绪后额外验证进程存活，防止假就绪。
  - 后台进程存活监控（3s 周期），异常退出通过心跳上报 failed。
  - running 状态下 `last_error` 为空；failed/stopped 才保留失败原因。
  - 实例操作后 Web 原地更新状态；过渡态轮询直至终态。

### 状态检查与异常恢复

- Agent 离线时，状态检查返回 "Agent 离线，无法检查实例状态"，Web 显示黄色 warning 标签和红色错误通知。
- Agent 重启后恢复 managed store 中持久化的受管进程（不扫描外部进程）。通过 `/proc/{pid}/stat` 的 start_time 校验防止 PID 复用误判。
- Agent 重启/重连后首次心跳立即上报受管实例状态；Server reconcile 同步。
- Server 重启后状态从 SQLite 恢复，下一次 Agent 心跳触发 reconcile。
- `last_error` 仅表示错误/警告；running 状态下为空。
- 受管进程被手工 kill 后 ≤33s（monitor 3s + heartbeat 15s + Web refresh 15s）自动同步为 failed。
- 外部手工启动的进程不自动纳管。

### 平台日志与审计

- Server / Agent 各自写入受控日志文件，支持级别过滤、按大小轮转、按文件数/天数保留。
- 日志目录自动创建，路径白名单管控（仅 server.log / agent.log / instance.log）。
- 日志写入和读取全程做敏感信息隐藏（token / password / authorization / secret 等）。
- Web 前端未捕获异常和 API 请求失败自动上报 Server。
- 配置、模型、模型文件、运行环境、实例、垃圾箱等关键操作均记录审计事件。

### 配置策略

- Server 以"内置默认 + 全局策略 + 节点覆盖"合成 Agent 生效配置，通过心跳和任务通道下发。
- Agent 本地配置主要是 bootstrap；运行参数和策略由 Server/Web 统一下发。

## 代码结构

```
lightai-platform/
  server/src/
    agent_tasks.rs     # Agent 任务生命周期（poll / record / timeout / notify）
    domain.rs          # 业务逻辑聚合模块（约 2444 行，待继续拆分）
    repository.rs      # 数据库访问、节点注册、心跳、reconcile
    routes.rs          # HTTP API 路由
    models.rs          # 请求/响应类型
    db.rs              # SQLite 迁移与 schema
    ...
  agent/src/
    tasks.rs           # Agent 侧任务执行（实例启停、文件验证等）
    managed_process.rs # 受管进程持久化与恢复
    heartbeat.rs       # 心跳与指标上报
    platform_log.rs    # 日志写入与脱敏
    ...
  web/src/
    components/
      InstancesPanel.vue # 实例管理 UI
      LogsAuditPanel.vue # 日志与审计 UI
      ...
  migrations/         # SQLite 迁移文件
  deploy/             # TOML 配置示例
  docs/               # 文档
```

> **当前重构状态**：`stage3a.rs` 已删除。`agent_tasks.rs` 已提取为独立模块。`domain.rs` 仍约 2444 行，聚合了运行环境、模型、模型文件、实例、垃圾箱、日志、验证等业务逻辑，后续需按业务域继续拆分。详见 `docs/REFACTOR_PLAN.md`。

## 启动

```bash
# Server（默认 127.0.0.1:8080）
cargo run -p lightai-server

# Agent（默认 127.0.0.1:8081）
cargo run -p lightai-agent

# Web（默认 127.0.0.1:5173）
cd web && npm install && npm run dev
```

## 构建与检查

```bash
cargo fmt --all --check
cargo test --workspace          # 当前 92 项
cargo build --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd web && npm run build
```

## 需真实环境手工验证

以下端到端场景代码路径已由 92 项自动化测试覆盖，完整流程仍需在 GPU + llama-server + GGUF 模型环境中验证：

- Agent 离线状态检查 → 红色错误 + yellow warning 标签
- Agent 重启后存活实例自动恢复 running（last_error 清空）
- 手工 kill 受管进程后自动纠正为 failed
- Server 重启后 SQLite 状态恢复 + heartbeat reconcile
- Agent token 重注册后 node_id 不变

详见 `docs/LOCAL_TEST_ENV.md`。

## 当前未实现

- Docker 推理进程完整启动模板、进程守护、日志流
- OpenAI-compatible API Gateway、API Key 管理
- 使用量统计、计费、复杂报表、告警
- 历史数据自动清理、降采样
- Kubernetes、GPU virtualization、IAM/SSO、高可用
- 国产 GPU 厂商 SDK collector（当前走 custom collector 适配）
