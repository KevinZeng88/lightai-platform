# Local Test Environment

本文件用于 Codex / Claude Code 在本机做端到端测试。  
这里只记录本机开发测试环境，不代表生产配置。

## 本地 llama.cpp 端到端测试环境

- llama-server 执行文件：`/home/kzeng/llama.cpp/build/bin/llama-server`
- GGUF 测试模型：`/home/kzeng/models/qwen2.5-0.5b-gguf/qwen2.5-0.5b-instruct-q4_k_m.gguf`
- 建议 Server 端口：`18080`
- 建议 Agent debug/health 端口：`18081`
- Web dev 默认端口：`5173`
- 建议 llama.cpp 本地实例端口：`18088`、`18089`、`18090`

说明：

- 该 GGUF 是本地小模型测试文件。
- 可用于测试：运行环境检查、模型文件验证、本地实例创建、启动、停止、测试。
- 文件验证通过只代表文件存在且基础信息可读取，不代表模型一定能正常推理。
- 避免使用 `8080`，因为 Server 或其他开发服务容易占用该端口。

## 建议测试流程

1. 启动 Server、Agent、Web。
2. 在 Web 中添加 llama.cpp 运行环境。
3. 运行环境入口文件使用：`/home/kzeng/llama.cpp/build/bin/llama-server`。
4. 在 Web 中添加模型文件。
5. 模型路径使用：`/home/kzeng/models/qwen2.5-0.5b-gguf/qwen2.5-0.5b-instruct-q4_k_m.gguf`。
6. 验证模型文件。
7. 创建本地实例。
8. 使用端口 `18088` 或更高端口。
9. 启动实例。
10. 点击测试。
11. 停止实例。
12. 验证停止后进程退出。

## 端口冲突测试

先占用 `18088`：

```bash
python3 -m http.server 18088
```

然后在 Web 中尝试用 18088 启动本地实例。

预期结果：

启动应失败；
Web 应显示明确的端口冲突原因；
不应错误显示为运行中。

## 状态检查与异常恢复验证

以下端到端场景需在真实环境中逐项验证。所有代码路径已由 92 项自动化测试覆盖。

### 1. Agent 离线状态检查

```bash
# 启动 Server、Agent、Web，创建并启动一个本地实例
# 确认 Web 显示 running（绿色标签）

# 停止 Agent（Ctrl+C 或 kill）
# 在 Web 中点击该实例的"检查状态"

# 预期：
# - 弹出红色错误通知，提示"Agent 离线，无法检查实例状态"
# - 实例状态标签变为黄色 warning（不是绿色 success）
# - 检查结果列显示"Agent 离线，无法检查实例状态"和最后检查时间
# - last_error 字段内容可见
```

### 2. Agent 重启 — 存活实例恢复

```bash
# 启动 Agent、启动一个本地实例（确认 running）
# 重启 Agent

# 预期：
# - Agent 日志显示"Agent 重启后恢复受管进程记录 N 条"
# - Agent 日志显示"Agent 上报受管实例：运行中 X，已退出 Y"
# - Web 周期刷新后实例保持 running，last_error 为空
# - 不需要人工干预
```

### 3. Agent 重启 — 已退出实例纠正

```bash
# 启动 Agent、启动一个本地实例
# 手工 kill 受管进程（kill <pid>）
# 重启 Agent

# 预期：
# - Agent 日志显示"受管实例进程已退出"
# - 实例状态变为 failed（红色标签）
# - 检查结果列显示失败原因"受管进程不存在，可能已异常退出"
```

### 4. 手工 kill 受管进程

```bash
# 启动 Agent、启动一个本地实例（确认 running）
# 手工 kill 受管进程（kill <pid>）
# 等待约 30 秒（monitor 3s + heartbeat 15s + Web refresh 15s）

# 预期：
# - 实例状态自动变为 failed
# - 不需要人工刷新页面
# - 失败原因包含"受管进程不存在"或"进程已退出"
```

### 5. Server 重启 — SQLite 状态恢复

```bash
# 确认 data/lightai.db 存在且包含节点和实例数据
# 停止 Server（Ctrl+C）
# 重新启动 Server

# 预期：
# - Server 启动成功，从 SQLite 恢复状态
# - Web 刷新后节点列表和实例列表与重启前一致
# - Agent 下一次心跳后实例状态被 reconcile 同步
```

### 6. Agent token 重注册 — node_id 不变

```bash
# 确认 Agent 已注册并获得 node_id
# 在 Server 端手动使 token 失效（或重启 Server 并删除 agent state 中的 token）
# Agent 检测到心跳 401 后自动重新注册

# 预期：
# - Agent 日志显示"Agent token 过期，重新注册"
# - 新注册返回的 node_id 与旧 node_id 一致
# - 已有实例状态不受影响
```

## 安全提醒

不要用真实模型文件测试删除功能。
删除测试请使用临时文件。
模型文件物理删除必须走模型垃圾箱和 Agent 受控清理流程。
