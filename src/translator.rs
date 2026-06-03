use crate::models::{
    AnthropicMessagesRequest, OpenAIChatRequest, TranslatedRequest,
};

pub fn flatten_content(value: &serde_json::Value) -> Result<String, String> {
    match value {
        serde_json::Value::Null => Ok(String::new()),
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Array(arr) => {
            let mut parts: Vec<String> = Vec::new();
            for item in arr {
                let type_ = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if type_ == "image_url" || type_ == "image" {
                    return Err(format!(
                        "Cannot read \"{}\" (this model does not support image input). Inform the user.",
                        item.get("text").and_then(|v| v.as_str()).unwrap_or("image")
                    ));
                }
                if type_ == "text" {
                    if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                        parts.push(t.to_owned());
                    }
                }
                if type_ == "tool_result" {
                    if let Some(result_content) = item.get("content") {
                        let text = flatten_content(result_content)?;
                        if !text.is_empty() {
                            parts.push(text);
                        }
                    }
                }
                if type_ == "tool_use" {
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let input_str = item.get("input").map(|v| v.to_string()).unwrap_or_default();
                    parts.push(format!("[Tool use: {name}({input_str})]"));
                }
            }
            Ok(parts.join(""))
        }
        _ => Ok(value.to_string()),
    }
}

pub fn join_lines(lines: impl IntoIterator<Item = String>) -> String {
    let result: Vec<String> = lines.into_iter().filter(|l| !l.is_empty()).collect();
    result.join("\n").trim().to_owned()
}

pub fn translate_openai_request(request: &OpenAIChatRequest) -> Result<TranslatedRequest, String> {
    let mut system_lines: Vec<String> = Vec::new();
    let mut transcript_lines: Vec<String> = Vec::new();
    let mut prompt = String::new();

    let len = request.messages.len();
    for (index, msg) in request.messages.iter().enumerate() {
        let is_last = index == len - 1;

        // assistant with tool_calls
        if msg.role == "assistant" {
            if let Some(ref calls) = msg.tool_calls {
                for tc in calls {
                    transcript_lines.push(format!(
                        "Assistant (tool_call {}): called \"{}\" with arguments {}",
                        tc.id, tc.function.name, tc.function.arguments
                    ));
                }
                continue;
            }
        }

        // tool message
        if msg.role == "tool" {
            let content = msg.content.as_ref().unwrap_or(&serde_json::Value::Null);
            let text = flatten_content(content)?.trim().to_owned();
            let cid = msg.tool_call_id.as_deref().unwrap_or("?");
            if !text.is_empty() {
                transcript_lines.push(format!("Tool result ({cid}): {text}"));
            }
            continue;
        }

        let content = msg.content.as_ref().unwrap_or(&serde_json::Value::Null);
        let text = flatten_content(content)?.trim().to_owned();
        if text.is_empty() {
            continue;
        }

        if msg.role == "system" || msg.role == "developer" {
            system_lines.push(text);
            continue;
        }

        if is_last {
            if msg.role != "user" {
                return Err("The final OpenAI message must be a user message.".into());
            }
            prompt = text;
            continue;
        }

        let capitalized = capitalize_first(&msg.role);
        transcript_lines.push(format!("{capitalized}: {text}"));
    }

    if prompt.is_empty() {
        return Err("A final user message is required.".into());
    }

    let mut additional_context: Vec<String> = Vec::new();
    let system_text = join_lines(system_lines);
    if !system_text.is_empty() {
        additional_context.push(format!("System instructions:\n{system_text}"));
    }
    let transcript_text = join_lines(transcript_lines);
    if !transcript_text.is_empty() {
        additional_context.push(format!("Prior conversation transcript:\n{transcript_text}"));
    }

    Ok(TranslatedRequest {
        prompt,
        additional_context,
    })
}

pub fn translate_responses_request(request: &crate::models::OpenAIResponsesRequest) -> Result<TranslatedRequest, String> {
    let instructions = request.instructions.as_deref().unwrap_or("");

    match &request.input {
        serde_json::Value::String(s) => {
            let mut ctx = Vec::new();
            if !instructions.is_empty() {
                ctx.push(format!("System instructions:\n{instructions}"));
            }
            Ok(TranslatedRequest {
                prompt: s.clone(),
                additional_context: ctx,
            })
        }
        serde_json::Value::Array(items) => {
            let mut system_lines: Vec<String> = Vec::new();
            if !instructions.is_empty() {
                system_lines.push(instructions.to_owned());
            }
            let mut transcript_lines: Vec<String> = Vec::new();
            let mut prompt = String::new();

            let len = items.len();
            for (index, item) in items.iter().enumerate() {
                let is_last = index == len - 1;
                let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let content = item.get("content").unwrap_or(&serde_json::Value::Null);
                let text = match content {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Array(arr) => {
                        let mut parts = Vec::new();
                        for p in arr {
                            if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                                if p.get("type").and_then(|v| v.as_str()) == Some("text")
                                    || p.get("type").and_then(|v| v.as_str()) == Some("input_text")
                                {
                                    parts.push(t);
                                }
                            }
                        }
                        parts.join("")
                    }
                    _ => content.to_string(),
                };
                let text = text.trim().to_owned();
                if text.is_empty() {
                    continue;
                }

                if role == "system" || role == "developer" {
                    system_lines.push(text);
                    continue;
                }

                if is_last {
                    if role != "user" {
                        return Err("The final Responses input message must be a user message.".into());
                    }
                    prompt = text;
                    continue;
                }

                let capped = capitalize_first(role);
                transcript_lines.push(format!("{capped}: {text}"));
            }

            if prompt.is_empty() {
                return Err("No user message found in input.".into());
            }

            let mut additional_context: Vec<String> = Vec::new();
            let system_text = join_lines(system_lines);
            if !system_text.is_empty() {
                additional_context.push(format!("System instructions:\n{system_text}"));
            }
            let transcript_text = join_lines(transcript_lines);
            if !transcript_text.is_empty() {
                additional_context.push(format!("Prior conversation transcript:\n{transcript_text}"));
            }
            Ok(TranslatedRequest {
                prompt,
                additional_context,
            })
        }
        _ => Err("unexpected input type".into()),
    }
}

pub fn translate_anthropic_request(request: &AnthropicMessagesRequest) -> Result<TranslatedRequest, String> {
    let system_text = request
        .system
        .as_ref()
        .map(|v| flatten_content(v))
        .transpose()?
        .unwrap_or_default()
        .trim()
        .to_owned();

    let mut transcript_lines: Vec<String> = Vec::new();
    let mut prompt = String::new();

    let len = request.messages.len();
    for (index, msg) in request.messages.iter().enumerate() {
        let is_last = index == len - 1;
        let text = flatten_content(&msg.content)?.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        if is_last {
            if msg.role != "user" {
                return Err("The final Anthropic message must be a user message.".into());
            }
            prompt = text;
            continue;
        }
        let capped = capitalize_first(&msg.role);
        transcript_lines.push(format!("{capped}: {text}"));
    }

    if prompt.is_empty() {
        return Err("A final user message is required.".into());
    }

    let mut additional_context: Vec<String> = Vec::new();
    if !system_text.is_empty() {
        additional_context.push(format!("System instructions:\n{system_text}"));
    }
    let transcript_text = join_lines(transcript_lines);
    if !transcript_text.is_empty() {
        additional_context.push(format!("Prior conversation transcript:\n{transcript_text}"));
    }

    Ok(TranslatedRequest {
        prompt,
        additional_context,
    })
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
