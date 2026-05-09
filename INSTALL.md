# LightAI Platform 安装指南

适用于将 lightai-platform release 包拷贝到 Linux 服务器进行安装验证。

## 1. 选择 release 包

### glibc2.28 包（推荐用于跨服务器部署）

```
lightai-platform-v0.1.0-linux-x86_64-glibc2.28.tar.gz
```

- 在 Rocky Linux 8 容器内编译，glibc 依赖上限为 2.28。
- 适用于：
  - **RHEL 8 / Rocky Linux 8 / AlmaLinux 8** 及更新版本
  - **Ubuntu 20.04 / Debian 11** 及更新版本
  - 大部分基于 RHEL 8 生态的国产 Linux
- 如果遇到 `GLIBC_x.xx not found` 错误，应改用此包，**不要升级目标服务器 glibc**。

### native 包（仅限本机构建测试）

```
lightai-platform-v0.1.0-linux-x86_64-native.tar.gz
```

- 在宿主机直接编译，依赖宿主机的 glibc 版本。
- 只适合同一台编译机或 glibc 版本不低于编译机的系统。
- **不建议跨服务器分发**，可能因 glibc 版本不匹配而无法启动。

## 2. 安装前依赖

### 系统要求

- Linux x86_64
- glibc 2.28+（glibc2.28 包）；或 ≥ 构建机 glibc 版本（native 包）

验证 glibc 版本：

```bash
ldd --version | head -1
```

检查二进制动态依赖：

```bash
ldd bin/lightai-server
ldd bin/lightai-agent
```

SQLite 已静态编译到二进制中，**不需要安装 libsqlite3-dev 或 libsqlite3-0**。  
数据库文件由程序自动创建，无需手工建表。

运行 glibc2.28 包**不需要**安装以下任何一项：

- libsqlite3-dev / libsqlite3-0
- nginx
- python3
- Node.js / npm

### GPU 测试（可选）

- NVIDIA GPU 驱动 ≥ 525，`nvidia-smi` 可用
- Docker + nvidia-container-toolkit（仅 Docker/vLLM 实例测试时需要）

## 3. 安装步骤

### 3.1 解压

```bash
# glibc2.28 包
tar xzf lightai-platform-v0.1.0-linux-x86_64-glibc2.28.tar.gz
cd lightai-platform-v0.1.0-linux-x86_64-glibc2.28

# 或 native 包
tar xzf lightai-platform-v0.1.0-linux-x86_64-native.tar.gz
cd lightai-platform-v0.1.0-linux-x86_64-native
```

解压后目录包含预置的 `lightai-server.toml`，已启用 Web 静态文件服务。

> **首次安装请先运行初始化脚本**：`scripts/init-server.sh`（Server 端）和 `scripts/init-agent.sh`（Agent 端），脚本会生成证书、setup token 和配置文件，不需要手动处理证书。

### 3.2 准备目录

```bash
mkdir -p run logs data
```

### 3.3 配置

直接使用预置的 `lightai-server.toml`（已启用 Web），或复制 example 自行定制：

```bash
# 预置配置已可用；如需定制：
cp config/server.example.toml lightai-server.toml
```

编辑 `lightai-server.toml`，至少检查：

- `[server].listen_addr` — 监听地址（默认 0.0.0.0:18080）
- `[web].dist_dir` — Web 静态文件目录（默认 `web/dist`，注释掉则禁用）
- `[database].url` — 数据库路径（默认 `sqlite://./data/lightai.db`）
- `[metrics].retention_days` — 历史指标保留天数（默认 7）
- `[logs].dir` — 日志目录（默认 `logs`）

复制并修改 Agent 配置：

```bash
cp config/agent.example.toml lightai-agent.toml
```

编辑 `lightai-agent.toml`，至少检查：

- `[agent].server_url` — Server 地址（默认 `http://127.0.0.1:18080`）
- `[agent].node_name` — 节点名称（可选，默认主机名）
- `[agent].state_path` — Agent 状态文件路径
- `[gpu_collectors]` — 如需 GPU 监控，配置 collector 目录和启用列表

## 4. 启动

### 4.1 启动 Server

```bash
bash scripts/start-server.sh
```

验证 Server 正常：

```bash
curl http://127.0.0.1:18080/health
# 预期：{"status":"ok","service":"server"}
```

### 4.2 启动 Agent

```bash
bash scripts/start-agent.sh
```

检查 Agent 日志确认注册成功：

```bash
tail -f logs/agent.log
# 预期看到：Agent registered, node_id=...
```

### 4.3 访问 Web

Server 直接托管 Web 控制台静态文件（通过 `[web].dist_dir` 配置，默认已启用）。

浏览器打开 `http://<服务器IP>:18080/` 即可访问。

如果 `dist_dir` 被注释或未配置，Server 退化为纯 API 模式，需单独托管 `web/dist/`。

## 5. 初始化管理员

数据库为空时，Web 会自动跳转到初始化页面，创建第一个管理员账号。

- 用户名和密码由你自行设置。
- 不支持通过配置文件或环境变量预设管理员密码。
- 忘记密码时，在服务器本机执行：

```bash
bin/lightai-server --reset-password <USERNAME> <PASSWORD>
```

要求用户登录后修改密码。

## 6. 验证

### 6.1 Web 控制台

登录后检查以下页面：

- **节点** — 应看到 Agent 上报的节点，状态为在线（绿色）
- **无 GPU 时** — 节点页 GPU 列表区域显示 "GPU collector not configured" 或 "No GPU devices found"

### 6.2 GPU 监控（可选）

如需 GPU 指标：

1. 将 collector 脚本放到 Agent 机器上（release 包已包含 `collectors/gpu/nvidia-wsl/` 示例）。
2. 在 Server 端登记 collector：

```bash
# 推荐：--config 在 collector 子命令前面
bin/lightai-server --config lightai-server.toml collector sync --root collectors/gpu

# 或使用环境变量
LIGHTAI_SERVER_CONFIG=lightai-server.toml bin/lightai-server collector sync --root collectors/gpu

# 单个 collector 登记
bin/lightai-server --config lightai-server.toml collector register --dir collectors/gpu/nvidia-wsl

# 预览
bin/lightai-server --config lightai-server.toml collector inspect --root collectors/gpu
```

3. 在 Web「配置页面 → 采集器登记」块点击「刷新」查看登记结果。
4. Agent 配置中设置 `[gpu_collectors]` 并重启 Agent。

**collector 登记说明：**

- release 包已包含 `collectors/` 目录。
- collector 子命令只读取 `collector.toml` 并计算脚本 hash，不执行采集脚本。
- 可以在 Server 运行时另开终端执行登记，不需要停止 Server。
- 必须使用与正在运行 Server 相同的配置文件/数据库。
- 如登记成功但 Web 看不到，优先检查是否写入了不同数据库。
- 不要手工修改 collector hash，不要绕过 collector registry 安全机制。

详见 Agent 配置模板中的注释。

## 7. 目标服务器实测清单

以下步骤建议在每次解压安装后执行，快速确认服务正常：

```bash
# 1. 检查二进制依赖
ldd bin/lightai-server
ldd bin/lightai-agent
# 不应出现 libsqlite3.so 或 "not found" 的库。

# 2. 复制并编辑配置
cp config/server.example.toml lightai-server.toml
cp config/agent.example.toml lightai-agent.toml
# 编辑 lightai-server.toml（至少确认 listen_addr、database.url）
# 编辑 lightai-agent.toml（至少确认 server_url）

# 3. 启动 Server
bash scripts/start-server.sh
curl http://127.0.0.1:18080/health
# 预期：{"status":"ok","service":"server"}

# 4. 验证 Web 自托管和 API 路由
curl -s http://127.0.0.1:18080/ | head -c 50
# 预期：<!doctype html>...

curl -s http://127.0.0.1:18080/api/setup/status
# 预期：{"setup_required":true}

curl -s http://127.0.0.1:18080/api/nonexistent-endpoint
# 预期：{"error":"not_found",...}（JSON 404，不是 HTML）

# 5. 启动 Agent
bash scripts/start-agent.sh
# 检查日志: tail logs/agent.log

# 6. Web 控制台
# 浏览器打开 http://<服务器IP>:18080/
# 初始化管理员 → 登录 → 检查节点列表（应看到 Agent 在线）

# 7. 停止
bash scripts/stop.sh
```

## 8. 停止与清理

### 8.1 停止服务

```bash
bash scripts/stop.sh
```

### 8.2 清理测试数据

```bash
# 删除数据库（下次启动会按最新 schema 自动重建）
rm -f data/lightai.db data/lightai.db-wal data/lightai.db-shm

# 清理日志
rm -rf logs/*
```

## 9. systemd 部署（可选）

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

## 10. 常见问题

### GLIBC_x.xx not found

改用 glibc2.28 包（`lightai-platform-v0.1.0-linux-x86_64-glibc2.28.tar.gz`），不要升级目标服务器的 glibc。

### 端口被占用

Server 默认监听 18080。修改 `lightai-server.toml` 中 `[server].listen_addr` 为其他端口。

### Web 页面打不开

确认 `lightai-server.toml` 中 `[web].dist_dir` 指向正确的 `web/dist` 路径（默认已配置）。  
确认 Server 已启动且 `curl http://127.0.0.1:18080/` 能返回 HTML。

### Agent 连不上 Server

检查 `lightai-agent.toml` 中 `[agent].server_url` 是否正确。  
检查 Agent 日志：`tail logs/agent.log`。

### 无 GPU 时的预期表现

节点页 GPU 列表区域显示 "GPU collector not configured" 或 "No GPU devices found"。这是正常状态。

### 清空测试数据

删除 `data/*.db` 会清空测试数据，下次启动 Server 会自动按最新 schema 重新初始化。

## 11. 内置角色说明

当前 MVP 使用固定三种内置角色，暂不支持自定义角色：

| 角色 | 英文 | 权限 |
|------|------|------|
| 管理员 | admin | 管理用户、用户组、Agent 配置、collector registry、Trash 物理清理、模型、Runtime、实例、审计 |
| 运维 | operator | 管理模型、Runtime、实例启停、状态检查和日志查看，不能管理用户和系统设置 |
| 只读 | viewer | 只读查看节点、GPU、模型、Runtime、实例、日志和配置 |

- 用户可以直接拥有角色，也可以通过启用状态的用户组继承角色。
- 后端统一计算最高权限 `effective_role`，前端根据角色隐藏不具备权限的操作按钮。
- 后端已实现最后一个 admin 保护：不能禁用或降级最后一个启用的管理员。
- 后续如需扩展权限模型，将结合 API Key、租户和计费统一设计。

## 12. 安全说明

- **默认 HTTPS 18443**：Server 使用自签证书提供 HTTPS（`[https]` 配置）。HTTP 18080 默认关闭，仅可选本机排障（127.0.0.1）。
- **证书生成**：`lightai-server cert init` 纯 Rust 生成自签 CA + Server 证书，不需要 openssl。
- **setup token**：首次创建管理员必须提供初始化口令；由 `lightai-server cert setup-token` 生成。
- **Agent TLS**：Agent 使用 ca.crt 校验 Server 证书。`lightai-agent ca fetch` 下载 CA 并显示指纹确认。
- **ca.crt 可分发**，ca.key / server.key 不可分发。
- **当前版本无 Agent 安装码**，适合可信内网/客户测试网段；不建议公网暴露 18443。
- **per-agent token 不自动轮换**；如怀疑泄露应禁用/重新注册 Agent。后续可扩展 token revoke/rotate。

## 13. 依赖说明

- **SQLite**：已 bundled 编译到二进制中，不依赖系统 `libsqlite3.so`。
- **Web**：由 Server 自托管（`[web].dist_dir = "web/dist"`），不需要 nginx。
- **系统库**：仅依赖 Linux 标准基础库（`libgcc_s`、`libpthread`、`libm`、`libdl`、`libc`、`ld-linux`）。
- **不需要**：libsqlite3-dev、nginx、python3、Node.js、openssl、curl。
- **GPU/Docker**：Docker、NVIDIA Driver、nvidia-container-toolkit 只在 Docker/vLLM/GPU 实例测试时需要，不在 release 包内。
