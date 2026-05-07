# LightAI Platform

轻量级私有 GPU 模型服务管理平台。仓库是 Rust workspace + Vue/Vite Web monorepo，包含中央 Server、节点 Agent 和 Web 控制台。

## 架构

```text
Agent (GPU 节点) ──主动注册/心跳/拉任务──> Server (控制面 + SQLite) <── Web (控制台)
```

- **Server**：Rust + Axum 服务，使用 SQLite 保存节点、指标、模型、运行环境、实例、任务、日志策略和审计记录。
- **Agent**：Rust 服务，运行在 GPU 节点上，主动连接 Server，上报系统/GPU 指标，并执行平台定义的受控任务。
- **Web**：Vue 3 + Vite + Element Plus 控制台，只调用 Server API，不直接连接 Agent 或节点本地服务。

## 核心模型

**Model + Runtime Environment + Node + Instance Overrides = Model Instance**

| 概念 | 当前职责 |
|------|----------|
| Model | 模型定义，关联一个或多个节点上的模型文件/目录路径 |
| Runtime Environment | 某节点上的运行能力，包含 backend 和 deploy_type（`binary` / `script` / `docker`） |
| Node | Agent 注册后的 GPU 节点，当前实例仍是单节点单副本 |
| Model Instance | 外部服务记录，或绑定 Model File + Runtime + Node 后由 Agent 管理的本地实例 |

Instance 的顶层类型是 `external` 或 `local`。`local` 实例的实际启动方式由所选 Runtime 的 `deploy_type` 决定，当前支持本地程序、受控脚本和 Docker 容器。

## 当前能力

- Agent 注册、Bearer token 心跳鉴权、name/hostname 唯一身份规则。
- CPU、内存、磁盘、NVIDIA GPU 指标采集；支持受控 custom GPU collector。
- 节点当前状态和历史指标查询，Web 支持节点/GPU 趋势图。
- Agent 配置策略：全局默认 + 节点覆盖，随心跳/任务轮询下发。
- Runtime 管理和 Agent 侧可用性检查。
- Model 与节点模型文件管理；新增/编辑时由 Agent 验证路径存在和基础信息。
- Model File 垃圾箱；物理删除通过 Agent 在受控目录内执行。
- Model Instance 创建、编辑、删除、检查、启动、停止、测试、日志刷新。
- 本地实例支持 `binary` / `script` / `docker` Runtime，Docker 路径已实现但仍需真实 GPU 环境端到端验证。
- Agent 退出不主动终止受管实例；Agent/Server 重启后通过 managed store 和心跳 reconcile 状态。
- 平台日志、实例日志摘要、前端错误上报和审计事件基础展示。

## 启动

```bash
# Server（默认 127.0.0.1:8080）
cargo run -p lightai-server

# Agent（默认 127.0.0.1:8081）
cargo run -p lightai-agent

# Web（默认 127.0.0.1:5173）
cd web
npm install
npm run dev
```

配置示例在 `deploy/server.example.toml` 和 `deploy/agent.example.toml`。

## 检查

```bash
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd web && npm run build
```

本地 NVIDIA 环境可额外运行：

```bash
bash scripts/dev_check_nvidia.sh
```

## 代码结构

```text
server/src/          # HTTP API、业务域、SQLite 访问、任务调度、日志审计
agent/src/           # 心跳、指标/GPU 采集、任务执行、受管进程/容器恢复
web/src/             # Vue 控制台、API client、页面组件
migrations/          # SQLite 初始迁移；部分幂等 schema 修正在 server/src/db.rs
deploy/              # 配置和 systemd 示例
docs/                # 架构、交接、实现细节和本地验证说明
```

## 当前边界

- 未实现 OpenAI-compatible API Gateway、API Key 管理、用量统计、计费、复杂报表和告警。
- 未实现多节点调度、自动 GPU 调度、Kubernetes、高可用、IAM/SSO。
- 未实现历史指标自动清理、降采样或聚合后台任务。
- 未内置国产 GPU 厂商 SDK collector，目前通过 custom collector 适配。
- Docker/vLLM 已有代码路径和单元测试覆盖，但未在真实 GPU 环境完成完整验收。

## 文档

- [架构说明](docs/ARCHITECTURE.md)
- [AI 接手文档](docs/AI_HANDOFF.md)
- [实现细节](docs/IMPLEMENTATION_NOTES.md)
- [本地测试环境](docs/LOCAL_TEST_ENV.md)
