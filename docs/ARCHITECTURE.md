# Architecture

## 整体架构

```
Agent (GPU node) ──主动连接──> Server <── Web (console)
```

- Agent 主动向 Server 注册、心跳、拉取任务。Server 不主动直连 Agent。
- Web 只调用 Server HTTP API。
- 所有本地动作（模型验证、进程启停、文件清理）由 Agent 执行，通过平台定义的任务类型下发。

## 代码结构

```
lightai-platform/
  server/src/
    main.rs            # 启动入口
    lib.rs             # 模块声明
    routes.rs          # HTTP API 路由（~990 行）
    domain.rs          # 业务逻辑聚合（~2444 行，待继续拆分）
    agent_tasks.rs     # Agent 任务生命周期（494 行）
    repository.rs      # 数据库访问、节点注册、心跳、reconcile（~1260 行）
    models.rs          # 请求/响应/视图类型
    db.rs              # SQLite 迁移、schema 修正
    auth.rs            # token 生成与验证
    config.rs          # Server 配置加载
    http_check.rs      # HTTP 可达性检查
    platform_log.rs    # 日志写入/读取/脱敏/轮转
    util.rs            # now_unix_secs() 等共享函数

  agent/src/
    main.rs            # 启动入口
    tasks.rs           # 任务执行（实例启停、文件验证、环境检查等，~1750 行）
    heartbeat.rs       # 心跳、指标采集、配置同步
    managed_process.rs # 受管进程持久化记录与恢复
    platform_log.rs    # 日志写入/读取/脱敏/轮转
    client.rs          # Server HTTP 客户端
    models.rs          # Agent 侧请求/响应类型
    state.rs           # Agent 状态文件读写
    config.rs          # Agent 配置加载
    gpu.rs             # GPU 采集（nvidia-smi + custom collector）
    metrics.rs         # CPU/内存/磁盘指标采集

  web/src/
    main.ts            # Vue 应用入口，全局错误捕获
    api.ts             # Server API 客户端
    components/
      InstancesPanel.vue   # 实例管理（~680 行）
      LogsAuditPanel.vue   # 日志与审计
      NodesPanel.vue       # 节点监控
      ModelsPanel.vue      # 模型管理
      ...
```

## 关键模块职责

### server/src/agent_tasks.rs

Agent 任务生命周期的**唯一实现**。包含：
- `poll_agent_task` — Agent 长轮询获取任务
- `record_agent_task_result` — 记录任务结果并更新关联状态
- `mark_timed_out_tasks` / `mark_task_timed_out` — 超时标记
- `notify_agent_tasks` — 唤醒等待中的 Agent 长连接

### server/src/domain.rs

**临时聚合模块**，承载运行环境、模型、模型文件、实例、垃圾箱、日志、验证等全部业务逻辑。约 2444 行。后续需按业务域拆分（详见 `docs/REFACTOR_PLAN.md`）。

当前 domain.rs 通过 re-export 保持与 routes.rs 的兼容：
```rust
pub use crate::agent_tasks::{notify_agent_tasks, poll_agent_task, record_agent_task_result};
```

### server/src/repository.rs

数据库访问层：
- `register_node` — 节点注册（事务 + 身份冲突检查 + 并发重试）
- `reconcile_managed_instances` — 心跳 reconcile 实例状态
- `record_heartbeat` — 心跳写入
- `effective_agent_config` — 配置策略合成
- `list_nodes` / `list_audit_events` — 查询

### server/src/routes.rs

Axum HTTP 路由定义和请求处理器。所有 handler 委托给 `domain::` 或 `repository::` 的具体函数。

### agent/src/tasks.rs

Agent 侧任务执行。包含：
- `start_model_instance_with_store` — 启动本地实例
- `stop_model_instance_with_store` — 停止本地实例
- `test_model_instance` — 测试实例
- `verify_model_file` — 模型文件验证
- `cleanup_model_file` — 受控文件清理
- `read_instance_log` — 读取实例日志

### web/src/components/InstancesPanel.vue

实例管理 UI，约 680 行。包含：
- 实例列表、创建/编辑表单
- start / stop / test / check 操作 + 自动刷新
- 过渡态轮询（pollInstanceUntilStable）
- 周期刷新（15s）
- 探测配置面板
- 日志查看对话框 + 刷新按钮

后续可提取 composables 进一步拆分。

## 数据流

### Agent 注册与心跳

```
Agent 启动 → POST /api/agent/register → Server 返回 node_id + token
Agent loop → POST /api/agent/heartbeat → Server reconcile 实例状态
```

### 本地实例启动

```
Web 点击启动 → POST /api/model-instances/{id}/start
  → domain::start_model_instance → 创建 agent_task
  → agent_tasks::notify_agent_tasks
  → Agent poll 获取任务 → tasks::start_model_instance_with_store
  → 端口检查 → spawn 进程 → 就绪探测 → 持久化 managed store
  → 上报结果 → agent_tasks::record_agent_task_result → 更新实例状态
```

### 状态恢复

```
Agent 重启 → managed_process::load → 逐条 /proc/{pid}/stat 校验
  → reports() → heartbeat managed_instances
  → Server reconcile_managed_instances → running 实例保持 running
  → 已退出实例标记为 failed + 原因
```
