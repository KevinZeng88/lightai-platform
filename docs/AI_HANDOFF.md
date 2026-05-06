# AI Handoff

## 当前状态

- 仓库是 Rust workspace + Vue/Vite Web 的 monorepo，包含 `server/`、`agent/`、`web/`、`migrations/`、`deploy/`、`docs/` 等目录。
- 当前交接时，工作区状态以 git status 为准。
- Server 是 Rust 服务，提供健康检查、Agent 注册、Bearer token 心跳鉴权、节点/GPU 当前状态、历史指标、配置策略、运行环境、模型、模型文件、实例、模型垃圾箱、日志和审计能力。Server 自身写入受控操作日志（server.log），支持可配置的日志级别、轮转和保留。
- Server 使用 SQLite 保存状态，启动时由 `server/src/db.rs` 执行当前内置迁移和幂等 schema 修正。
- Agent 是 Rust 服务，运行在 GPU 节点上，提供本地健康检查，主动注册 Server，发送心跳，上报 CPU、内存、磁盘、GPU 指标和受管本地实例状态。
- Agent GPU 采集支持内置 NVIDIA `nvidia-smi` collector 和受控 custom collector 脚本。
- Agent 通过主动任务控制通道接收受控任务，包括模型文件验证、运行环境检查、本地实例启动/停止/测试、模型文件清理、Agent 日志读取和实例日志刷新。Agent 启动本地实例前检查端口占用，启动后按 ProbeConfig 执行服务就绪探测。
- Agent 本地配置主要是 bootstrap；运行参数、采样、collector、日志、受控模型目录等由 Server/Web 配置策略下发。
- Web 是 Vue 3 + Vite + Element Plus 控制台，页面包括节点监控、配置、运行环境、模型、实例、模型垃圾箱、日志审计。
- Web 只调用 Server，不直接连接 Agent 或节点本地服务。

## 架构说明

- Agent 主动连接 Server；Server 不主动直连 Agent 端口。
- Web 只访问 Server；节点本地动作必须由 Agent 执行。
- 本地动作走平台定义的任务类型，不接受前端传入任意 shell 命令。
- Agent 执行本地程序或脚本时使用 argv 方式，不通过 shell 拼接。
- Server 负责状态保存、任务创建、任务结果入库、配置合成和 Web API。
- Agent 负责节点本地事实采集、路径验证、运行环境检查、进程启动/停止、日志读取和受控文件删除。
- 安全边界保持保守：路径校验、受控目录、软链接检查、日志脱敏，以及 Server 不直接删除远端节点文件。

## 关键概念

- 产品模型：**Model + Runtime + Node + Instance Overrides = Model Instance**
- Runtime：描述"以什么后端、什么运行形态跑"。backend (vllm/llama_cpp/ollama/custom) × deploy_type (local/docker)。Runtime 是默认运行模板，包含 image、gpu、ipc、container_port、cache 路径、vLLM 默认参数。
- Instance：用户选择 Model + Runtime + Node，填写实例覆盖参数（container_name、host_port、model_container_path、served_model_name、资源参数覆盖）。Instance params_json 只保存覆盖参数，不重复完整 Runtime 配置。
- 参数边界：Runtime 是默认模板，Instance 是本次运行覆盖。Instance 保存不修改 Runtime。container_port 属于 Runtime（容器内服务端口），host_port 属于 Instance（宿主机映射端口）。Host 不在 UI 配置，容器内监听地址固定为 0.0.0.0。
- External 实例：已有外部模型服务，平台只记录和 HTTP 可达性检查，不负责启动/停止。
- 运行中资源锁定：running/starting/stopping 的 Instance 不能编辑配置，被此类 Instance 引用的 Runtime 和 Model 也不能修改。Server 返回 409，Web 显示警告。
- 三层配置合并：Agent 启动时 Model（模型路径+模型名）+ Runtime（image/entrypoint+默认参数）+ Instance Overrides（端口/名称/资源覆盖）→ 最终 docker run 参数。Instance 覆盖优先于 Runtime 默认。

## Docker 运行语义

- Docker 容器默认不加 `--rm`，便于 Agent 在容器退出后 inspect/logs 获取 OOM、退出码等诊断信息。
- 用户显式 stop instance 才 docker stop；删除实例/清理资源时再 docker rm。
- Agent 退出不停止容器；Agent 是管理进程，不是宿主进程。
- Agent 重启后通过 managed store 记录检查容器状态（docker inspect），存活则保持 running。
- Docker start/stop/inspect 操作均记录 command summary 到 agent.log（ISO 8601 时间戳、脱敏），便于审计和排错。
- docker run 使用 argv 方式（不通过 shell），参数始终包含 `--gpus`、`--ipc`（来自 Runtime 默认值）。
- GPU 优先级：instance.gpu > runtime.gpu > 内部默认 "all"。

## 当前验证重点

- 真实 Docker vLLM 端到端验证：
  - image: `vllm/vllm-openai:latest`
  - model path: `/data/models/qwen3-0.6b`（容器内 `/models/qwen3-0.6b`）
  - cache path: `/data/vllm-cache`
  - host_port: 18000, container_port: 8000
  - gpu_memory_utilization: 0.5, max_model_len: 4096, max_num_seqs: 8
- Docker start 前 agent.log 记录完整 command summary（image、--gpus、--ipc、端口、volume、backend args）
- Agent 重启后容器存活实例自动恢复 running
- Agent 离线时 Web 实例显示 warning 不误改为 failed

## 当前边界

- 当前 Instance 是单节点单副本；未来多节点通过 Deployment/Replica 抽象扩展。
- 不做自动 GPU 调度；GPU 参数是 Docker/vLLM 运行参数，不是平台级资源配额。
- 模型路径必须在目标 Node 上可访问。
- 不做 OpenAI-compatible API Gateway、API Key 管理、使用量统计、计费、高可用。

## 最近完成

- Docker 后端完整实现：`agent/src/tasks/docker_backend.rs`（~1060 行）
  - 三层配置合并 `merge_docker_config()`：Model + Runtime + Instance Overrides → DockerPayload
  - docker run/stop/inspect/logs 全生命周期
  - GPU/IPC 默认值修复（gpu 默认 "all"，ipc 默认 "host"）
  - Docker start/stop/inspect 操作日志记录 command summary
- Docker 与 local 统一生命周期（start/stop/check/test/logs/recover）
- Web 产品模型落地：Runtime 结构化表单、Instance Docker 参数覆盖表单
  - Runtime 默认值在 Instance 表单中显示具体值 + 来源标识
  - Instance 覆盖字段可 "恢复默认"，保存时不写 Runtime 默认值
  - container_port 只读来自 Runtime，host_port 为 Instance 可编辑
  - Runtime 表单端口统一为 "容器内服务端口"，Host 不在 UI 配置
- 运行中资源锁定：运行中 Instance 不能修改配置；被引用 Runtime/Model 不能修改
- 平台日志时间 ISO 8601；命令摘要脱敏
- 旧完整 JSON 兼容

## 已知限制

- 运行状态监控周期（3s）仍为 Agent 内部常量，不进入配置。
- 进程守护、自动重启和完整日志流式查看仍是扩展点。
- 手工 kill 进程后，状态同步到 Web 的最坏延迟约 33 秒（monitor 3s + heartbeat 15s + Web refresh 15s）。
- Docker 实例尚未在真实 GPU 环境完成端到端验证。
- OpenAI-compatible API Gateway 尚未实现。
- API Key 管理尚未实现。
- 使用量统计和计费规则尚未实现。
- 复杂报表、聚合、降采样和告警尚未实现。
- 历史数据自动清理后台任务尚未实现。
- Kubernetes、GPU virtualization、IAM/SSO、高可用部署尚未实现。
- 国产 GPU 当前推荐通过 custom collector 适配；尚未内置厂商 SDK collector。
- 模型文件验证只证明路径存在且基础信息可读，不证明模型格式正确或服务可用。
- 模型垃圾箱不支持批量清理、定时清理或删除目录。
- SQLite migration ledger 尚未正式化，部分幂等 schema 修正仍在代码中。
- 前端错误上报是 fire-and-forget 模式，网络失败时静默丢失。

## 下一步建议

1. Docker vLLM 端到端真实环境验证（vllm/vllm-openai:latest + qwen3-0.6b GPU 推理）
2. 增加历史指标保留清理、基础聚合和降采样
3. 本地运行层稳定后，再做 OpenAI-compatible Gateway、API Key、路由和用量统计
4. 根据实际硬件需求增加厂商 GPU collector adapter
5. 评估引入正式 migration ledger，减少代码里的 schema 修正逻辑
6. 完善审计记录的 Web 展示（分页、详情展开、导出）
7. 缩短手工 kill 后的状态同步延迟（例如 Agent 心跳携带退出事件而非仅依赖 store 轮询）

## 验证命令

```bash
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cd web && npm run build

本地 NVIDIA 验证可使用： scripts/dev_check_nvidia.sh
```
