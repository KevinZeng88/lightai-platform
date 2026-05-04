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


然后在 Web 中尝试用 18088 启动本地实例。

预期结果：

启动应失败；
Web 应显示明确的端口冲突原因；
不应错误显示为运行中。
安全提醒
不要用真实模型文件测试删除功能。
删除测试请使用临时文件。
模型文件物理删除必须走模型垃圾箱和 Agent 受控清理流程。
