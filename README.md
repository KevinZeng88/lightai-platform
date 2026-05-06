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

### 产品模型：Model / Runtime / Node / Instance

平台核心产品模型是：**Model + Runtime Environment + Node + Instance Overrides = Model Instance**

- **Model（模型）**：描述"跑什么模型"。包含模型名称、路径、路径类型（directory/file/ollama/custom）、模型格式（huggingface/gguf/ollama/custom）、支持的后端（vllm/llama.cpp/ollama/custom）、默认 served_model_name。模型元数据通过 `params_json` 保存，Web 提供模板快捷填充。
- **Runtime Environment（运行环境）**：描述"以什么后端、什么运行形态跑"。由 **backend** + **deploy_type** 组合。backend 是推理引擎（vllm / llama_cpp / ollama / custom），deploy_type 是运行形态（local / docker）。Docker 不是 backend，它是 deploy_type。Runtime 是默认运行模板，包含 image、gpu、ipc、container_port、cache 路径、vLLM 默认参数。Web 提供结构化表单。
- **参数边界**：Runtime 是默认模板，Instance 是本次运行覆盖。Instance 保存不修改 Runtime。container_port 属于 Runtime（容器内服务端口），host_port 属于 Instance（宿主机映射端口）。Host 不在 UI 配置，容器内监听地址固定为 0.0.0.0。GPU 优先级：instance.gpu > runtime.gpu > 内部默认 "all"。
- **Node（节点）**：描述"在哪台机器上跑"。当前每个 Model Instance 绑定一个 Node（单节点单副本）。Agent 离线时，该 Node 上实例显示 warning。未来多节点部署通过 Deployment/Replica 抽象扩展。
- **Model Instance（实例）**：用户选择 Node + Model + Runtime，填写实例覆盖参数（container_name、host_port、model_container_path、资源参数覆盖），系统合并三层配置生成最终启动参数。实例 params_json 只保存覆盖参数，不重复完整 Runtime 配置。Instance 表单显示 Runtime 默认值的具体数值（非 "未启用"），覆盖字段可恢复默认。

### 实例生命周期

- **External 实例**：接入已有外部模型服务。平台只记录和 HTTP 可达性检查，不负责启动/停止。
- **本地实例**：绑定节点、运行环境和已验证模型文件，由 Agent 负责启动/停止/测试。
  - 支持 local（本地进程）和 docker（Docker 容器）两种 deploy_type，统一使用同一套 start/stop/check/logs 操作。
  - **local**：启动本地二进制/脚本进程，记录 pid + start_time。启动前端口占用检查。
  - **docker**：通过 `docker run --detach` 启动容器（argv 方式，不通过 shell），记录 container_id + container_name。默认不加 `--rm`，便于异常退出后 inspect/logs 诊断。Agent 退出不停止容器；显式 stop 才执行 docker stop。
  - **Agent 合并三层配置**：Model（模型路径 + 模型名）+ Runtime（镜像/entrypoint + 默认参数）+ Instance Overrides（端口/名称/资源覆盖）→ 最终启动参数。Instance 覆盖优先于 Runtime 默认值。
  - Docker 容器默认不加 `--rm`，便于 Agent 在容器退出后仍能 inspect/logs 获取 OOM、退出码等诊断信息。用户显式 stop instance 才 docker stop；删除实例/清理资源时再 docker rm。
  - Docker 与 local 共用同一套 start/stop/check/test/logs 按钮和生命周期语义。
  - 就绪后额外验证进程存活（local）或 docker inspect 状态（docker），防止假就绪。
  - 后台进程存活监控（local 3s 周期 / docker heartbeat 周期 inspect），异常退出通过心跳上报 failed。
  - Docker 第一版通过高级 JSON 参数配置，后续可扩展为 Web 表单字段。
  - running 状态下 `last_error` 为空；failed/stopped 才保留失败原因。
  - 实例操作后 Web 原地更新状态；过渡态轮询直至终态。

### Agent 与模型实例的进程关系

- **Agent 是模型实例的管理进程，不是宿主进程。** Agent 退出、崩溃或升级重启时，不会主动终止它启动的模型实例。
- 模型实例以独立进程组启动（Unix `process_group(0)`），stdin 设为 null，stdout/stderr 写入受控日志文件，不依赖 Agent 进程存活。
- Agent 退出时只 flush 状态、写日志、退出自身，不遍历和 kill 受管进程。Agent 退出日志包含"不会终止受管实例"及 managed store 保留记录数。
- Agent 重启后读取 managed store，通过 `/proc/{pid}/stat` 校验原进程是否仍存活。
- **只有用户显式执行"停止实例"操作，才允许终止模型进程**。停止时优先按 pid + start_time 校验，确认是原进程后才 kill。
- **环境限制**：若 Agent 运行在 systemd 下且 KillMode=control-group，则 systemd 停止 Agent service 时会 kill 整个 cgroup 内的子进程。建议设置 `KillMode=process`。若 Agent 运行在 Docker 容器内，容器停止会终止容器内所有进程，Agent 与模型实例不能共用同一个会被停止的容器生命周期。

### 状态检查与异常恢复

- **Agent 离线 ≠ 实例 failed**。Agent 离线时，模型实例进程可能仍在运行，Server 无法确认。实例状态保持原值（如 running），`node_online` 返回 false，Web 显示黄色 warning 标签"Agent 离线，运行状态无法确认"。不将 running 误改为 failed。
- Agent 离线时，状态检查返回 "Agent 离线，无法检查实例状态"，Web 显示黄色 warning 标签和红色错误通知。
- Agent 重启后恢复 managed store 中持久化的受管进程（不扫描外部进程）。通过 `/proc/{pid}/stat` 的 start_time 校验防止 PID 复用误判。
- Agent 重启/重连后首次心跳立即上报受管实例状态；Server reconcile 同步。
- Server 重启后状态从 SQLite 恢复，下一次 Agent 心跳触发 reconcile。
- `last_error` 仅表示错误/警告；running 状态下为空（Agent 离线不写 last_error）。
- 受管进程被手工 kill 后 ≤33s（monitor 3s + heartbeat 15s + Web refresh 15s）自动同步为 failed。
- 外部手工启动的进程不自动纳管。
- Server heartbeat reconcile 将 running→failed 时写入 server log。
- Agent 进程监控检测到进程退出时写入 agent log（包含 instance_id、pid、退出原因）。
- Web 周期刷新自动更新 Agent 离线状态，用户无需手工点击检查。

### 平台日志与审计

- Server / Agent 各自写入受控日志文件，支持级别过滤、按大小轮转、按文件数/天数保留。
- 日志时间戳使用 ISO 8601 格式（如 `2026-05-05T10:23:11Z`），人类可读。
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
    domain.rs          # 轻量 facade（43 行），re-export 业务模块
    domain/
      runtimes.rs        # 运行环境 CRUD + 检查
      instances.rs       # 实例 CRUD + start/stop/test/check
      model_catalog.rs   # 模型 CRUD
      model_files.rs     # 模型文件 CRUD + 验证
      model_trash.rs     # 模型文件垃圾箱 + 清理
      instance_logs.rs   # 日志读取 + 错误摘要
      support.rs         # 共享类型、验证函数、常量
    agent_tasks.rs     # Agent 任务生命周期（poll / record / timeout / notify）
    repository.rs      # 数据库访问、节点注册、心跳、reconcile
    routes.rs          # HTTP API 路由
    models.rs          # 请求/响应类型
    db.rs              # SQLite 迁移与 schema
    ...
  agent/src/
    tasks/
      mod.rs              # facade：re-export + run/run_once 调度 + 共享类型与 helper
      process.rs          # 实例启停（start/stop）、受管进程监控、日志缓冲
      probe.rs            # 就绪探测配置、测试 URL 构建、失败摘要
      verify_model.rs     # 模型文件验证
      cleanup.rs          # 受控模型文件清理
      logs.rs             # 实例日志读取
    managed_process.rs # 受管进程持久化与恢复
    heartbeat.rs       # 心跳与指标上报
    platform_log.rs    # 日志写入与脱敏
    ...
  web/src/
    utils/
      instance.ts         # 共享状态/标签/格式化 helper
    components/
      InstancesPanel.vue # 实例管理 UI
      LogsAuditPanel.vue # 日志与审计 UI
      ...
  migrations/         # SQLite 迁移文件
  deploy/             # TOML 配置示例
  docs/               # 文档
```

> **当前重构状态**：`stage3a.rs` 已删除。`agent_tasks.rs` 已提取为独立模块。`domain.rs` 已变为 43 行轻量 facade；业务逻辑已拆入 `domain/` 下 7 个业务域模块。`agent/src/tasks.rs` 已拆为 `tasks/` 目录下的 6 个子模块。`server/tests/stage3a_api.rs` 已重命名为 `instance_lifecycle_api.rs`。`InstancesPanel.vue` 已提取 `web/src/utils/instance.ts` 公共 helper。剩余大文件：`repository.rs`（1255 行）、`routes.rs`（981 行）、`instance_lifecycle_api.rs`（2805 行）。详见 `docs/REFACTOR_PLAN.md`。

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
cargo test --workspace          # 当前 119 项（Agent 57 + Server 62）
cargo build --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd web && npm run build
```

## 需真实环境手工验证

以下端到端场景代码路径已由 95 项自动化测试覆盖，完整流程仍需在 GPU + llama-server + GGUF 模型环境中验证：

- Agent 离线状态检查 → 红色错误 + yellow warning 标签
- Agent 离线后 Web 周期刷新自动更新为 warning（无需手工点击检查）
- Agent 重启后存活实例自动恢复 running（last_error 清空）
- Agent 退出后模型实例进程仍存活（进程隔离）
- 手工 kill 受管进程后自动纠正为 failed
- Server 重启后 SQLite 状态恢复 + heartbeat reconcile
- Agent token 重注册后 node_id 不变
- 日志时间戳为 ISO 8601 格式（人类可读）
- systemd KillMode 对模型实例进程的影响（建议 KillMode=process）

详见 `docs/LOCAL_TEST_ENV.md`。

## 当前未实现

- OpenAI-compatible API Gateway、API Key 管理
- 使用量统计、计费、复杂报表、告警
- 历史数据自动清理、降采样
- Kubernetes、GPU virtualization、IAM/SSO、高可用
- 国产 GPU 厂商 SDK collector（当前走 custom collector 适配）
