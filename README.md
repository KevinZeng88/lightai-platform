# LightAI Platform

企业级模型服务与 GPU 资源调度平台。仓库是 Rust workspace + Vue/Vite Web monorepo，包含中央 Server、节点 Agent 和 Web 控制台。

项目当前仍处于第一阶段，重点是多台 GPU 服务器统一纳管、基础模型实例管理和控制台能力。当前实现不代表最终产品只做 GPU 服务器管理；后续阶段会继续推进统一模型调用入口、API Key、部门/项目/业务系统归属、额度、计量、优先级调度、费用和 SLA 分析等能力。

## 架构

```text
Agent (GPU 节点) ──主动注册/心跳/拉任务──> Server (控制面 + SQLite) <── Web (控制台)
```

- **Server**：Rust + Axum 服务，使用 SQLite 保存节点、指标、模型、运行环境、实例、任务、用户/用户组、日志策略和审计记录。
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
- CPU、内存、磁盘指标采集；GPU 指标通过受 registry/hash 校验的脚本化 collector 上报。
- 节点当前状态和历史指标查询，Web 支持节点/GPU 趋势图。
- Agent 配置策略：全局默认 + 节点覆盖，随心跳/任务轮询下发。
- Runtime 管理和 Agent 侧可用性检查。
- Model 与节点模型文件管理；新增/编辑时由 Agent 验证路径存在和基础信息。
- Model File 垃圾箱；物理删除通过 Agent 在受控目录内执行。
- Model Instance 创建、编辑、删除、检查、启动、停止、测试、日志刷新。
- 本地实例支持 `binary` / `script` / `docker` Runtime，Docker 路径已实现但仍需真实 GPU 环境端到端验证。
- Agent 退出不主动终止受管实例；Agent/Server 重启后通过 managed store 和心跳 reconcile 状态。
- 平台日志、实例日志摘要、前端错误上报、审计事件基础展示，以及本地用户登录/退出、用户组和极简权限继承。
- 历史指标采样数据自动清理（默认保留 7 天，可配置），防止 SQLite 长期膨胀。

## 阶段规划

| 阶段 | 目标 | 当前状态 |
|------|------|----------|
| 第一阶段 | GPU 服务器统一纳管、Agent 心跳/GPU 状态上报、基础模型/Runtime/实例管理、Web 控制台、本地用户与用户组 | 正在实现 |
| 第二阶段 | 模型服务管理与统一调用入口，包括 OpenAI-compatible API Gateway、模型路由和调用认证 | 未实现 |
| 第三阶段 | API Key、部门/项目/业务系统归属、额度、限流、调用统计和基础计量 | 未实现 |
| 第四阶段 | GPU 资源调度、关键模型优先级、扩缩容、降级策略和资源紧张时的保障策略 | 未实现 |
| 第五阶段 | 费用归集、SLA、审计分析、运营报表和企业级治理能力 | 未实现 |

## 启动

```bash
# Server（默认 127.0.0.1:10080；空数据库首次访问 Web 时初始化管理员）
cargo run -p lightai-server

# Agent（默认 127.0.0.1:10081）
cargo run -p lightai-agent

# Web（默认 127.0.0.1:5173）
cd web
npm install
npm run dev
```

配置示例在 `deploy/server.example.toml` 和 `deploy/agent.example.toml`。空数据库首次访问 Web 会进入初始化页面并创建第一个管理员；项目不再支持通过配置文件或 `LIGHTAI_ADMIN_PASSWORD` 写入首次管理员密码。之后除 `/health`、`/api/setup/*`、`/api/auth/login` 与 `/api/agent/*` 外，Server 控制面 API 都需要已登录用户会话。忘记管理员密码时，在服务器本机执行 `lightai-server --reset-password <USERNAME> <PASSWORD>`；重置后用户需要登录并先修改密码。

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
deploy/              # 配置、systemd 示例、collector 脚本（collectors/gpu/nvidia/）
docs/                # 架构、交接、实现细节和本地验证说明
```

## 当前阶段边界

- 第一阶段不实现 OpenAI-compatible API Gateway、API Key 管理、额度、限流、调用统计、计量、费用归集、复杂报表和告警；这些是后续阶段目标。
- 第一阶段不实现 GPU 自动调度、关键模型优先级、自动扩缩容、降级策略、Kubernetes、高可用、复杂 IAM/RBAC/SSO；这些是后续阶段目标或明确不采用的复杂化方向。
- 当前用户组只作为后续部门、项目、业务系统、API Key、额度和优先级归属的基础对象，不做资源级授权、多租户隔离、组管理员或审批流。
- 当前角色只有 `admin`、`operator`、`viewer`。用户直接角色与启用用户组角色共同计算 `effective_role`；后端据此做权限判断。`admin` 管理用户、用户组、配置和危险清理操作（含 collector registry 写操作、Trash 物理清理）；`operator` 可管理节点相关配置以外的 Runtime、模型、实例等日常运维对象；`viewer` 只读查看节点、GPU、模型、Runtime、实例、日志等状态。Web 前端会根据 effective_role 隐藏不具备权限的写操作按钮，但后端仍是最终权限边界。
- 审计事件查询有默认 limit 500（最大 1000），支持 offset 分页。
- 历史指标采样数据（`node_metric_samples` / `gpu_metric_samples`）自动清理，默认保留 7 天、每 6 小时执行一次，可通过 `[metrics]` 配置。不做小时聚合、降采样或长期报表。
- 未内置 GPU 厂商 SDK collector；已实现脚本化 collector 框架，可通过新增 collector 目录接入（无需改 Rust 代码），详见 [实现细节](docs/IMPLEMENTATION_NOTES.md)。
- Docker/vLLM 已有代码路径和单元测试覆盖，但未在真实 GPU 环境完成完整验收。

## 文档

- [架构说明](docs/ARCHITECTURE.md)
- [AI 接手文档](docs/AI_HANDOFF.md)
- [实现细节](docs/IMPLEMENTATION_NOTES.md)
- [本地测试环境](docs/LOCAL_TEST_ENV.md)
