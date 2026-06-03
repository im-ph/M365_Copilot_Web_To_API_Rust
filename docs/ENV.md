# 环境变量配置

## 变量列表

| 变量名 | 必填 | 默认值 | 说明 |
|--------|------|--------|------|
| `M365_ACCESS_TOKEN` | 运行时必需 | - | M365 Copilot WebSocket 连接的 access_token。通过 `set-token` 或 `capture-token` 子命令自动写入 `.env` 文件 |
| `M365_TIME_ZONE` | 否 | `Asia/Tokyo` | 发送给 M365 Copilot 的时区标识。影响时间相关对话 |
| `M365_MODEL_ALIAS` | 否 | `m365-copilot` | 客户端在 model 字段中使用的模型名。本代理仅用此名称校验允许的模型 |
| `M365_OID` | 否 | `00000000-0000-0000-0000-000000000000` | 用户 Object ID（Microsoft Entra ID）。某些企业租户可能校验此值 |
| `M365_TID` | 否 | `00000000-0000-0000-0000-000000000000` | 租户 Tenant ID（Microsoft Entra ID）。某些企业租户可能校验此值 |
| `RUST_LOG` | 否 | - | 日志级别。支持 `error`、`warn`、`info`、`debug`、`trace`。示例：`RUST_LOG=debug` |

## 配置加载顺序

```
[1] 当前目录的 .env 文件（如果存在）
[2] 系统环境变量（覆盖 .env 中的同名变量）
[3] 默认值（仅对可选变量）
```

## .env 文件格式

标准 `.env` 格式：

```ini
M365_ACCESS_TOKEN=eyJhbGciOiJkaXIiLCJlbmMiOiJBMjU2...
M365_TIME_ZONE=Asia/Tokyo
M365_OID=00000000-0000-0000-0000-000000000000
M365_TID=00000000-0000-0000-0000-000000000000
```

### Token 跨行支持

Token 值可能很长。`.env` 解析支持续行格式——如果某行不以 `=` 或续行标记开头，则自动作为上一行的延续拼接：

```ini
M365_ACCESS_TOKEN=eyJhbGciOiJkaXIiLCJlbmMiOiJBMjU2...
...NiIsImtpZCI6IlhYMjAyNTAzM...
...V0In0.eyJleHAiOjE3NDg5ND...
```

注意：这种方式不需要续行符，只要不以 `=` 开头的行都会被追加到上一行的值中。但推荐使用单行值以防意外。

### 重要安全提示

- `.env` 包含敏感的 `M365_ACCESS_TOKEN`，**绝不**提交到版本控制系统
- 已通过 `.gitignore` 排除 `.env`
- Token 有效期约 1 小时（JWT），过期后需要重新获取

## Token 持久化行为

```
Token 获取方式          ├── set-token (交互输入)
                       │     └── 写入 .env → 热重载
                       │
                       └── capture-token (CDP)
                             └── 写入 .env → 热重载
```

Token 生命周期管理：

1. **启动时加载**：读取 `.env` 中的 `M365_ACCESS_TOKEN`
2. **运行时热重载**：每次 `get()` 检查 `.env` 文件修改时间
3. **CDP 自动刷新**（需 `--auto-capture`）：
   - 启动时 Token 无效 → 自动捕获
   - 运行中 JWT Token 过期前 5 分钟 → 自动捕获

## DEBUG 模式

启用详细日志：

```powershell
$env:RUST_LOG = "debug"
cargo run -- serve
```

`debug` 级别包含：
- WebSocket 连接建立/断开事件
- SignalR 消息帧的原始内容（前 1000 字符）
- Token 验证结果和剩余有效期

`trace` 级别额外包含：
- 完整的 SignalR 消息 JSON
- WebSocket 二进制帧的十六进制转储
- HTTP 请求和响应的完整头

## 多实例配置

如需运行多个代理实例：

```powershell
# 实例 1
$env:M365_ACCESS_TOKEN = "token1"
cargo run -- serve --port 8000

# 实例 2（需要不同的 .env 或手动设置环境变量）
$env:M365_ACCESS_TOKEN = "token2"
cargo run -- serve --port 8001
```

每个实例需要独立的 M365 Copilot 会话和 Token。

## 环境变量校验

程序启动时校验：

1. `M365_ACCESS_TOKEN` 为空 → panic 退出并提示 "ACCESS_TOKEN is required"
2. Token 格式不是有效 JWT/JWE → 日志警告但不阻塞
3. `M365_OID` / `M365_TID` 为空 → 使用默认全零 UUID

## 配置示例

### 最小配置（仅 Token）

```ini
M365_ACCESS_TOKEN=eyJ...
```

### 完整配置

```ini
M365_ACCESS_TOKEN=eyJ...
M365_TIME_ZONE=America/New_York
M365_MODEL_ALIAS=gpt-4
M365_OID=12345678-1234-1234-1234-123456789abc
M365_TID=87654321-4321-4321-4321-cba987654321
RUST_LOG=info
```
