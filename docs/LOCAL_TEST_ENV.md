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

以下端到端场景需在真实环境中逐项验证。所有代码路径已由 95 项自动化测试覆盖。

### 1. Agent 离线 — 自动状态检测

```bash
# 启动 Server、Agent、Web，创建并启动一个本地实例
# 确认 Web 显示 running（绿色标签）

# 停止 Agent（Ctrl+C 或 kill）
# 等待约 15-30 秒（Web 周期刷新）

# 预期（无需用户点击任何按钮）：
# - 实例状态标签自动变为黄色 warning
# - 标签文字："Agent 离线，运行状态无法确认"
# - 检查结果列显示 "[Agent 离线] 实例运行状态无法确认"
# - 显示最后心跳时间（格式化后的本地时间）
# - 实例 status 字段仍为 running（没有误改为 failed）
# - last_error 为空
```

### 2. Agent 离线 — 点击检查状态

```bash
# 停止 Agent 后点击"检查状态"

# 预期：
# - 弹出红色错误通知，提示"Agent 离线，无法检查实例状态"
# - instance last_error 记录"Agent 离线，无法检查实例状态"
# - last_checked_at 更新为当前时间
```

### 3. Agent 退出 — 模型实例进程继续存活

```bash
# 启动 Agent、启动一个本地实例（确认 running）
# 记录模型实例的 PID（ps aux | grep llama）
# 停止 Agent（Ctrl+C 或 kill）

# 预期：
# - Agent 日志显示"Agent 正在退出，不会终止受管实例"
# - Agent 日志显示 managed store 保留 N 条记录
# - 模型实例进程仍然存在（ps aux 可见原 PID）
# - 模型服务仍可访问（curl 原端口）
```

### 4. Agent 重启 — 存活实例恢复

```bash
# 启动 Agent、启动一个本地实例（确认 running）
# 重启 Agent

# 预期：
# - Agent 日志显示"Agent 重启后恢复受管进程记录 N 条"
# - Agent 日志显示"Agent 上报受管实例：运行中 X，已退出 Y"
# - Web 周期刷新后实例保持 running，last_error 清空
# - 不需要人工干预
```

### 5. Agent 重启 — 已退出实例纠正

```bash
# 启动 Agent、启动一个本地实例
# 手工 kill 受管进程（kill <pid>）
# 重启 Agent

# 预期：
# - Agent 日志显示"受管实例进程已退出"（包含 instance_id、pid）
# - Server log 显示 running→failed reconcile
# - 实例状态变为 failed（红色标签）
# - 检查结果列显示失败原因"受管进程不存在，可能已异常退出"
```

### 6. 手工 kill 受管进程

```bash
# 启动 Agent、启动一个本地实例（确认 running）
# 手工 kill 受管进程（kill <pid>）
# 等待约 30 秒（monitor 3s + heartbeat 15s + Web refresh 15s）

# 预期：
# - Agent 日志包含实例退出详情（instance_id、pid、exit_status）
# - 实例状态自动变为 failed
# - 不需要人工刷新页面
# - 失败原因包含"受管进程不存在"或"进程已退出"
```

### 7. 显式 stop instance — 进程被终止

```bash
# 在 Web 中点击"停止"按钮

# 预期：
# - 模型实例进程被 kill（ps aux 不再可见）
# - managed store 记录被移除
# - 实例状态变为 stopped
# - Agent 日志记录停止操作
```

### 8. Server 重启 — SQLite 状态恢复

```bash
# 确认 data/lightai.db 存在且包含节点和实例数据
# 停止 Server（Ctrl+C）
# 重新启动 Server

# 预期：
# - Server 启动成功，从 SQLite 恢复状态
# - Web 刷新后节点列表和实例列表与重启前一致
# - Agent 下一次心跳后实例状态被 reconcile 同步
```

### 9. Agent token 重注册 — node_id 不变

```bash
# 确认 Agent 已注册并获得 node_id
# 在 Server 端手动使 token 失效（或重启 Server 并删除 agent state 中的 token）
# Agent 检测到心跳 401 后自动重新注册

# 预期：
# - Agent 日志显示"Agent token 过期，重新注册"
# - 新注册返回的 node_id 与旧 node_id 一致
# - 已有实例状态不受影响
```

### 10. 日志格式验证

```bash
cat logs/agent.log | head -5
cat logs/server.log | head -5

# 预期：
# - 每行开头为 ISO 8601 时间戳，如 2026-05-05T10:23:11Z
# - 而非 Unix timestamp 如 1777953391
```

## 部署环境注意事项

### systemd — KillMode

若 Agent 以 systemd 运行，**必须**将 `KillMode` 设为 `process`。systemd 默认 `KillMode=control-group`，在 `systemctl stop lightai-agent` 或 `systemctl restart lightai-agent` 时会向整个 cgroup 内所有进程发送 SIGTERM，导致 Agent 启动的模型实例进程也被终止。这违反"Agent 退出不终止模型实例"的设计约束。

完整 service 示例文件见 `deploy/lightai-agent.service`，关键配置：

```ini
[Service]
KillMode=process
```

部署后验证：

```bash
# 1. 检查当前 KillMode
systemctl show lightai-agent -p KillMode

# 2. 预期输出
KillMode=process

# 3. 端到端验证：启动实例后重启 Agent service
systemctl restart lightai-agent

# 4. 确认模型实例进程未被终止
ps aux | grep llama  # 原 PID 应仍然存在
curl http://127.0.0.1:18088/health  # 服务应仍可访问
```

> **注意**：若 Agent 升级需要重启，使用 `systemctl restart` 而非 `systemctl stop && systemctl start`，配合 `KillMode=process` 确保模型实例不中断。

### Docker 容器

若 Agent 运行在 Docker 容器中，容器停止会终止容器内所有进程。如果要求模型实例在 Agent 退出后继续运行，Agent 与模型进程不能共用同一个会被停止的容器生命周期。建议：

- Agent 以 host 网络模式运行（`--network host`）
- 模型实例进程由 Agent 启动在宿主机上（Agent 容器有宿主机 PID namespace 访问权限时）
- 或模型实例运行在独立容器中，Agent 仅通过 Docker API 管理

上述部署模式不在当前平台自动化范围内，需运维侧配合。

## 安全提醒

不要用真实模型文件测试删除功能。
删除测试请使用临时文件。
模型文件物理删除必须走模型垃圾箱和 Agent 受控清理流程。

## Docker 端到端测试（手工验证）

### 前置条件

- Docker + NVIDIA GPU + nvidia-container-toolkit
- vLLM Docker 镜像：`vllm/vllm-openai:latest`
- 测试模型目录：`/data/models/qwen3-0.6b`
- 缓存目录：`/data/vllm-cache`
- 测试端口：`18000`

### 预期平台 docker run

```
docker run --name lightai-<model> --gpus all --ipc host \
  -p 18000:8000 \
  -v /data/vllm-cache:/root/.cache/huggingface \
  -v /data/models/qwen3-0.6b:/models/qwen3-0.6b:ro \
  --detach vllm/vllm-openai:latest \
  --model /models/qwen3-0.6b --served-model-name qwen3-0.6b \
  --host 0.0.0.0 --port 8000 \
  --gpu-memory-utilization 0.5 --max-model-len 4096 --max-num-seqs 8
```

注意：平台默认不加 `--rm`，使用 `--detach`。
agent.log 中记录完整 command summary，可对比手工命令。

### 测试步骤

1. **创建 Docker Runtime**：Web → 运行环境 → 新增，deploy_type=docker, backend=vllm
   - image: vllm/vllm-openai:latest
   - container_port: 8000
   - GPU: all, IPC: host
   - 缓存：/data/vllm-cache → /root/.cache/huggingface
2. **创建模型**：名称 qwen3-0.6b，路径 /data/models/qwen3-0.6b
3. **创建 Docker Instance**：选择节点、runtime、模型文件，host_port=18000
4. **启动** → `docker ps | grep lightai`，对比 agent.log 中 command summary
5. **验证** → `curl http://127.0.0.1:18000/v1/models`
6. **检查** → Web 显示 running
7. **日志** → Web 日志页面显示 command summary
8. **Agent 重启** → 容器仍在，Web 保持 running
9. **手工 `docker stop` 容器** → 等待心跳 → Web failed
10. **Web stop** → 容器停止
11. **Agent 离线** → Web warning
