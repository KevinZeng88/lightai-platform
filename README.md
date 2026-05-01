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

默认监听 `127.0.0.1:8081`。Agent 本地配置只保留 bootstrap 信息：Server 地址、节点名称、state 文件和调试用 health 监听地址。心跳间隔、指标采样、采集器、超时和 `allowed_model_dirs` 由 Server/Web 统一管理并下发。

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

## Migration 说明

当前 SQLite 启动迁移由 `server/src/db.rs` 控制。`0001`、`0002`、`0003` 会被直接执行；`migrations/0004_stage3a_corrections.sql` 是 Stage 3A 修正的历史参考文件，不会被自动执行。

原因是项目当前还没有 migration ledger，而 SQLite 修改列约束需要重建表。为避免重复执行 `ALTER TABLE` / `RENAME TABLE` 造成已有数据库损坏，Stage 3A 的表修正逻辑放在 `db.rs` 的幂等 schema 检查中执行。本阶段不引入完整 migration 框架。

## 配置文件

- Server 示例：`deploy/server.example.toml`
- Agent 示例：`deploy/agent.example.toml`

默认不指定配置文件时使用内置默认配置。可以通过环境变量指定 TOML 配置文件：

```bash
LIGHTAI_SERVER_CONFIG=deploy/server.example.toml cargo run -p lightai-server
LIGHTAI_AGENT_CONFIG=deploy/agent.example.toml cargo run -p lightai-agent
```

内置默认配置仍绑定 `127.0.0.1`，适合纯本机开发。示例配置文件将 `listen_addr` 设置为 `0.0.0.0`，适合从其它机器或 Windows 浏览器访问 WSL 中的 Server/Agent 服务。按需修改配置文件中的监听地址后，通过 `LIGHTAI_SERVER_CONFIG` 和 `LIGHTAI_AGENT_CONFIG` 启动即可。

Agent state 文件包含节点 ID 和 `agent_token`。Unix 下保存时会设置为 `0600` 权限，Windows 暂使用默认文件权限。不要把 state 文件提交到版本库或日志中。

## Agent 配置策略

Web 的“配置”页面提供轻量 Agent 配置入口：

- 全局默认策略：适用于所有节点。
- 节点级覆盖策略：只填写需要覆盖的字段，优先级高于全局默认。
- Agent 生效配置：Server 按“内置默认值 + 全局默认 + 节点覆盖”计算后，通过 Agent 主动连接的心跳和任务控制通道返回。

节点监控页会展示 Server 计算的生效配置、Agent 最近上报的配置版本和同步状态。策略保存后版本会递增；在线 Agent 会在下一次心跳或当前任务控制长连接返回时获取并在线应用。Server 地址、节点名称、state 文件路径和 Agent 本地 health 监听地址属于 bootstrap，修改后需要重启 Agent；Web 页面会提示这些字段不属于在线策略。

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

Stage 3A 支持 Agent 配置策略、节点运行环境登记与检查、模型文件配置、External 模型实例接入、本地实例生命周期控制和模型文件垃圾箱入口。

External 表示接入已有模型服务，平台不负责启动进程，也不要求先登记模型定义、运行环境或绑定节点。最小创建信息是实例名称、外部服务模型名和基础地址；健康检查地址、非标准 endpoint、服务实现、版本等属于高级配置。

创建流程：

1. 在 Web 的“实例”页面直接创建 External 模型实例。
2. 填写实例名称、`model_name` 和 `base_url`。
3. 按需展开高级配置，填写 `health_url`、`endpoint_url`、`backend`、版本或备注。
4. 点击“检查状态”，Server 会按 `health_url`、`endpoint_url`、`base_url` 的顺序检查可访问性。

HTTP 2xx/3xx 视为 `running`，请求失败或非成功状态视为 `failed`，没有可检查 URL 时视为 `unknown`。检查请求有超时，避免外部服务不可达时长时间阻塞。

“模型”页面定位为模型文件配置入口：确定某台服务器上有哪些模型文件或目录。新增模型路径配置时必须选择节点并填写该节点上的路径；保存时 Server 会先创建验证任务，并等待对应节点 Agent 回报结果。只有验证成功后，模型和节点路径记录才会写入数据库。同一路径即使出现在多台节点上，也需要分别登记并由各自节点 Agent 验证。

模型节点文件验证只表示：

- 该节点上的路径存在；
- 路径是普通文件或目录；
- 普通文件基础信息可读取，例如文件大小；目录会记录为目录类型。

文件验证不代表模型格式正确，不代表后端可以加载，也不代表推理服务可用。模型整体文件状态仅基于节点文件路径验证结果展示，包括未配置文件、待验证、部分节点文件已验证、全部节点文件已验证和验证失败。

每个本地模型至少需要保留一条节点文件路径。删除节点文件路径只删除平台记录，不会物理删除节点上的文件；如果某模型只剩一条节点文件路径，平台会拒绝删除该记录。

Agent 仍然采用主动连接模式。Agent 会主动建立任务控制长连接等待 `verify_model_file`、`check_runtime_environment`、`start_model_instance`、`stop_model_instance`、`test_model_instance`、`cleanup_model_file` 等受控任务；Server 有任务或配置版本更新时会唤醒等待中的连接，让 Agent 尽快获取并执行。Server 不主动直连 Agent，也不直接检查远端节点文件。Agent 离线时相关操作会失败并返回明确中文原因；Agent 在线但未及时回报时，操作会在超时后失败并保留错误信息。

“运行环境”页面用于描述某节点具备哪些本地运行能力，不再配置 External URL。运行环境必须绑定节点，当前优先支持 `ollama`、`llama_cpp`、`vllm` 和 `custom`；运行方式收敛为本地程序、脚本和 Docker。Docker 方式填写镜像，Script / 本地程序方式填写受控入口路径。新增和重新检查运行环境时，会由对应节点 Agent 执行受控检查；检查结果会区分入口可用、版本可获取、版本不可获取、不可执行、路径错误等状态。版本优先使用 Agent 返回值；无法自动获取时会提示原因，也可以在 Web 中手工填写或覆盖版本。llama.cpp/llama-server 的 `--version` 输出会过滤 CUDA/ggml 初始化噪声，不能可靠识别时显示“版本无法自动获取”。

“实例”页面区分两类实例：

- External 实例：直接添加外部已有服务，不需要节点、运行环境或模型文件；删除只删除平台接入记录，不影响外部服务。
- 本地实例：选择节点、该节点已检查通过的运行环境、该节点已验证的模型文件或目录后创建；可配置监听地址、端口、上下文、GPU 层数、线程数和一行一个的高级参数。启动、停止和测试都会创建 Agent 受控任务。删除本地实例只删除实例记录，不删除模型文件。

当前本地实例支持通过 Agent 真实启动受控本地进程，优先覆盖 llama.cpp/llama-server、Ollama、vLLM 和 custom 脚本形态：Agent 使用运行环境入口、模型路径和实例参数生成 argv 并直接执行，不通过 shell 拼接。启动后不再立即判定为 `running`，而是结合进程是否仍存在、端口和常见健康接口（`/health`、`/v1/models`、`/`）判断。启动失败会标记为失败，并把最近 stdout/stderr 摘要、错误原因和“程序 + 参数列表”形式的受控命令摘要保存到实例记录；日志会隐藏疑似 token、secret、password、authorization 等敏感行或参数。

custom 后端必须使用运行环境中配置的受控脚本路径。启动时 Agent 以 argv 方式调用脚本，停止、检查和测试也只能通过对应 Agent 任务触发；Web 不提供任意 shell 命令入口。脚本需要自行实现可审计、可超时的动作语义，平台会记录执行输出摘要。Docker 启动模板、完整日志文件采集和进程守护仍是后续扩展点。

工作目录是运行环境配置项，用于设置本地程序或脚本的 `current_dir`。未配置时 Agent 使用自身启动目录；建议为每个运行环境配置固定应用目录，例如 `/opt/lightai/apps/<runtime>`，不要依赖 `/tmp`、用户家目录或其它不稳定目录。

## GPU 发现与采集边界

当前 Agent GPU 采集有两条路径：

- 内置 NVIDIA collector：启用后调用 `nvidia-smi --query-gpu=... --format=csv,noheader,nounits`，解析显存、利用率、温度、功耗、驱动等基础字段。
- 自定义 collector 脚本：由 Agent 配置策略下发明确脚本路径，Agent 直接执行该路径，不通过 shell。脚本输出统一 JSON 后可映射到 `gpu_key`、vendor、名称、显存、利用率、温度、功耗和 raw JSON。

国产 GPU 当前不做大而全硬件管理，也不直接依赖厂商 SDK。推荐扩展路径是在节点上安装厂商工具或轻量适配脚本，通过自定义 collector 返回统一 GPU 指标。后续如需正式支持某类国产卡，应优先新增独立 collector adapter，而不是把厂商 SDK 逻辑散落到 Server 或 Web。

模型文件垃圾箱面向具体节点上的具体模型文件路径：

- 删除模型配置不会删除磁盘模型文件。
- 删除模型配置不会自动登记垃圾箱记录。
- 垃圾箱记录对应具体节点上的具体模型文件路径。
- “删除文件”会创建 `cleanup_model_file` 任务，由对应节点 Agent 在本机物理删除文件。
- “删除记录”只从垃圾箱页面移除记录，不删除任何真实文件。
- Agent 只会删除当前生效配置中 `allowed_model_dirs` 受控目录内的普通文件；未配置受控目录时会拒绝物理删除。错误信息会区分未配置、目录不存在、目录不可访问、目标不在允许范围、目标不存在、非普通文件和软链接等安全风险。
- 不支持批量清理、定时清理或删除目录。

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

3. 在平台“实例”页面直接创建 External 模型实例：

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

运行环境检查、本地实例启动/停止/测试、模型路径验证和垃圾箱物理删除都通过 Agent 主动拉取任务实现。检查和启动/停止/测试动作必须是平台定义的受控动作；不接受前端传入任意命令，不通过 shell 拼接命令，检查超时必须可控。

## 当前未实现，未来可扩展

- 真实 Docker 推理进程启动模板、进程守护和日志采集
- OpenAI-compatible API gateway
- API Key 管理
- 使用量统计和计费规则
- 复杂报表、聚合、降采样和告警
- 历史数据自动清理后台任务
- Kubernetes 集成
- GPU virtualization
- IAM/SSO
- 高可用部署
