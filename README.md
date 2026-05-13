# LightAI Platform

企业级模型服务与 GPU 资源调度平台。仓库是 Rust workspace + Vue/Vite Web monorepo，包含中央 Server、节点 Agent 和 Web 控制台。

项目当前处于第一阶段（v0.1），重点是多台 GPU 服务器统一纳管、基础模型实例管理和控制台能力。后续阶段会继续推进统一模型调用入口、API Key、部门/项目/业务系统归属、额度、计量、优先级调度、费用和 SLA 分析等能力。

## 架构

```text
Agent (GPU 节点) ──主动注册/心跳/拉任务──> Server (控制面 + SQLite) <── Web (控制台)
```

- **Server**：Rust + Axum + HTTPS（自签 CA）服务，使用 SQLite 保存节点、指标、模型、运行环境、实例、任务、用户/用户组、日志策略和审计记录。内置 Web 静态资源托管。
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

Instance 的顶层类型是 `external` 或 `local`。`local` 实例的实际启动方式由所选 Runtime 的 `deploy_type` 决定。

### 支持的后端（backend）

| 后端 | 部署方式 | v0.1 状态 |
|------|---------|----------|
| **llama.cpp** | binary / script（本地进程） | 可用；gpu_layers 默认不传参，由运行环境自行决定 |
| **vLLM** | docker（推荐） | 可用；支持 gpu_memory_utilization / max_model_len / tensor_parallel_size 等参数 |
| **Ollama** | binary（daemon 托管） | v0.1 最小可用；共享 daemon，多模型逻辑实例 |
| **lmdeploy / mindie / Triton** | binary / script / docker | 预留；未完整验证 |
| **custom** | binary / script（受控脚本） | 可用 |

## 当前能力

- Agent 注册、Bearer token 心跳鉴权、name/hostname 唯一身份规则。
- CPU、内存、磁盘指标采集；GPU 指标通过受 registry/hash 校验的脚本化 collector 上报。
- 节点当前状态和历史指标查询，Web 支持节点/GPU 趋势图。
- Agent 配置策略：全局默认 + 节点覆盖，随心跳/任务轮询下发。
- Runtime 管理和 Agent 侧可用性检查。
- Model 与节点模型文件管理；新增/编辑时由 Agent 验证路径存在和基础信息。
- Model File 垃圾箱；物理删除通过 Agent 在受控目录内执行。
- Model Instance 创建、编辑、删除、检查、启动、停止、测试、日志刷新。
- 本地实例支持 `binary` / `script` / `docker` Runtime。
- **Ollama**：共享 daemon 模式，多个 Instance 加载不同模型，stop 卸载模型不停止 daemon；Instance 页面支持刷新本地模型列表。
- **vLLM Docker**：支持 gpu_memory_utilization、max_model_len、max_num_seqs、tensor_parallel_size 等参数配置。
- **llama.cpp**：gpu_layers 默认不再强制传 0，用户可显式设置 0 做 CPU-only 调试。
- Agent 退出不主动终止受管实例；Agent/Server 重启后通过 managed store 和心跳 reconcile 状态。
- 平台日志、实例日志摘要、前端错误上报、审计事件基础展示，以及本地用户登录/退出、用户组和极简权限继承。
- 历史指标采样数据自动清理（默认保留 7 天，可配置），防止 SQLite 长期膨胀。

## 阶段规划

| 阶段 | 目标 | 当前状态 |
|------|------|----------|
| 第一阶段 | GPU 服务器统一纳管、Agent 心跳/GPU 状态上报、基础模型/Runtime/实例管理、Web 控制台、本地用户与用户组 | v0.1 |
| 第二阶段 | 模型服务管理与统一调用入口，包括 OpenAI-compatible API Gateway、模型路由和调用认证 | 未实现 |
| 第三阶段 | API Key、部门/项目/业务系统归属、额度、限流、调用统计和基础计量 | 未实现 |
| 第四阶段 | GPU 资源调度、关键模型优先级、扩缩容、降级策略和资源紧张时的保障策略 | 未实现 |
| 第五阶段 | 费用归集、SLA、审计分析、运营报表和企业级治理能力 | 未实现 |

## 快速测试

### 开发环境

```bash
# Server（需要 HTTPS 证书；本地验证脚本会自动生成）
cargo run -p lightai-server

# Agent（默认连接 Server HTTPS 18443）
cargo run -p lightai-agent

# Web（默认 127.0.0.1:5173）
cd web
npm install
npm run dev
```

### 本机快速验证

```bash
# 日常开发：首次自动初始化，后续自动增量更新 + 重启（默认保持运行）
bash scripts/verify-local-deployment.sh

# 强制重建
bash scripts/verify-local-deployment.sh --fresh --yes

# CI / 自动化完整测试（创建管理员 + 登录验证 + 完成后停止）
bash scripts/verify-local-deployment.sh --fresh --yes --auto --stop-after-verify

# 只停止服务
bash scripts/verify-local-deployment.sh --stop

# 清理部署目录
bash scripts/verify-local-deployment.sh --clean --yes
```

验证脚本默认工作目录为 `.local/lightai-deployment`（可通过 `LIGHTAI_VERIFY_DIR` 或 `--workdir` 覆盖）。

### 跨服务器测试

```bash
# 构建 glibc2.28 发布包（需要 Docker）
bash scripts/package-release-docker.sh v0.1.0

# 或本机 native 构建（仅限本机测试）
bash scripts/package-release.sh v0.1.0 native
```

详细安装步骤见 [INSTALL.md](INSTALL.md)。

配置示例在 `deploy/server.example.toml` 和 `deploy/agent.example.toml`。空数据库首次访问 Web 会进入初始化页面并创建第一个管理员。之后除 `/health`、`/api/setup/*`、`/api/auth/login` 与 `/api/agent/*` 外，Server 控制面 API 都需要已登录用户会话。忘记管理员密码时，在服务器本机执行 `lightai-server --reset-password <USERNAME> <PASSWORD>`。

## 检查

```bash
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd web && npm run build
```

## 代码结构

```text
server/src/          # HTTP API、业务域、SQLite 访问、任务调度、日志审计
agent/src/           # 心跳、指标/GPU 采集、任务执行、受管进程/容器恢复
  tasks/ollama.rs    # Ollama daemon 管理、模型加载/卸载/检查/测试
web/src/             # Vue 控制台、API client、页面组件
migrations/          # SQLite 初始迁移
deploy/              # 配置、systemd 示例、collector 脚本
docs/                # 架构、交接、实现细节和本地验证说明
```

## 当前阶段边界

- 第一阶段不实现 OpenAI-compatible API Gateway、API Key 管理、额度、限流、调用统计、计量、费用归集、复杂报表和告警。
- 不实现 GPU 自动调度、关键模型优先级、自动扩缩容、降级策略、Kubernetes、高可用、复杂 IAM/RBAC/SSO。
- 当前角色只有 `admin`、`operator`、`viewer`。用户直接角色与启用用户组角色共同计算 `effective_role`。Web 前端会根据 effective_role 隐藏不具备权限的写操作按钮，但后端仍是最终权限边界。
- 审计事件查询有默认 limit 500（最大 1000），支持 offset 分页。
- 历史指标采样数据自动清理，默认保留 7 天。不做小时聚合、降采样或长期报表。
- Docker/vLLM 已有代码路径和单元测试覆盖，但未在真实 GPU 环境完成完整验收。

## v0.1 已知限制

- 暂无 Prometheus / Grafana / 复杂告警系统
- 暂无 Kubernetes 集成
- 暂无 GPU 自动调度和扩缩容
- 暂无 OpenAI-compatible API Gateway
- Ollama 暂不支持自动 pull、Modelfile 管理、GPU 可见性控制（CUDA_VISIBLE_DEVICES）、多 daemon 调度
- vLLM Docker 未在所有真实 GPU 环境完成端到端验收
- 权限体系为 admin/operator/viewer 基础模型
- 当前以内部验证、小规模部署、演示为主要目标

## 文档

- [架构说明](docs/ARCHITECTURE.md)
- [AI 接手文档](docs/AI_HANDOFF.md)
- [实现细节](docs/IMPLEMENTATION_NOTES.md)
- [本地测试环境](docs/LOCAL_TEST_ENV.md)
