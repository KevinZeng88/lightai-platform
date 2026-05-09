# LightAI Platform 安装指南

适用于将 lightai-platform release 包拷贝到 Linux 服务器进行安装验证。

## 1. 安装前依赖

### 系统要求

- Linux x86_64（glibc 2.31+）

验证：

```bash
ldd --version | head -1
```

二进制动态依赖检查（应在任何标准 Linux 发行版上通过）：

```bash
ldd bin/lightai-server
ldd bin/lightai-agent
```

SQLite 已静态编译到二进制中，**不需要安装 libsqlite3-dev 或 libsqlite3-0**。  
数据库文件由程序自动创建，无需手工建表。

### GPU 测试（可选）

- NVIDIA GPU 驱动 ≥ 525，`nvidia-smi` 可用
- Docker + nvidia-container-toolkit（仅 Docker/vLLM 实例测试时需要）

## 2. 安装步骤

### 2.1 解压

```bash
tar xzf lightai-platform-v0.1.0-linux-x86_64.tar.gz
cd lightai-platform-v0.1.0-linux-x86_64
```

解压后目录包含预置的 `lightai-server.toml`，已启用 Web 静态文件服务。

### 2.2 准备目录

```bash
mkdir -p run logs data
```

### 2.3 配置

直接使用预置的 `lightai-server.toml`（已启用 Web），或复制 example 自行定制：

```bash
# 预置配置已可用；如需定制：
cp config/server.example.toml lightai-server.toml
```

编辑 `lightai-server.toml`，至少检查：

- `[server].listen_addr` — 监听地址（默认 0.0.0.0:10080）
- `[web].dist_dir` — Web 静态文件目录（默认 `web/dist`，注释掉则禁用）
- `[database].url` — 数据库路径（默认 `sqlite://./data/lightai.db`）
- `[metrics].retention_days` — 历史指标保留天数（默认 7）
- `[logs].dir` — 日志目录（默认 `logs`）

复制并修改 Agent 配置：

```bash
cp config/agent.example.toml lightai-agent.toml
```

编辑 `lightai-agent.toml`，至少检查：

- `[agent].server_url` — Server 地址（默认 `http://127.0.0.1:10080`）
- `[agent].node_name` — 节点名称（可选，默认主机名）
- `[agent].state_path` — Agent 状态文件路径
- `[gpu_collectors]` — 如需 GPU 监控，配置 collector 目录和启用列表

## 3. 启动

### 3.1 启动 Server

```bash
bash scripts/start-server.sh
```

验证 Server 正常：

```bash
curl http://127.0.0.1:10080/health
# 预期：{"status":"ok","service":"server"}
```

### 3.2 启动 Agent

```bash
bash scripts/start-agent.sh
```

检查 Agent 日志确认注册成功：

```bash
tail -f logs/agent.log
# 预期看到：Agent registered, node_id=...
```

### 3.3 访问 Web

Server 直接托管 Web 控制台静态文件（通过 `[web].dist_dir` 配置，默认已启用）。

浏览器打开 `http://<服务器IP>:10080/` 即可访问。

如果 `dist_dir` 被注释或未配置，Server 退化为纯 API 模式，需单独托管 `web/dist/`。

## 4. 初始化管理员

数据库为空时，Web 会自动跳转到初始化页面，创建第一个管理员账号。

- 用户名和密码由你自行设置。
- 不支持通过配置文件或环境变量预设管理员密码。
- 忘记密码时，在服务器本机执行：

```bash
bin/lightai-server --reset-password <USERNAME> <PASSWORD>
```

要求用户登录后修改密码。

## 5. 验证

### 5.1 Web 控制台

登录后检查以下页面：

- **节点** — 应看到 Agent 上报的节点，状态为在线（绿色）
- **无 GPU 时** — 节点页 GPU 列表区域显示 "GPU collector not configured" 或 "No GPU devices found"

### 5.2 GPU 监控（可选）

如需 GPU 指标：

1. 将 collector 脚本放到 Agent 机器上
2. 用 `bin/lightai-agent collector inspect <目录>` 生成注册 JSON
3. 在 Web「采集器登记」页面粘贴登记
4. Agent 配置中设置 `[gpu_collectors]` 并重启 Agent

详见 Agent 配置模板中的注释。

## 6. 停止与清理

### 6.1 停止服务

```bash
bash scripts/stop.sh
```

### 6.2 清理测试数据

```bash
# 删除数据库（下次启动会按最新 schema 自动重建）
rm -f data/lightai.db data/lightai.db-wal data/lightai.db-shm

# 清理日志
rm -rf logs/*
```

## 7. systemd 部署（可选）

生产环境建议使用 systemd：

```bash
sudo cp systemd/lightai-server.service /etc/systemd/system/
sudo cp systemd/lightai-agent.service /etc/systemd/system/
sudo useradd -r -s /sbin/nologin lightai
sudo mkdir -p /opt/lightai/{bin,web,data,logs,run}
sudo cp bin/* /opt/lightai/bin/
sudo cp -r web/dist /opt/lightai/web/
sudo cp lightai-server.toml /opt/lightai/lightai-server.toml
sudo cp lightai-agent.toml /opt/lightai/lightai-agent.toml
sudo chown -R lightai:lightai /opt/lightai
sudo systemctl daemon-reload
sudo systemctl enable --now lightai-server lightai-agent
```

## 8. 注意事项

- SQLite 已编译内置，目标服务器不需要安装任何 SQLite 运行时。
- 数据库 schema 已编译内置于二进制，不需要额外 SQL 文件。
- 当前 MVP 不兼容历史数据库。删除 `data/lightai.db` 后会自动按最新 schema 初始化。
- 配置模板不包含真实 token、密码或密钥。
- Agent 退出不会终止模型实例进程；如需停止实例，在 Web 中显式点击「停止」。
- Agent systemd service 必须使用 `KillMode=process`。
- Docker / NVIDIA Driver / nvidia-container-toolkit / 实际 GPU 驱动属于可选外部环境依赖。
