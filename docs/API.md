# API 接口文档

## 概述

本代理兼容三种 API 格式：

| 端点 | API 类型 | 兼容客户端 |
|------|----------|-----------|
| `POST /v1/chat/completions` | OpenAI Chat Completions API | OpenCode、Continue、通用 OpenAI 客户端 |
| `POST /v1/messages` | Anthropic Messages API | Claude Code、Anthropic SDK |
| `POST /v1/responses` | OpenAI Responses API | OpenAI Responses SDK（预览） |
| `GET /health` | 健康检查 | 通用 |
| `GET /v1/models` | 模型列表 | 通用 OpenAI 客户端 |

所有请求和响应均使用 `Content-Type: application/json`。流式响应使用 `text/event-stream`。

---

## `POST /v1/chat/completions`

兼容 OpenAI Chat Completions 格式。

### 请求格式

```json
{
  "model": "m365-copilot",
  "messages": [
    {
      "role": "system",
      "content": "You are a helpful assistant."
    },
    {
      "role": "user",
      "content": "你好，请介绍一下你自己"
    }
  ],
  "stream": false,
  "temperature": 0.7,
  "max_tokens": 2048
}
```

### 参数说明

| 参数 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `model` | string | 是 | - | 模型标识符。以 `:persist` 后缀启用持久会话 |
| `messages` | array | 是 | - | 消息数组 |
| `messages[].role` | string | 是 | - | `system` / `user` / `assistant` / `tool` |
| `messages[].content` | string 或 array | 是 | - | 如果是数组，支持 `text`、`tool_result`、`tool_use` 类型 |
| `stream` | boolean | 否 | `false` | 是否启用 SSE 流式响应 |
| `temperature` | number | 否 | - | 传递给 M365 Copilot（效果因模型而异） |
| `max_tokens` | number | 否 | - | 生成最大 Token 数 |
| `tools` | array | 否 | - | 工具定义（见下方说明） |
| `user` | string | 否 | - | 可用于传递会话 ID 前缀 |

### `content` 数组格式

支持的类型：

```json
// 纯文本
{"type": "text", "text": "Hello"}

// 工具调用结果（递归提取文本作为上下文）
{"type": "tool_result", "content": [{"type": "text", "text": "文件内容..."}]}

// 工具调用引用（格式化为描述文本）
{"type": "tool_use", "name": "bash", "input": {"command": "ls"}}
```

### 非流式响应

```json
{
  "id": "chatcmpl-550e8400-e29b-41d4-a716-446655440000",
  "object": "chat.completion",
  "created": 1718000000,
  "model": "m365-copilot",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "我是 Microsoft 365 Copilot..."
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 42,
    "completion_tokens": 128,
    "total_tokens": 170
  }
}
```

### 流式响应 (SSE)

每行格式：`data: {json}\n\n`

```
data: {"choices":[{"index":0,"delta":{"role":"assistant"}}]}

data: {"choices":[{"index":0,"delta":{"content":"我是"}}]}

data: {"choices":[{"index":0,"delta":{"content":" Microsoft"}}]}

data: {"choices":[{"index":0,"delta":{"content":" 365 Copilot"}}]}

data: {"choices":[{"index":0,"delta":{}},"finish_reason":"stop"}]}

data: [DONE]
```

### 工具调用（Tools）

M365 Copilot 原生不支持自定义工具调用。本代理通过模式匹配在本地模拟：

```json
{
  "messages": [{"role": "user", "content": "计算 1+1 等于多少？"}],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "calculator",
        "description": "执行数学计算",
        "parameters": {
          "type": "object",
          "properties": {
            "expr": {"type": "string"}
          },
          "required": ["expr"]
        }
      }
    }
  ]
}
```

如果用户消息内容不匹配任何工具模式，工具参数将被忽略，消息正常发送给 M365 Copilot 答复。

---

## `POST /v1/messages`

兼容 Anthropic Messages API。

### 请求格式

```json
{
  "model": "m365-copilot",
  "messages": [
    {
      "role": "user",
      "content": "你好，请介绍一下你自己"
    }
  ],
  "system": "You are a helpful assistant.",
  "stream": false,
  "max_tokens": 2048
}
```

### 参数说明

| 参数 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `model` | string | 是 | - | 模型标识符 |
| `messages` | array | 是 | - | 消息数组，与 Anthropic 格式兼容 |
| `messages[].role` | string | 是 | - | `user` / `assistant` |
| `messages[].content` | string 或 array | 是 | - | 支持 `[text]`、`[tool_result]`、`[tool_use]` 数组 |
| `system` | string | 否 | - | 系统提示 |
| `stream` | boolean | 否 | `false` | 启用 SSE 流式响应 |
| `max_tokens` | number | 否 | 4096 | 最大 Token 数 |
| `metadata` | object | 否 | - | 用户 ID（可选） |

### 非流式响应

```json
{
  "id": "msg_550e8400e29b41d4a716446655440000",
  "type": "message",
  "role": "assistant",
  "content": [
    {
      "type": "text",
      "text": "我是 Microsoft 365 Copilot..."
    }
  ],
  "model": "m365-copilot",
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "usage": {
    "input_tokens": 42,
    "output_tokens": 128
  }
}
```

### 流式响应 (SSE)

```
event: message_start
data: {"type":"message_start","message":{"id":"...","type":"message","role":"assistant","content":[],"model":"m365-copilot","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":42,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: ping
data: {"type":"ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"我是"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" Microsoft 365 Copilot"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":128}}

event: message_stop
data: {"type":"message_stop"}
```

### SSE 事件序列

| 顺序 | 事件 | 说明 |
|------|------|------|
| 1 | `message_start` | 消息开始，包含消息元数据 |
| 2 | `content_block_start` | 内容块开始（当前固定 index=0） |
| 3 | `ping` | 心跳（可重复多次） |
| 4 | `content_block_delta` | 增量文本（可重复多次，每次一个片段） |
| 5 | `content_block_stop` | 内容块结束 |
| 6 | `message_delta` | 消息增量（stop_reason、usage） |
| 7 | `message_stop` | 消息结束 |

注意：M365 Copilot 只返回纯文本内容块。`tool_use` 等类型不会被返回，因为 M365 Copilot 不支持。

---

## `POST /v1/responses`

兼容 OpenAI Responses API（预览格式，支持 SSE 流式）。

### 请求格式

```json
{
  "model": "m365-copilot",
  "input": [
    {
      "role": "user",
      "content": [{"type": "input_text", "text": "你好"}]
    }
  ],
  "stream": false,
  "max_output_tokens": 2048
}
```

### 流式响应 SSE 事件

```
data: {"type":"response.created","response":{"id":"...","object":"response","status":"completed","model":"m365-copilot","usage":{...}}}

data: {"type":"response.output_item.added","item":{"id":"...","type":"message","role":"assistant","content":[]}}

data: {"type":"response.content_part.added","part":{"type":"output_text","text":""}}

data: {"type":"response.output_text.delta","delta":"你好","index":0}

data: {"type":"response.output_text.delta","delta":"！我是","index":0}

data: {"type":"response.output_text.delta","delta":" Microsoft 365 Copilot","index":0}

data: {"type":"response.output_text.done","text":"你好！我是 Microsoft 365 Copilot","index":0}

data: {"type":"response.output_item.done","item":{...}}

data: {"type":"response.completed","response":{...}}
```

---

## `GET /health`

基础健康检查。

### 响应

```json
{
  "status": "ok",
  "timestamp": "2025-06-03T12:00:00Z"
}
```

HTTP 状态码：`200 OK`

---

## `GET /v1/models`

返回可用模型列表，兼容 OpenAI 格式。

### 响应

```json
{
  "object": "list",
  "data": [
    {
      "id": "m365-copilot",
      "object": "model",
      "created": 1718000000,
      "owned_by": "m365-copilot-openai-proxy"
    }
  ]
}
```

---

## 请求头

| 请求头 | 说明 |
|--------|------|
| `Authorization: Bearer <任意值>` | API Key（本代理不校验，仅作为占位符） |
| `Content-Type: application/json` | 请求体格式（必需） |
| `X-M365-Session-Id` | （可选）指定持久会话 ID |
| `X-M365-Stream-Delay` | （可选）流式发送时在每个 delta 之间添加的延迟毫秒数 |

## 响应头

| 响应头 | 说明 |
|--------|------|
| `Content-Type: application/json` | 非流式响应 |
| `Content-Type: text/event-stream` | 流式响应 |
| `X-Request-Id` | 每个请求的唯一标识 |
| `X-M365-Token-Expires-In` | 当前 Token 剩余有效秒数（仅对 JWT 类型） |

## 错误响应

### 格式

```json
{
  "error": {
    "message": "Error description",
    "type": "server_error",
    "code": 500
  }
}
```

### 常见错误码

| HTTP 状态码 | 类型 | 常见原因 |
|-------------|------|---------|
| 400 | `bad_request` | JSON 解析失败、请求格式无效 |
| 401 | `auth_error` | Token 缺失或无效 |
| 502 | `upstream_error` | M365 Copilot 后端返回错误或超时 |
| 504 | `timeout` | 请求超过 120 秒未完成 |

---

## 实际客户端行为差异

### OpenCode

- 使用 `POST /v1/chat/completions` 端点
- 发送 `stream: true` 进行流式请求
- 非流式请求也正常工作
- model 参数固定为 `m365-copilot`

### Claude Code

- 使用 `POST /v1/messages` 端点
- 总以流式请求（不发送 `stream: false` 的非流式请求）
- 会发送 `metadata` 字段用于用户 ID 标识
- 注意：不支持 tool use，Claude Code 的部分功能（文件读写、命令执行等）受限

### Continue

- 使用 `POST /v1/chat/completions` 端点
- 支持非流式和流式
- 在 `config.json` 中配置 `apiBase` 为本代理地址
