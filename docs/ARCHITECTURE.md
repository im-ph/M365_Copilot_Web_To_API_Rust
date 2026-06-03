# 架构详解

## 一、总体架构

本代理是一个位于客户端（如 OpenCode、Claude Code）和 Microsoft 365 Copilot 后端之间的 HTTP 中间层。其核心功能是协议翻译：将客户端发出的标准 OpenAI / Anthropic API 请求，翻译为 M365 Copilot 内部使用的 SignalR 协议，通过 WebSocket 发送到 `substrate.office.com`，再将响应翻译回标准 API 格式返回给客户端。

```
┌──────────────┐     HTTP/SSE (标准API)     ┌──────────────────┐
│  客户端       │ ◄─────────────────────────► │  M365 Copilot    │
│  OpenCode     │    /v1/chat/completions     │  OpenAI Proxy    │
│  Claude Code  │    /v1/messages             │  (Rust)          │
│  Continue     │    /v1/responses            │  :8000           │
│  自定义脚本    │                             │                  │
└──────────────┘                             └───────┬──────────┘
                                                      │
                                                      │ SignalR/WebSocket (wss://)
                                                      │ wss://substrate.office.com/m365Copilot/Chathub/
                                                      ▼
                                              ┌──────────────────┐
                                              │  M365 Copilot    │
                                              │  云服务           │
                                              │  substrate.office│
                                              │  .com            │
                                              └──────────────────┘
```

## 二、模块依赖关系

```
main.rs  (CLI 入口)
├── config.rs         从 .env 和环境变量加载配置
├── app.rs            Axum HTTP 服务器
│   ├── models.rs     所有 API 格式的数据结构
│   ├── translator.rs 协议翻译引擎
│   │   ├── translate_openai_request()     OpenAI Chat → 内部格式
│   │   ├── translate_responses_request()  OpenAI Responses → 内部格式
│   │   └── translate_anthropic_request()  Anthropic Messages → 内部格式
│   ├── substrate.rs  WebSocket SignalR 客户端
│   │   ├── ws_url()       构建 WebSocket URL
│   │   ├── chat_stream()  发起聊天 + 流式读取
│   │   ├── ws_loop()      WebSocket 消息循环
│   │   └── signalr.rs     SignalR 编解码工具
│   ├── token_store.rs Token 存储和热重载
│   ├── session_store.rs   持久会话管理
│   ├── tools.rs           工具调用检测
│   └── cdp.rs             CDP Token 捕获
└── token_store.rs     Token 验证函数（共享）
```

## 三、完整请求处理流程

### 3.1 非流式请求

```
[1] 客户端发送 HTTP POST 到 /v1/chat/completions
    Content-Type: application/json
    Body: {"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}]}

[2] Axum Json<OpenAIChatRequest> 反序列化
    - serde_json 解析请求体
    - 如果 content 是数组，递归提取 text 类型块
    - 如果含有 tool_result 类型块，提取其中的文本作为上下文

[3] translate_openai_request()
    - 提取最后一条 user 消息作为 prompt
    - 收集 system 消息、历史对话、tool 消息作为 additional_context
    - 返回 TranslatedRequest { prompt, additional_context }

[4] persistent_session()
    - 判断模型名是否以 :persist 结尾
    - 或者检查 X-M365-Session-Id 请求头
    - 如果有，从 session_store 获取或创建持久会话
    - 否则返回 None（无状态请求）

[5] build_client() → SubstrateClient::new(token, time_zone, oid, tid)
    - 从 token_store 获取当前有效的 access_token
    - 验证 token 格式（JWE 或 JWT）
    - JWT 验证 audience 是否为 https://substrate.office.com/

[6] client.chat_stream()
    ├── combine_text() 拼接 prompt 和 additional_context
    ├── 生成 conv_id / session_id / req_id (UUID v4)
    ├── 构建 WebSocket URL
    ├── 构建 SignalR chat invoke 消息
    ├── 创建 mpsc channel (capacity 64)
    └── tokio::spawn ws_loop() 异步执行

[7] ws_loop() — WebSocket 连接和消息循环
    ├── 构建 HTTP 请求，添加 Origin: https://m365.cloud.microsoft
    ├── connect_async() 建立 WSS 连接
    ├── 发送 negotiate: {"protocol":"json","version":1}\x1e
    ├── 读取 negotiate 响应（忽略内容）
    ├── 发送 chat invoke 消息
    └── 持续读取消息直到收到 type=3

[8] 响应读取循环
    ├── Text 帧 → 直接解析
    ├── Binary 帧 → from_utf8_lossy() 解码
    ├── SignalR 消息以 \x1e (Record Separator) 分隔
    ├── type=1, target="update" → 实时文本流
    │   ├── writeAtCursor → 发送到 mpsc channel
    │   └── messages → fallback 完整文本
    ├── type=2 → item.messages fallback 文本
    ├── type=3 → 流结束，返回
    └── type=6 → keepalive，跳过

[9] client.chat() 收集所有 chunks
    - 从 mpsc receiver 持续读取
    - 拼接所有字符串
    - 等待 channel 关闭（ws_loop 退出）

[10] 构建响应 JSON
    - OpenAI: {"choices":[{"message":{"content":"..."}}]}
    - 设置 id / created / model / finish_reason
    - Axum Json 序列化返回
```

### 3.2 流式请求

流程同 3.1 的第 [1-7] 步，区别在于第 [8-10] 步：

```
[8] ws_loop() 实时发送每个 writeAtCursor delta 到 mpsc channel

[9] app.rs 中的流式处理器：
    OpenAI Chat Completions:
      data: {"choices":[{"delta":{"role":"assistant"}}]}
      data: {"choices":[{"delta":{"content":"Hello"}}]}
      data: {"choices":[{"delta":{"content":" there"}}]}
      data: {"choices":[{"delta":{}}, "finish_reason":"stop"}]}
      data: [DONE]

    OpenAI Responses:
      data: {"type":"response.created",...}
      data: {"type":"response.output_item.added",...}
      data: {"type":"response.content_part.added",...}
      data: {"type":"response.output_text.delta","delta":"Hello"}
      ...

    Anthropic Messages:
      event: message_start
      event: content_block_start
      event: ping
      event: content_block_delta (多个)
      event: content_block_stop
      event: message_delta
      event: message_stop

[10] 通过 sse_response() 设置 Content-Type: text/event-stream 返回
```

## 四、SignalR 协议详解

### 4.1 协议基础

ASP.NET SignalR 协议运行在 WebSocket 之上。使用 `\x1e`（Record Separator）作为消息分隔符。协议类型在 negotiate 时指定为 JSON。

### 4.2 交互序列

```
Client → Server:

[1] negotiate 请求
    {"protocol":"json","version":1}\x1e

Server → Client:

[2] negotiate 响应
    {"negotiateVersion":1}\x1e

Client → Server:

[3] Chat Invoke
    {
      "type": 4,              ← Invocation 消息类型
      "target": "chat",       ← 目标 Hub 方法
      "invocationId": "0",
      "arguments": [{
        "source": "officeweb",
        "clientCorrelationId": "<uuid>",
        "sessionId": "<uuid>",
        "conversationId": "<uuid>",
        "isStartOfSession": true/false,
        "streamingMode": "ConciseWithPadding",
        "spokenTextMode": "None",
        "message": {
          "author": "user",
          "inputMethod": "Keyboard",
          "text": "<用户消息>",
          "locale": "en-us",
          "messageType": "Chat",
          "experienceType": "Default",
          "requestId": "<uuid>"
        },
        "optionsSets": [...],
        "allowedMessageTypes": [...],
        "plugins": [{"Id":"BingWebSearch","Source":"BuiltIn"}],
        "clientInfo": {
          "clientPlatform": "mcmcopilot-web",
          "clientAppName": "Office",
          "clientEntrypoint": "mcmcopilot-officeweb",
          "clientSessionId": "<uuid>",
          "clientAppType": "Web",
          "deviceOS": "Windows",
          "deviceType": "Desktop"
        },
        "tone": "Magic",
        "isSbsSupported": true,
        "renderReferencesBehindEOS": true
      }]
    }\x1e

Server → Client (持续):

[4] 多个更新消息
    {
      "type": 1,              ← StreamItem
      "target": "update",
      "arguments": [{
        "writeAtCursor": "生成的文本...",
        "messages": [{"author":"bot","text":"完整文本..."}],
        ...
      }]
    }\x1e

[5] 结束消息
    {
      "type": 3               ← Completion
    }\x1e

[6] Keepalive（定期）
    {
      "type": 6               ← KeepAlive
    }\x1e
```

### 4.3 消息类型

| type | 含义 | 处理方式 |
|------|------|---------|
| 1 | StreamItem（流式更新） | 提取 writeAtCursor 作为增量文本；保存 messages 作为后备 |
| 2 | 非流式结果 | 从 item.messages 提取最终文本 |
| 3 | Completion（完成） | 关闭连接；发送最后 fallback 文本（如果没有流式输出过） |
| 4 | Invocation（调用） | 由客户端发送，服务端不处理此类型 |
| 6 | KeepAlive（心跳） | 忽略，继续等待 |

## 五、Token 生命周期

### 5.1 Token 类型

两种 Token 格式：

1. **JWT**（JSON Web Token）
   - 三部分：`header.payload.signature`
   - 可解码查看过期时间（`exp` 字段）
   - aud：`https://substrate.office.com/`
   - 有效期约 1 小时

2. **JWE**（JSON Web Encryption）
   - 五部分：`header.iv.ciphertext.tag`
   - payload 加密，无法查看过期时间
   - 假设永久有效

### 5.2 Token 验证

```rust
decode_jwt_payload(token) → 解析 base64url 编码的 payload
  ├── 5 部分 → JWE，解码第 1 部分（header）
  └── 3 部分 → JWT，解码第 2 部分（payload）

is_substrate_token_claims(claims) → 检查 aud 前缀
  └── https://substrate.office.com/ 开头 → 有效
```

### 5.3 Token 存储

```
[内存] AccessTokenStore
  ├── token: Arc<RwLock<String>>       ← 当前 Token
  ├── env_path: Arc<RwLock<String>>    ← .env 文件路径
  └── mtime_ns: Arc<RwLock<u64>>       ← .env 文件修改时间戳

[磁盘] .env 文件
  └── M365_ACCESS_TOKEN=<token_value>
```

### 5.4 热重载机制

每次 `get()` 时检查 `.env` 文件的 mtime：

```rust
fn reload_if_changed(&self) {
    let current_mtime = path.metadata().modified()...;
    if current_mtime != last_mtime {
        let token = read_env_token(path);  // 重新读取 .env
        self.token = token;                // 更新内存
        self.mtime_ns = current_mtime;     // 更新记录
    }
}
```

支持多行 Continuation：Token 值若跨行（续行不以 `=` 开头），自动拼接。

### 5.5 Token 刷新

```
服务器启动
  │
  ├── auto_capture=true 且 Token 过期？
  │   └── CDP 无头浏览器捕获新 Token → 写入 .env → 热重载
  │
  └── 后台线程（每 60 秒）
      ├── 读取当前 Token
      ├── JWT? → 检查 exp - now < 300 秒？
      │   └── CDP 捕获 → 写入 .env
      └── JWE? → 假设有效，不刷新
```

## 六、CDP Token 捕获流程

见 [CDP.md](CDP.md) 的详细说明。核心步骤：

```
capture_token()
  ├── launch_edge_headless()
  │   ├── 查找 msedge.exe
  │   ├── 参数：--headless=new --remote-debugging-port=9222
  │   ├── 专用用户数据目录
  │   └── 导航到 https://m365.cloud.microsoft/chat
  │
  ├── wait_for_browser()    等待 CDP 端口可用
  ├── wait_for_page()       等待 M365 Copilot 标签页
  ├── connect CDP WebSocket
  ├── Network.enable
  ├── Page.reload
  ├── 监听 Network.webSocketCreated
  │   ├── URL 包含 substrate.office.com 且 access_token= → 提取
  │   └── 否则继续等待
  ├── kill Edge
  └── return token
```

## 七、中文/Unicode 支持

### 7.1 请求端编码

使用 `ensure_ascii_json()` 函数处理发送给 M365 Copilot 的 JSON 负载：

```rust
pub fn ensure_ascii_json(s: &str) -> String {
    // 将所有非 ASCII 字符转义为 \uXXXX
    // 中文 "你好" → "\u4f60\u597d"
}
```

目的：避免企业代理/中间设备对 WebSocket 文本帧中的原始 UTF-8 多字节序列进行修改。

### 7.2 响应端处理

- 从 M365 Copilot 接收的文本帧直接作为 UTF-8 字符串处理
- Binary 帧通过 `from_utf8_lossy()` 解码（无效字节替换为 U+FFFD）
- 转发给客户端时 Axum 自动设置 `Content-Type: application/json`（UTF-8）

## 八、会话管理

### 8.1 无状态模式（默认）

每次请求生成新的 `conversationId`，M365 Copilot 视为新对话。

### 8.2 持久会话（`:persist` 后缀）

```rust
fn persistent_session(st, model, fallback_key) {
    if model.ends_with(":persist") {
        let key = fallback_key.unwrap_or("default");
        // session_store 根据 key 查找或创建 PersistentSession
        Some(Arc::new(Mutex::new(session)))
    } else {
        None
    }
}
```

`PersistentSession` 结构：

```rust
pub struct PersistentSession {
    conversation_id: String,
    client_session_id: String,
    turn_number: u32,
    is_start_of_session: bool,
}

impl PersistentSession {
    pub fn reserve_turn() → TurnInfo {
        // conversation_id + session_id + turn_number + is_start_of_session
    }
}
```

### 8.3 会话标识方式

| 方式 | 示例 |
|------|------|
| 模型后缀 | `model: "m365-copilot:persist"` |
| HTTP 请求头 | `X-M365-Session-Id: my-session` |
| 用户字段 | `user: "my-session"`（OpenAI Chat 格式） |

## 九、Tool Call 处理

### 9.1 OpenAI 格式的工具调用检测

```rust
// 在 chat_completions 处理器中
if tools 参数存在 && 最后一条消息是 user 消息 {
    detect_tool_call(content, tools)
    // 如果匹配，直接返回 tool_calls 响应
}
```

`tools.rs` 中的 `detect_tool_call()` 会匹配特定模式并立即返回，**不**经过 M365 Copilot。

### 9.2 Anthropic 格式的上下文保留

`flatten_content()` 在处理消息内容时：

```rust
match type {
    "text"       → 提取 text 字段
    "tool_result" → 递归提取 content 中的文本
    "tool_use"   → 格式化为 "[Tool use: name(input)]"
    "image"      → 返回错误（不支持图片）
}
```

这样即使 M365 Copilot 不支持 tool use，之前工具调用返回的文件内容也会作为上下文传递给模型。

## 十、模块详细说明

### config.rs

```rust
pub struct Settings {
    access_token: String,    // M365_ACCESS_TOKEN
    time_zone: String,       // M365_TIME_ZONE (默认 Asia/Tokyo)
    model_alias: String,     // M365_MODEL_ALIAS (默认 m365-copilot)
    oid: String,             // M365_OID (默认全零 UUID)
    tid: String,             // M365_TID (默认全零 UUID)
    env_path: PathBuf,       // .env 路径
}
```

`from_env()` 先加载 `.env` 文件，再读取环境变量。

### signalr.rs

```rust
pub const SIGNALR_SEP: char = '\x1e';   // Record Separator

pub fn encode_message(msg: &str) → String    // msg + \x1e
pub fn decode_messages(raw: &str) → Vec<String> // split \x1e + trim + filter empty
pub fn ensure_ascii_json(s: &str) → String    // 非 ASCII 转 \uXXXX
```

### models.rs

定义了所有 API 格式的请求/响应结构体：

- OpenAI Chat: `OpenAIChatRequest`, `OpenAIMessage`, `ToolDef`, `FunctionDef`, `ChatCompletionResponse`, `ChatChoice`, `ChatResponseMessage`, `ToolCall`
- OpenAI Responses: `OpenAIResponsesRequest`, `ResponseInputItem`
- Anthropic: `AnthropicMessagesRequest`, `AnthropicMessage`
- 通用: `TranslatedRequest`, `TokenStatus`
