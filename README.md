# M365 Copilot OpenAI Proxy — Rust 重写版

将 Microsoft 365 Copilot Chat 的私有 SignalR/WebSocket 协议翻译为 OpenAI / Anthropic 标准 API 格式的本地 HTTP 代理服务器。

## 项目定位

- 零 Azure 应用注册：无需管理员同意，直接用浏览器登录的 M365 Copilot 会话
- 双版本：原版 Python 版（`uv run`）和本 Rust 重写版（`cargo run`）
- Rust 版优势：无 Python 依赖链、启动更快、原生支持 CDP 无头浏览器 Token 捕获

## 系统要求

- Rust 工具链（edition 2024）
- Microsoft Edge（CDP Token 捕获功能需要）
- 一个已有 M365 Copilot 登录会话的浏览器（首次 Token 设置需要）

## 快速开始

```powershell
# 克隆后首次构建
cargo build --release

# 启动代理服务器（默认 127.0.0.1:8000）
cargo run --release -- serve

# 首次使用：启动可见 Edge 窗口登录 M365 Copilot
cargo run -- launch-edge
```

## 构建说明

### 开发模式

```powershell
cargo build
```

增量编译，适合开发迭代。

### 发布模式

```powershell
cargo build --release
```

全量优化编译，产物在 `target/release/m365-copilot-openai-proxy.exe`。

## 子命令

### `serve` — 启动 HTTP 代理

```
cargo run -- serve [选项]
```

| 选项 | 默认值 | 说明 |
|------|--------|------|
| `--host` | `127.0.0.1` | 监听地址 |
| `--port` | `8000` | 监听端口 |
| `--launch-edge` | 不启用 | 启动时同时打开可见的调试 Edge 窗口 |
| `--auto-capture` | 不启用 | 启动时自动检查 Token 有效性，缺失或过期则 CDP 捕获 |
| `--cdp-port` | `9222` | Chrome DevTools Protocol 端口 |

示例：

```powershell
# 默认启动
cargo run -- serve

# 启动时打开 Edge 并开启自动捕获 Token
cargo run -- serve --launch-edge --auto-capture

# 自定义地址和端口
cargo run -- serve --host 0.0.0.0 --port 9980
```

### `set-token` — 手动设置 Token

```
cargo run -- set-token
```

交互式输入。支持两种输入格式：

1. **完整 WebSocket URL**（从 DevTools 复制）
   ```text
   wss://substrate.office.com/m365Copilot/Chathub/...?access_token=eyJ...
   ```
   程序自动从 URL 中提取 `access_token` 参数。

2. **纯 Token 值**
   ```text
   eyJhbGciOiJkaXIiLCJlbmMiOiJBMjU2...
   ```

写入 `.env`：`M365_ACCESS_TOKEN=<token>`

### `capture-token` — 无头浏览器捕获 Token

```
cargo run -- capture-token [选项]
```

| 选项 | 默认值 | 说明 |
|------|--------|------|
| `--cdp-port` | `9222` | CDP 调试端口 |
| `--timeout-seconds` | `90` | 超时秒数 |

执行流程：

1. 检查 `localhost:9222` 是否已有 Edge 实例在运行
2. 如果没有，启动无头 Edge（`--headless=new`），加载 `https://m365.cloud.microsoft/chat`
3. 通过原始 TCP HTTP 请求获取 `http://localhost:9222/json` 返回的调试页面列表
4. 查找 URL 以 `https://m365.cloud.microsoft/` 开头的标签页
5. 连接该标签页的 CDP WebSocket 调试端点
6. 发送 `Network.enable` 启用网络事件监听
7. 发送 `Page.reload` 刷新页面，触发 `substrate.office.com` 的 WebSocket 连接
8. 监听 `Network.webSocketCreated` 事件，检测 URL 包含 `substrate.office.com` 且带 `access_token=` 的 WebSocket
9. 使用正则 `[?&]access_token=([^&]+)` 提取 Token
10. 写入 `.env` 并退出

注：需要先通过 `launch-edge` 完成一次登录。未登录时会报错并提示先执行 `launch-edge`。

### `launch-edge` — 启动可见 Edge 用于登录

```
cargo run -- launch-edge [选项]
```

| 选项 | 默认值 | 说明 |
|------|--------|------|
| `--cdp-port` | `9222` | 远程调试端口 |

执行流程：

1. 查找 `msedge.exe`（标准安装路径：`Program Files (x86)` 或 `Program Files`）
2. 创建专用用户数据目录：`%USERPROFILE%\.m365-copilot-openai-proxy\edge-profile\`
3. 启动 Edge：
   ```
   msedge.exe --remote-debugging-port=9222 ^
              --user-data-dir=<profile_dir> ^
              --no-first-run ^
              https://m365.cloud.microsoft/chat
   ```
4. 在弹出的 Edge 窗口中完成 M365 Copilot 登录
5. 后续所有 CDP 操作重用到这个专用配置

## 客户端配置

### OpenAI 兼容客户端

| 配置项 | 值 |
|--------|-----|
| Base URL | `http://127.0.0.1:8000/v1` |
| API Key | `dummy`（任意字符串，服务端不校验） |
| 模型名 | `m365-copilot` |

### OpenCode

```powershell
$env:OPENAI_BASE_URL = "http://127.0.0.1:8000"
$env:OPENAI_API_KEY = "dummy"
opencode
```

### Claude Code

```powershell
$env:ANTHROPIC_BASE_URL = "http://127.0.0.1:8000"
$env:ANTHROPIC_API_KEY = "dummy"
claude
```

限制：本代理不支持 tool use。M365 Copilot 不会输出 `tool_use` 内容块，因此 Claude Code 的文件读取、命令执行、代码编辑等智能体功能需要直连 Anthropic API。

### Continue

编辑 `~/.continue/config.json`：

```json
{
  "models": [
    {
      "title": "M365 Copilot",
      "provider": "openai",
      "model": "m365-copilot",
      "apiBase": "http://127.0.0.1:8000/v1",
      "apiKey": "dummy"
    }
  ]
}
```

## 快速验证

```powershell
$body = @{
  model = "m365-copilot"
  messages = @(
    @{ role = "user"; content = "Say hello in 5 words" }
  )
} | ConvertTo-Json -Depth 10

$r = Invoke-RestMethod `
  -Method Post `
  -Uri "http://127.0.0.1:8000/v1/chat/completions" `
  -ContentType "application/json" `
  -Body $body

$r.choices[0].message.content
```

## 目录结构

```
rust-rewrite/
├── Cargo.toml          # 项目配置和依赖
├── Cargo.lock          # 依赖锁定文件
├── .env                # Token 和环境变量（不纳入版本控制）
├── src/
│   ├── main.rs         # CLI 入口：子命令分发
│   ├── app.rs          # Axum HTTP 路由和请求处理器
│   ├── config.rs       # 环境变量配置加载
│   ├── models.rs       # 请求/响应数据结构体
│   ├── translator.rs   # API 格式翻译（OpenAI/Anthropic → 内部格式）
│   ├── substrate.rs    # WebSocket SignalR 客户端
│   ├── signalr.rs      # SignalR 消息编解码工具
│   ├── token_store.rs  # JWT/JWE Token 解析和热加载
│   ├── session_store.rs# 持久会话管理
│   ├── cdp.rs          # CDP 协议 Token 捕获
│   └── tools.rs        # 工具调用检测
├── docs/
│   ├── ARCHITECTURE.md # 架构文档
│   ├── API.md          # API 接口文档
│   ├── ENV.md          # 环境变量文档
│   └── CDP.md          # CDP Token 捕获说明
└── README.md           # 本文件
```

## 详细文档

- [架构详解](docs/ARCHITECTURE.md) — 请求流程、模块依赖、SignalR 协议、Token 生命周期
- [API 接口文档](docs/API.md) — 全部端点、请求/响应格式、SSE 事件流
- [环境变量](docs/ENV.md) — 配置变量说明、Token 持久化行为
- [CDP Token 捕获](docs/CDP.md) — 无头浏览器 Token 捕获原理和命令详解

## 许可

MIT
