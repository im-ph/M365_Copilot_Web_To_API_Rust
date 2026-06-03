use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------- ContentPart ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub type_: String,
    pub text: Option<String>,
}

// ---------- Tool calls within assistant messages ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

// ---------- Tool definitions sent by client ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub type_: String,
    pub function: FunctionDef,
}

// ---------- OpenAI /v1/chat/completions ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIChatRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    #[serde(default)]
    pub stream: bool,
    pub temperature: Option<f64>,
    pub user: Option<String>,
    pub tools: Option<Vec<ToolDef>>,
    pub tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatResponseMessage,
    pub finish_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatResponseMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub function: ToolCallFunction,
}

// ---------- OpenAI /v1/responses ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIResponsesRequest {
    pub model: String,
    pub input: serde_json::Value,
    pub instructions: Option<String>,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseOutputMessage {
    #[serde(rename = "type")]
    pub type_: String,
    pub id: String,
    pub role: String,
    pub content: Vec<ResponseContentPart>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseContentPart {
    #[serde(rename = "type")]
    pub type_: String,
    pub text: String,
}

// ---------- Anthropic /v1/messages ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessagesRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub system: Option<serde_json::Value>,
    #[serde(default)]
    pub stream: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
}

// ---------- Translated result ----------

#[derive(Debug, Clone)]
pub struct TranslatedRequest {
    pub prompt: String,
    pub additional_context: Vec<String>,
}

// ---------- Copilot WebSocket types ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotMessage {
    pub id: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attributions: Vec<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotConversation {
    pub id: String,
    #[serde(default)]
    pub messages: Vec<CopilotMessage>,
}

// ---------- Token status ----------

#[derive(Debug, Clone, Serialize)]
pub struct TokenStatus {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub expires_at: Option<String>,
    pub seconds_remaining: i64,
}
