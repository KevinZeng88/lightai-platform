# LightAI Platform

LightAI Platform 是一个轻量级私有 GPU 模型服务管理平台。当前实现包含基础 monorepo、Server/Agent 健康检查、节点注册、Agent 心跳、基础指标采集、GPU 状态采集、SQLite 状态保存、Web 节点看板、模型定义管理和 External 模型服务接入。

## Stage 1 范围

- Rust workspace，包含独立可运行的 Server 和 Agent。
- Server 提供 `GET /health`。
- Agent 提供 `GET /health`。
- Web 使用 Vue 3 + Vite + Element Plus，提供占位首页。
- `deploy/` 提供 TOML 配置示例。
- `migrations/` 提供 SQLite migration 占位文件。

## Stage 2 范围

- Agent 向 Server 注册，Server 返回 `node_id` 和一次性明文 `agent_token`。
- Heartbeat 使用 `Authorization: Bearer <agent_token>`。
- Server 保存节点、节点最新状态、GPU 最新状态和历史采样。
- Agent 采集 CPU、内存、磁盘基础指标。
- Agent 支持 NVIDIA `nvidia-smi` 采集。
- Agent 支持自定义 GPU collector 脚本，脚本通过明确路径执行，不通过 shell。
- Web 显示节点列表、GPU 状态、最近 1 小时/6 小时/24 小时/7 天/自定义时间段趋势。

## 本地依赖

- Rust toolchain
- Node.js 和 npm
- SQLite

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

节点 API：

```bash
curl http://127.0.0.1:8080/api/nodes
curl "http://127.0.0.1:8080/api/nodes/<node_id>/metrics?from=1700000000&to=1700003600"
curl "http://127.0.0.1:8080/api/nodes/<node_id>/gpus/<gpu_key>/metrics?from=1700000000&to=1700003600"
```

历史指标接口会返回请求时间范围和实际数据范围：

```json
{
  "requested_from": 1700000000,
  "requested_to": 1700003600,
  "actual_from": 1700001200,
  "actual_to": 1700003500,
  "sample_count": 10,
  "samples": []
}
```

当没有采样点时，`actual_from` 和 `actual_to` 为 `null`，`sample_count` 为 `0`。

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

如果需要从其它机器或 Windows 浏览器访问 WSL 中的 Web 开发服务，可以使用：

```bash
npm run dev -- --host 0.0.0.0
```

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

默认不指定配置文件时使用内置默认配置。可以通过环境变量指定 TOML 配置文件：

```bash
LIGHTAI_SERVER_CONFIG=deploy/server.example.toml cargo run -p lightai-server
LIGHTAI_AGENT_CONFIG=deploy/agent.example.toml cargo run -p lightai-agent
```

内置默认配置仍绑定 `127.0.0.1`，适合纯本机开发。示例配置文件将 `listen_addr` 设置为 `0.0.0.0`，适合从其它机器或 Windows 浏览器访问 WSL 中的 Server/Agent 服务。按需修改配置文件中的监听地址后，通过 `LIGHTAI_SERVER_CONFIG` 和 `LIGHTAI_AGENT_CONFIG` 启动即可。

Agent state 文件包含 `agent_token`。Unix 下保存时会设置为 `0600` 权限，Windows 暂使用默认文件权限。

## 本机 NVIDIA 验证

这组步骤用于本地开发验证，不是必须测试项，不需要 Prometheus、Grafana 或其它外部监控系统。

1. 确认 `nvidia-smi` 可用：

```bash
nvidia-smi
nvidia-smi --query-gpu=index,name,uuid,driver_version,memory.total,memory.used,utilization.gpu,temperature.gpu,power.draw --format=csv,noheader,nounits
```

2. 启动 Server：

```bash
cargo run -p lightai-server
```

3. 启动 Agent：

```bash
cargo run -p lightai-agent
```

4. 查看节点和 GPU 是否出现：

```bash
curl http://127.0.0.1:8080/api/nodes
```

确认响应中的 `gpus` 列表包含本机 NVIDIA GPU，并检查这些字段：

- `memory_total_bytes`
- `memory_used_bytes`
- `utilization_percent`
- `temperature_celsius`

5. 查看最近时间窗口历史采样。先从 `/api/nodes` 响应中取出 `node_id` 和 `gpu_key`，再查询：

```bash
NOW=$(date +%s)
FROM=$((NOW - 3600))
curl "http://127.0.0.1:8080/api/nodes/<node_id>/metrics?from=$FROM&to=$NOW"
curl "http://127.0.0.1:8080/api/nodes/<node_id>/gpus/<gpu_key>/metrics?from=$FROM&to=$NOW"
```

响应中的 `samples` 应该包含最近心跳写入的原始采样点。

也可以使用辅助脚本做基础检查：

```bash
scripts/dev_check_nvidia.sh
```

## Stage 3A 模型与 External 接入

Stage 3A 支持模型定义管理、External 模型实例接入、运行环境登记和模型文件垃圾箱入口。

External 表示接入已有模型服务，平台不负责启动进程，也不要求先登记运行环境或绑定节点。创建流程：

1. 在 Web 的“模型”页面创建模型定义。
2. 在“实例”页面创建 External 模型实例。
3. 填写 `backend`、`base_url`、`health_url`、`endpoint_url`、`model_name` 等外部服务信息。
4. 点击“检查状态”，Server 会按 `health_url`、`endpoint_url`、`base_url` 的顺序检查可访问性。

HTTP 2xx/3xx 视为 `running`，请求失败或非成功状态视为 `failed`，没有可检查 URL 时视为 `unknown`。检查请求有超时，避免外部服务不可达时长时间阻塞。

“运行环境”页面保留为高级配置，用于描述节点本地运行能力，主要服务后续 Docker / Script 部署。Docker / Script 运行环境必须绑定节点，并要求节点 Agent 在线。本阶段不会由 Server 主动连接 Agent，也不会启动 Docker 或 Script。

模型文件垃圾箱只是待清理记录：

- 删除模型配置不会删除磁盘模型文件。
- 加入模型文件垃圾箱也不会立即删除文件。
- 后续物理删除必须由 Agent 在受控目录范围内执行。

## 本机 llama.cpp External 验证

本阶段不会启动 llama.cpp，只接入你已手工启动的服务。

1. 手工启动 llama-server：

```bash
llama-server -m /path/to/model.gguf --host 0.0.0.0 --port 8088
```

2. 优先使用 health URL：

```text
http://127.0.0.1:8088/health
```

如果 `/health` 不可用，可以使用：

```text
http://127.0.0.1:8088/v1/models
```

3. 在平台中创建模型定义，然后创建 External 模型实例：

- `backend = llama_cpp`
- `base_url = http://127.0.0.1:8088`
- `health_url = http://127.0.0.1:8088/v1/models`
- `model_name = 自定义测试名称`

点击“检查状态”后，如果 llama-server 可访问，应显示 `running`。

## Agent 配置下发

Agent 仍然采用主动连接模式：注册 Server、发送 heartbeat、上报指标和状态。Server 不主动直连 Agent。

Server 在 register 和 heartbeat 响应中下发轻量 Agent 配置：

- `heartbeat_interval_secs`
- `metrics_sample_interval_secs`
- `task_poll_interval_secs`
- `config_refresh_interval_secs`
- `command_timeout_secs`
- `environment_check_timeout_secs`
- `config_version`

Agent 本地配置作为启动默认值；Server 下发配置优先。Agent heartbeat 会上报当前实际生效配置，Web 节点列表中会展示心跳间隔、采样间隔和配置版本。

未来运行环境检查将通过 Agent 主动拉取任务实现。检查动作必须是内置固定动作，例如 `ollama --version`、`llama-server --version` 或固定 HTTP endpoint；不接受前端传入任意命令，不通过 shell 拼接命令，检查超时必须可控。

## 当前未实现，未来可扩展

- Docker / Script 模型启动
- OpenAI-compatible API gateway
- API Key 管理
- 使用量统计和计费规则
- 复杂报表、聚合、降采样和告警
- 历史数据自动清理后台任务
- Kubernetes 集成
- GPU virtualization
- IAM/SSO
- 高可用部署
