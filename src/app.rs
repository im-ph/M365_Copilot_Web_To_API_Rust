use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::json;
use uuid::Uuid;

use crate::config::Settings;
use crate::models::*;
use crate::session_store::PersistentSessionStore;
use crate::substrate::SubstrateClient;
use crate::token_store::AccessTokenStore;
use crate::translator::{translate_anthropic_request, translate_openai_request, translate_responses_request};
use crate::tools::detect_tool_call;

const PERSIST_MODEL_SUFFIX: &str = ":persist";

#[derive(Clone)]
pub struct AppState {
    pub settings: Settings,
    pub token_store: Arc<AccessTokenStore>,
    pub session_store: Arc<RwLock<PersistentSessionStore>>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/token/status", get(token_status))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(openai_responses))
        .route("/v1/messages", post(anthropic_messages))
        .with_state(state)
}

async fn healthz(State(st): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "token": st.token_store.status(),
    }))
}

async fn token_status(State(st): State<AppState>) -> Json<TokenStatus> {
    Json(st.token_store.status())
}

async fn list_models() -> Json<serde_json::Value> {
    let aliases = [
        "m365-copilot",
        "m365-copilot:persist",
        "gpt-5.5-quick",
        "gpt-5.5-deep",
        "gpt-5.4-deep",
        "gpt-5.3-quick",
        "gpt-5.2-quick",
        "gpt-5.2-deep",
        "gpt-4o",
        "gpt-4o-mini",
        "gpt-4-turbo",
        "gpt-4",
        "gpt-3.5-turbo",
    ];
    let mut models: Vec<serde_json::Value> = Vec::new();
    for alias in &aliases {
        models.push(json!({
            "id": alias,
            "object": "model",
            "owned_by": "microsoft-365-copilot",
        }));
    }
    Json(json!({ "object": "list", "data": models }))
}

fn build_client(st: &AppState) -> Result<SubstrateClient, crate::substrate::SubstrateError> {
    let token = st.token_store.get();
    SubstrateClient::new(token, st.settings.time_zone.clone(), st.settings.oid.clone(), st.settings.tid.clone())
}

fn persistent_session(
    st: &AppState,
    model: &str,
    fallback_key: Option<&str>,
) -> Option<Arc<std::sync::Mutex<crate::session_store::PersistentSession>>> {
    if model.ends_with(PERSIST_MODEL_SUFFIX) {
        let key = fallback_key.unwrap_or("default");
        let session = st.session_store.write().unwrap().get(key);
        Some(Arc::new(std::sync::Mutex::new(session)))
    } else {
        None
    }
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

fn sse_response(stream: impl futures_util::Stream<Item = Result<String, std::convert::Infallible>> + Send + 'static) -> axum::response::Response {
    let body = axum::body::Body::from_stream(stream);
    let mut resp = axum::response::Response::new(body);
    resp.headers_mut().insert("content-type", "text/event-stream".parse().unwrap());
    resp
}

async fn chat_completions(
    State(st): State<AppState>,
    Json(req): Json<OpenAIChatRequest>,
) -> axum::response::Response {
    // Tool detection
    if let Some(ref tools) = req.tools {
        if let Some(last_msg) = req.messages.last() {
            if last_msg.role == "user" {
                if let Some(tc) = detect_tool_call(&last_msg.content.clone().unwrap_or(serde_json::Value::Null), tools) {
                    return Json(json!({
                        "id": format!("chatcmpl_{}", Uuid::new_v4().to_string().replace("-", "")),
                        "object": "chat.completion",
                        "created": now_secs(),
                        "model": st.settings.model_alias,
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": null,
                                "tool_calls": tc,
                            },
                            "finish_reason": "tool_calls",
                        }],
                    })).into_response();
                }
            }
        }
    }

    let translated = match translate_openai_request(&req) {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let session = persistent_session(&st, &req.model, req.user.as_deref());
    let client = match build_client(&st) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    if req.stream {
        let rx = match client.chat_stream(&translated, session).await {
            Ok(r) => r,
            Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
        };
        let completion_id = format!("chatcmpl_{}", Uuid::new_v4().to_string().replace("-", ""));
        let created = now_secs();
        let model_alias = st.settings.model_alias.clone();
        let role_chunk = format!(
            "data: {}\n\n",
            json!({"id": &completion_id, "object": "chat.completion.chunk", "created": created, "model": &model_alias, "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]})
        );
        let cid2 = completion_id.clone();
        let ma2 = model_alias.clone();
        let content_stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(move |chunk| {
            Ok::<_, std::convert::Infallible>(format!(
                "data: {}\n\n",
                json!({"id": &completion_id, "object": "chat.completion.chunk", "created": created, "model": &model_alias, "choices": [{"index": 0, "delta": {"content": chunk}, "finish_reason": null}]})
            ))
        });
        let done_chunk = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"id": &cid2, "object": "chat.completion.chunk", "created": created, "model": &ma2, "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]})
        );
        let s = tokio_stream::once(Ok::<_, std::convert::Infallible>(role_chunk))
            .chain(content_stream)
            .chain(tokio_stream::once(Ok::<_, std::convert::Infallible>(done_chunk)));
        return sse_response(s);
    }

    let text = match client.chat(&translated, session).await {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };

    Json(json!({
        "id": format!("chatcmpl_{}", Uuid::new_v4().to_string().replace("-", "")),
        "object": "chat.completion",
        "created": now_secs(),
        "model": st.settings.model_alias,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": text},
            "finish_reason": "stop",
        }],
    })).into_response()
}

async fn openai_responses(
    State(st): State<AppState>,
    Json(req): Json<OpenAIResponsesRequest>,
) -> axum::response::Response {
    let translated = match translate_responses_request(&req) {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let session = persistent_session(&st, &req.model, None);
    let client = match build_client(&st) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    if req.stream {
        let rx = match client.chat_stream(&translated, session).await {
            Ok(r) => r,
            Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
        };
        let resp_id = format!("resp_{}", Uuid::new_v4().to_string().replace("-", ""));
        let item_id = format!("msg_{}", Uuid::new_v4().to_string().replace("-", ""));
        let created = now_secs();
        let preamble = format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\n",
            json!({"type": "response.created", "response": {"id": &resp_id, "object": "response", "created_at": created, "model": st.settings.model_alias, "status": "in_progress", "output": []}}),
            json!({"type": "response.output_item.added", "output_index": 0, "item": {"id": &item_id, "type": "message", "role": "assistant", "content": []}}),
            json!({"type": "response.content_part.added", "item_id": &item_id, "output_index": 0, "content_index": 0, "part": {"type": "output_text", "text": ""}}),
        );
        let item_id2 = item_id.clone();
        let sse_stream = tokio_stream::once(Ok::<_, std::convert::Infallible>(preamble)).chain(
            tokio_stream::wrappers::ReceiverStream::new(rx).map(move |chunk| {
                Ok::<_, std::convert::Infallible>(format!(
                    "data: {}\n\n",
                    json!({"type": "response.output_text.delta", "item_id": &item_id2, "output_index": 0, "content_index": 0, "delta": chunk})
                ))
            }),
        );
        return sse_response(sse_stream);
    }

    let text = match client.chat(&translated, session).await {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };

    Json(json!({
        "id": format!("resp_{}", Uuid::new_v4().to_string().replace("-", "")),
        "object": "response",
        "created_at": now_secs(),
        "model": st.settings.model_alias,
        "output": [{
            "type": "message",
            "id": format!("msg_{}", Uuid::new_v4().to_string().replace("-", "")),
            "role": "assistant",
            "content": [{"type": "output_text", "text": text}],
        }],
        "usage": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0},
    })).into_response()
}

async fn anthropic_messages(
    State(st): State<AppState>,
    Json(req): Json<AnthropicMessagesRequest>,
) -> axum::response::Response {
    let translated = match translate_anthropic_request(&req) {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let session = persistent_session(&st, &req.model, None);
    let client = match build_client(&st) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    if req.stream {
        let rx = match client.chat_stream(&translated, session).await {
            Ok(r) => r,
            Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
        };
        let msg_id = format!("msg_{}", Uuid::new_v4().to_string().replace("-", ""));
        let model_alias = st.settings.model_alias.clone();
        let start = format!(
            "event: message_start\ndata: {}\n\nevent: content_block_start\ndata: {}\n\nevent: ping\ndata: {}\n\n",
            json!({"type": "message_start", "message": {"id": &msg_id, "type": "message", "role": "assistant", "content": [], "model": &model_alias, "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 0, "output_tokens": 0}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "ping"}),
        );
        let content_stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(move |chunk| {
            Ok::<_, std::convert::Infallible>(format!(
                "event: content_block_delta\ndata: {}\n\n",
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": chunk}})
            ))
        });
        let stop = format!(
            "event: content_block_stop\ndata: {}\n\nevent: message_delta\ndata: {}\n\nevent: message_stop\ndata: {}\n\n",
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": 0, "input_tokens": 0}}),
            json!({"type": "message_stop"}),
        );
        let stream = tokio_stream::once(Ok::<_, std::convert::Infallible>(start))
            .chain(content_stream)
            .chain(tokio_stream::once(Ok::<_, std::convert::Infallible>(stop)));
        return sse_response(stream);
    }

    let text = match client.chat(&translated, session).await {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };

    Json(json!({
        "id": format!("msg_{}", Uuid::new_v4().to_string().replace("-", "")),
        "type": "message",
        "role": "assistant",
        "model": st.settings.model_alias,
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {"input_tokens": 0, "output_tokens": 0},
    })).into_response()
}
