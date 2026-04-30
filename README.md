# LightAI Platform

LightAI Platform 是一个轻量级私有 GPU 模型服务管理平台。Stage 1 只建立最小 monorepo、Server/Agent 健康检查、Web 占位控制台、配置示例和 SQLite migration 占位。

## Stage 1 范围

- Rust workspace，包含独立可运行的 Server 和 Agent。
- Server 提供 `GET /health`。
- Agent 提供 `GET /health`。
- Web 使用 Vue 3 + Vite + Element Plus，提供占位首页。
- `deploy/` 提供 TOML 配置示例。
- `migrations/` 提供 SQLite migration 占位文件。

## 本地依赖

- Rust toolchain
- Node.js 和 npm
- SQLite 用于 MVP 数据库方向；Stage 1 不连接数据库。

## 仓库结构

```text
lightai-platform/
  server/       # Rust Server
  agent/        # Rust Agent
  web/          # Vue 3 + Vite 控制台
  migrations/   # SQLite migration 文件
  deploy/       # 本地部署和配置示例
  docs/         # 文档
  scripts/      # 脚本
```

## 启动 Server

```bash
cargo run -p lightai-server
```

默认监听 `127.0.0.1:8080`。

```bash
curl http://127.0.0.1:8080/health
```

期望响应：

```json
{"status":"ok","service":"server"}
```

## 启动 Agent

```bash
cargo run -p lightai-agent
```

默认监听 `127.0.0.1:8081`。

```bash
curl http://127.0.0.1:8081/health
```

期望响应：

```json
{"status":"ok","service":"agent"}
```

## 启动 Web

```bash
cd web
npm install
npm run dev
```

默认访问地址为 `http://127.0.0.1:5173`。

## 构建和测试

```bash
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
```

```bash
cd web
npm run build
```

## 配置文件

- Server 示例：`deploy/server.example.toml`
- Agent 示例：`deploy/agent.example.toml`

Stage 1 代码使用最小默认配置启动，暂不加载配置文件。

## 当前 MVP/Stage 1 未实现，未来可扩展

- 模型生命周期管理
- OpenAI-compatible API gateway
- API Key 管理
- 使用量统计和计费规则
- GPU 和节点监控
- Kubernetes 集成
- GPU virtualization
- IAM/SSO
- 高可用部署
