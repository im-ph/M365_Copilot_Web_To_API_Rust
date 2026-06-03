# CDP Token 捕获

## 概述

Chrome DevTools Protocol（CDP）是一种使外部程序能够控制和检查基于 Chromium 的浏览器（如 Microsoft Edge、Google Chrome）的调试协议。本代理使用 CDP 以自动化的方式从 M365 Copilot 的 WebSocket 连接中提取 `access_token`，无需用户手动打开浏览器 DevTools。

这是一种**读取**操作——程序仅监听网络事件，不注入脚本、不修改页面、不与服务端交互，不会触发任何安全机制。

## 工作原理

### 整体流程

```
capture-token 命令
  │
  ├─ 检查现有 Edge 实例
  │   └─ http://localhost:9222/json → 获取标签页列表
  │
  ├─ 未运行？→ 启动无头 Edge
  │   └─ msedge.exe --headless=new --remote-debugging-port=9222
  │
  ├─ 导航到 https://m365.cloud.microsoft/chat
  │
  ├─ 寻找 M365 Copilot 标签页
  │   └─ URL 以 https://m365.cloud.microsoft/ 开头
  │
  ├─ 连接 CDP WebSocket 端点
  │
  ├─ 启用网络事件监听
  │   └─ {"id":1,"method":"Network.enable"}
  │
  ├─ 刷新页面触发 WebSocket 连接
  │   └─ {"id":2,"method":"Page.reload"}
  │
  ├─ 监听 Network.webSocketCreated 事件
  │   ├─ request.url 包含 substrate.office.com 且含 access_token=？
  │   │   ├─ 是 → 正则提取 Token
  │   │   │    access_token=([^&]+)
  │   │   └─ 否 → 继续等待
  │   └─ 超时（默认 90 秒）→ 报错退出
  │
  ├─ 解析 Token（URL 解码）
  │
  ├─ 终止 Edge 进程
  │
  └─ 写入 .env 文件
```

### CDP 协议细节

本代理使用原始 HTTP + WebSocket 与 CDP 通信，不依赖任何 CDP 客户端库。

#### 步骤 1：获取调试页面列表

```http
GET http://localhost:9222/json
```

响应示例：

```json
[
  {
    "id": "A1B2C3D4...",
    "title": "Microsoft Copilot",
    "url": "https://m365.cloud.microsoft/chat",
    "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/A1B2C3D4..."
  }
]
```

#### 步骤 2：连接到调试 WebSocket

程序使用 `tungstenite` 库建立到 `webSocketDebuggerUrl` 的 WebSocket 连接。

#### 步骤 3：发送 CDP 命令

所有 CDP 消息为 JSON 格式。使用请求-响应模式（`id` 字段匹配）。

**启用网络监控**：

```json
{"id": 1, "method": "Network.enable"}
```

**刷新页面**：

```json
{"id": 2, "method": "Page.reload"}
```

#### 步骤 4：监听事件

CDP 事件不包含 `id` 字段。程序监听 `method` 为 `Network.webSocketCreated` 的事件：

```json
{
  "method": "Network.webSocketCreated",
  "params": {
    "requestId": "12345.678",
    "url": "wss://substrate.office.com/m365Copilot/Chathub/...?access_token=eyJ...&_=..."
  }
}
```

## 命令说明

### 自动捕获（推荐）

```powershell
cargo run -- serve --auto-capture
```

启用后，服务器启动时自动执行：

1. 从 `.env` 读取 Token
2. 验证 Token 是否有效（格式检查 + JWT 过期检查 + JWE 格式检查）
3. Token 无效或不存在 → 调用 `capture-token` 自动捕获
4. 捕获成功 → 启动 Web 服务器
5. 捕获失败 → 提示用户手动执行 `set-token`

### 手动捕获

```powershell
cargo run -- capture-token
```

### 自定义 CDP 端口

```powershell
cargo run -- capture-token --cdp-port 9333
cargo run -- serve --cdp-port 9333 --auto-capture
```

## 前置条件：首次登录

CDP 捕获**要求**浏览器中已经有 M365 Copilot 的登录会话。使用 `launch-edge` 子命令完成首次登录：

```powershell
cargo run -- launch-edge
```

此命令会：

1. 查找 `msedge.exe`
   - 搜索顺序：`Program Files (x86)\Microsoft\Edge\Application\` → `Program Files\Microsoft\Edge\Application\`
   - 如果找不到，提示用户手动指定路径

2. 创建专用用户数据目录：
   ```
   %USERPROFILE%\.m365-copilot-openai-proxy\edge-profile\
   ```

3. 启动 Edge：
   ```powershell
   msedge.exe --remote-debugging-port=9222 ^
              --user-data-dir="%USERPROFILE%\.m365-copilot-openai-proxy\edge-profile\" ^
              --no-first-run ^
              https://m365.cloud.microsoft/chat
   ```

4. 在弹出的窗口中完成 M365 Copilot 登录

这个专用用户配置后续会被所有 CDP 操作复用。

### 检查登录状态

```powershell
# 查看 Edge 是否在调试端口运行
curl.exe http://localhost:9222/json

# 如果返回 JSON 且其中有一个 URL 为 https://m365.cloud.microsoft 的页面，说明已登录
```

## 超时说明

| 阶段 | 默认超时 | 说明 |
|------|---------|------|
| 等待浏览器启动 | 10 秒 | 检测 `localhost:9222` 可用 |
| 等待标签页加载 | 30 秒 | 找到 URL 为 `https://m365.cloud.microsoft/` 的页面 |
| 等待 WebSocket 连接 | 90 秒 | 通过 `--timeout-seconds` 参数自定义 |
| 总超时 | 各阶段之和 | 最坏情况约 130 秒 |

如果 `--timeout-seconds` 设为 0，则第一阶段（等待浏览器）超时设为 60 秒，WebSocket 等待永不超时（不推荐生产使用，可能导致命令挂起）。

## Token 提取正则

```rust
// 从 WebSocket URL 中提取 access_token 参数
// URL 格式：
// wss://substrate.office.com/m365Copilot/Chathub/...?
//   access_token=eyJ...&_=1718000000000
// 或
// wss://substrate.office.com/m365Copilot/Chathub/...?
//   _=1718000000000&access_token=eyJ...
lazy_static! {
    static ref WS_TOKEN_RE: Regex =
        Regex::new(r#"[?&]access_token=([^&]+)"#).unwrap();
}
```

捕获到的原始 Token 经过 URL 解码（`urlencoding::decode`）后再写入 `.env`。

## 常见问题

### 1. 找不到 msedge.exe

```
Error: msedge.exe not found in standard locations
```

手动安装 Edge 或设置环境变量：

```powershell
# 创建从 Edge 路径到标准位置的符号链接（需要管理员）
New-Item -ItemType SymbolicLink -Path "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" -Target "D:\Apps\msedge.exe"
```

### 2. CDP 端口已被占用

```
Error: Connection refused (os error 10061)
```

可能有其他 Edge 实例占用端口：

```powershell
# 查看谁在使用 9222 端口
netstat -ano | Select-String "9222"

# 终止占用进程（替换 <PID>）
taskkill /F /PID <PID>

# 或者使用不同的 CDP 端口
cargo run -- capture-token --cdp-port 9333
```

### 3. 捕获超时

```
Error: timeout waiting for WebSocket event after 90s
```

可能原因：

- 未登录：请先执行 `launch-edge` 完成登录
- 页面加载慢：增加超时 `--timeout-seconds 180`
- 网络问题：检查能否访问 `m365.cloud.microsoft` 和 `substrate.office.com`
- 已登录但 Copilot 面板未加载：确保标签页显示的是聊天界面，而不是登录页

### 4. Token 捕获成功但连接失败

```
捕获的 Token: eyJ...  ✓
  ↓
WebSocket connect error: 401 Unauthorized
```

可能 Token 已过期（JWT 有效期约 1 小时）或 Token 对当前租户无效。重新执行 `capture-token`。

## CDP 安全说明

- CDP 远程调试端口只监听 `127.0.0.1`（localhost），默认不对外暴露
- `--remote-debugging-port` 不添加 `--remote-allow-origins=*`，避免被外部访问
- 程序不通过 CDP 执行任何 JavaScript 或修改页面内容
- 捕获到的 Token 写入 `.env`，不记录日志
- 无头 Edge 进程在捕获完成后立即终止

## Windows 相关说明

### Edge 进程查找路径

```rust
let paths = [
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
];
```

### 用户数据目录

```
%USERPROFILE%\.m365-copilot-openai-proxy\edge-profile\
```

此目录包含 Edge 的 Cookie、LocalStorage 等会话数据。**不要删除此目录**，否则需要重新登录。

### 进程终止

```rust
// 查找所有 msedge.exe 进程（需通过任务管理器或系统调用）
// 终止特定端口的 Edge 进程
// 注意：不能 kill 所有 Edge 实例——可能其他 Edge 窗口正在使用
```

当前实现通过 `std::process::Command::new("taskkill")` 终止 Edge 进程。

## 备用方案

如果 CDP 捕获无法正常工作，可以使用手动方式获取 Token：

### 方法 1：DevTools 手动复制

1. 打开已登录 M365 Copilot 的 Edge 浏览器
2. 按 F12 打开 DevTools
3. 切换到 **网络（Network）** 标签
4. 筛选 WebSocket（WS）请求
5. 找到 `substrate.office.com` 的连接
6. 在 **消息（Messages）** 标签中，复制握手请求中的 URL
7. URL 中的 `access_token=...` 参数即为所需 Token

### 方法 2：手动输入（set-token）

```powershell
cargo run -- set-token
```

根据提示粘贴 Token 或完整的 WebSocket URL。
