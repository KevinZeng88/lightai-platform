# AI Handoff

## 当前状态

- 仓库是 Rust workspace + Vue/Vite Web 的 monorepo，包含 `server/`、`agent/`、`web/`、`migrations/`、`deploy/`、`docs/` 等目录。
- 当前交接时，工作区状态以 git status 为准。
- Server 是 Rust 服务，提供健康检查、Agent 注册、Bearer token 心跳鉴权、节点/GPU 当前状态、历史指标、配置策略、运行环境、模型、模型文件、实例、模型垃圾箱、日志和审计能力。Server 自身写入受控操作日志（server.log），支持可配置的日志级别、轮转和保留。
- Server 使用 SQLite 保存状态，启动时由 `server/src/db.rs` 执行当前内置迁移和幂等 schema 修正。
- Agent 是 Rust 服务，运行在 GPU 节点上，提供本地健康检查，主动注册 Server，发送心跳，上报 CPU、内存、磁盘、GPU 指标和受管本地实例状态。
- Agent GPU 采集支持内置 NVIDIA `nvidia-smi` collector 和受控 custom collector 脚本。
- Agent 通过主动任务控制通道接收受控任务，包括模型文件验证、运行环境检查、本地实例启动/停止/测试、模型文件清理、Agent 日志读取和实例日志刷新。Agent 启动本地实例前检查端口占用，启动后执行服务就绪探测（路径按后端区分），并对受管进程进行后台存活监控。
- Agent 本地配置主要是 bootstrap；运行参数、采样、collector、日志、受控模型目录等由 Server/Web 配置策略下发。
- Web 是 Vue 3 + Vite + Element Plus 控制台，页面包括节点监控、配置、运行环境、模型、实例、模型垃圾箱、日志审计。日志查看可区分 Server 系统日志、Agent 系统日志、实例日志、前端错误日志和审计日志。Web 的未捕获异常和 API 请求失败自动上报 Server 记录为前端错误日志。
- Web 只调用 Server，不直接连接 Agent 或节点本地服务。

## 架构说明

- Agent 主动连接 Server；Server 不主动直连 Agent 端口。
- Web 只访问 Server；节点本地动作必须由 Agent 执行。
- 本地动作走平台定义的任务类型，不接受前端传入任意 shell 命令。
- Agent 执行本地程序或脚本时使用 argv 方式，不通过 shell 拼接。
- Server 负责状态保存、任务创建、任务结果入库、配置合成和 Web API。
- Agent 负责节点本地事实采集、路径验证、运行环境检查、进程启动/停止、日志读取和受控文件删除。
- External 实例是外部已有服务接入，平台只记录和检查，不负责启动或停止。
- 本地实例必须绑定节点、运行环境和已验证的模型文件或目录。
- 安全边界保持保守：路径校验、受控目录、软链接检查、日志脱敏，以及 Server 不直接删除远端节点文件。

## 关键概念

- 模型：平台中的模型定义，用于组织模型配置和展示文件状态；自身不等于某个磁盘文件。
- 模型文件：某个节点上的具体文件或目录路径；创建或重新验证时由该节点 Agent 检查。
- 运行环境：某节点具备的本地运行能力，绑定节点，描述 backend、运行方式、入口、工作目录、日志目录、受控模型目录等。
- 实例：模型服务接入或运行记录，分为 External 实例和本地实例。
- External 实例：已有外部模型服务；不要求节点、运行环境或模型文件；状态检查由 Server 发起 HTTP 可达性检查。
- 本地实例：由 Agent 在节点上启动、停止和测试；启动时结合运行环境入口、模型路径和实例参数生成受控命令。
- 模型垃圾箱：面向具体节点上的具体模型文件路径；删除文件记录和物理删除分离。
- 配置：Server 以“内置默认 + 全局策略 + 节点覆盖”合成 Agent 生效配置，并通过心跳和任务控制通道同步。
- 日志：Server 和 Agent 各自写入受控日志文件，支持级别过滤、轮转和保留策略。Web 前端错误和 API 请求失败自动上报 Server 统一管理。日志写入和读取全程做敏感信息隐藏，日志文件白名单管控（仅 server.log / agent.log / instance.log）。
- 审计：Server 对配置、模型、模型文件、运行环境、实例、模型垃圾箱等关键操作记录审计事件（actor_type、operation_type、结果、错误原因），Web 支持按多维度筛选查看。前端错误也以审计事件形式入库（actor_type='frontend'），可在 Web 中与操作审计区分查看。

## 最近完成

- Server 管理 Agent 配置，并支持全局策略和节点级覆盖。
- 本地运行环境和本地实例管理流程。
- 本地实例诊断能力，包括命令摘要、日志摘要、测试动作和更清晰的错误信息。
- 受管本地进程生命周期恢复。
- 平台日志和审计基础能力。
- Web 打包体积优化。
- GPU 显存趋势零值渲染修复。
- Stage 3A External 模型管理细化。
- 共享 Agent 指导文件：`AGENTS.md` 和 `CLAUDE.md`。
- 本地实例可靠性增强：端口占用检查、按后端区分的服务就绪探测、就绪后进程存活验证、后台进程异常退出监控（3 秒周期）、实例日志刷新。
- 平台级日志能力：Server 操作日志（agent 注册、内部错误、前端错误上报）、Agent 操作日志（注册、心跳、配置更新、任务执行）、可配置日志级别/轮转/保留、日志策略在线生效。
- 前端错误上报：全局未捕获异常捕获、API 请求失败自动上报、前端错误在 Web 日志页可查看。
- 审计补全：model.update、model_file.update、model_file.verify 等之前遗漏的审计点已补齐。

## 已知限制

- 就绪探测参数（次数、间隔、超时）、后台进程监控周期等已集中为具名常量，但尚未进入 Web 配置界面或实例参数。
- Docker 运行环境当前主要做基础配置校验；真实 Docker 推理进程启动模板仍未完整实现。
- 进程守护、自动重启和完整日志流式查看仍是扩展点。
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

1. 将就绪探测参数、进程监控周期等已集中的具名常量纳入 Agent 配置策略或实例参数，使运维可在线调整。
2. 完善 Docker 运行模板：真实 Docker 推理进程的启动模板、容器生命周期和日志采集。
3. 增加历史指标保留清理、基础聚合和降采样。
4. 本地运行层稳定后，再做 OpenAI-compatible Gateway、API Key、路由和用量统计。
5. 根据实际硬件需求增加厂商 GPU collector adapter。
6. 评估引入正式 migration ledger，减少代码里的 schema 修正逻辑。
7. 完善审计记录的 Web 展示（分页、详情展开、导出）。

## 验证命令

```bash
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cd web && npm run build

本地 NVIDIA 验证可使用： scripts/dev_check_nvidia.sh
