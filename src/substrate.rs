use std::str::FromStr;
use std::sync::Arc;
use url::Url;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use tokio_tungstenite::tungstenite::client::IntoClientRequest;


use crate::models::TranslatedRequest;
use crate::session_store::PersistentSession;
use crate::signalr::{self, SIGNALR_SEP};
use crate::token_store::{decode_jwt_payload, is_jwe_token, is_substrate_token_claims};

const WS_BASE: &str = "wss://substrate.office.com/m365Copilot/Chathub";

const VARIANTS: &str = concat!(
    "EnableMcpServerWidgets,feature.EnableMcpServerWidgets,feature.EnableLuForChatCIQ,",
    "feature.enableChatCIQPlugin,EnableRequestPlugins,feature.EnableSensitivityLabels,",
    "EnableUnsupportedUrlDetector,feature.IsCustomEngineCopilotEnabled,feature.bizchatfluxv3,",
    "feature.enablechatpages,feature.enableCodeCanvas,feature.turnOnWorkTabRecommendation,",
    "feature.turnOnDARecommendation,feature.IsStreamingModeInChatRequestEnabled,",
    "IncludeSourceAttributionsConcise,SkipPublishEmptyMessage,",
    "feature.EnableDeduplicatingSourceAttributions,Enable3PActionProgressMessages,",
    "feature.enableClientWebRtc,feature.EnableMeetingRecapOfSeriesMeetingWithCiq,",
    "feature.EnableReferencesListCompleteSignal,feature.StorageMessageSplitDisabled,",
    "feature.EnableCuaTakeControlApi,SingletonEnvOn,feature.cwcallowedos,",
    "feature.EnableMergingPureDeltas,feature.disabledisallowedmsgs,",
    "feature.enableCitationsForSynthesisData,feature.EnableConversationShareApis,",
    "feature.enableGenerateGraphicArtOptionsSet,cdximagen,",
    "feature.EnableUpdatedUXForConfirmationDialog,",
    "feature.EnableContentApiandDocTypeHtmlInRichAnswers,",
    "cdxgrounding_api_v2_rich_web_answers_reference_bottom_force,",
    "cdxenablerenderforisocomp,feature.EnableClientFileURLSupportForOfficeWebPaidCopilot,",
    "feature.EnableDesignEditorImageGrounding,feature.EnableDesignerEditor,",
    "feature.EnableSkipRehydrationForSpeCIdImages,feature.EnableSkipEmittingMessageOnFlush,",
    "feature.EnableRemoveEmptySourceAttributions,feature.EnableRemoveStreamingMode,",
    "feature.OfficeWebToHelix,feature.OfficeDesktopToHelix,feature.M365TeamsHubToHelix,",
    "feature.OwaHubToHelix,feature.MonarchHubToHelix,feature.Win32OutlookHubToHelix,",
    "feature.MacOutlookHubToHelix,Agt_bizchat_enableGpt5ForHelix,",
    "feature.EnablePersonalizationForMSA"
);

const OPTIONS_SETS: &[&str] = &[
    "search_result_progress_messages_with_search_queries",
    "cwc_flux_image",
    "cwc_code_interpreter",
    "cwc_code_interpreter_amsfix",
    "cwcfluxgptv",
    "flux_v3_gptv_enable_upload_multi_image_in_turn_wo_ch",
    "cwc_code_interpreter_citation_fix",
    "code_interpreter_interactive_charts",
    "cwc_code_interpreter_interactive_charts_inline_image",
    "code_interpreter_matplotlib_patching",
    "cwc_fileupload_odb",
    "update_memory_plugin",
    "add_custom_instructions",
    "cwc_flux_v3",
    "flux_v3_progress_messages",
    "enable_batch_token_processing",
    "enable_gg_gpt",
    "flux_v3_image_gen_enable_dimensions",
    "flux_v3_image_gen_enable_icon_dimensions",
    "flux_v3_image_gen_enable_system_text_with_params",
    "flux_v3_image_gen_enable_designer_dimensions_meta_prompting_in_system_prompts",
];

const ALLOWED_MESSAGE_TYPES: &[&str] = &[
    "Chat", "Suggestion", "InternalSearchQuery", "Disengaged",
    "InternalLoaderMessage", "Progress", "GeneratedCode", "RenderCardRequest",
    "AdsQuery", "SemanticSerp", "GenerateContentQuery", "GenerateGraphicArt",
    "SearchQuery", "ConfirmationCard", "AuthError", "DeveloperLogs",
    "TriggerPlugin", "HintInvocation", "MemoryUpdate", "EndOfRequest",
    "TriggerConfirmation", "ResumeInvokeAction", "ResumeUserInputRequest",
    "TriggerUserInputRequest", "EscapeHatch", "TriggerPluginAuth",
    "ResumePluginAuth", "SideBySide", "ReferencesListComplete",
    "SwitchRespondingEndpoint",
];

#[derive(Debug, thiserror::Error)]
pub enum SubstrateError {
    #[error("{0}")]
    Custom(String),
    #[error("websocket: {0}")]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("url: {0}")]
    Url(#[from] url::ParseError),
}

impl From<String> for SubstrateError {
    fn from(s: String) -> Self {
        SubstrateError::Custom(s)
    }
}

pub struct SubstrateClient {
    token: String,
    time_zone: String,
    oid: String,
    tid: String,
}

impl SubstrateClient {
    pub fn new(token: String, time_zone: String, oid: String, tid: String) -> Result<Self, SubstrateError> {
        if token.is_empty() {
            return Err(SubstrateError::Custom(
                "M365_ACCESS_TOKEN is missing".into(),
            ));
        }
        if !is_jwe_token(&token) {
            let claims = decode_jwt_payload(&token).map_err(|e| SubstrateError::Custom(e))?;
            if !is_substrate_token_claims(&claims) {
                return Err(SubstrateError::Custom("not a substrate token".into()));
            }
        }
        Ok(Self {
            token,
            time_zone,
            oid,
            tid,
        })
    }

    fn ws_url(&self, conv_id: &str, session_id: &str, req_id: &str) -> String {
        let encoded: String = url::form_urlencoded::byte_serialize(self.token.as_bytes()).collect();
        let base = format!("{}/{}@{}", WS_BASE, self.oid, self.tid);
        format!(
            "{base}?chatsessionid={session_id}&XRoutingParameterSessionKey={session_id}&clientrequestid={req_id}&X-SessionId={session_id}&ConversationId={conv_id}&access_token={encoded}&variants={variants}&source=\"officeweb\"&product=Office&agentHost=Bizchat.FullScreen&licenseType=Premium&isEdu=false&agent=web&scenario=OfficeWebPremiumConsumerCopilot",
            base = base,
            session_id = session_id,
            req_id = req_id,
            conv_id = conv_id,
            encoded = encoded,
            variants = VARIANTS,
        )
    }

    fn chat_invoke(
        &self,
        text: &str,
        conv_id: &str,
        session_id: &str,
        req_id: &str,
        is_start_of_session: bool,
    ) -> String {
        let payload = json!({
            "arguments": [{
                "source": "officeweb",
                "clientCorrelationId": req_id,
                "sessionId": session_id,
                "optionsSets": OPTIONS_SETS,
                "streamingMode": "ConciseWithPadding",
                "spokenTextMode": "None",
                "options": {},
                "extraExtensionParameters": {},
                "allowedMessageTypes": ALLOWED_MESSAGE_TYPES,
                "sliceIds": [],
                "threadLevelGptId": {},
                "traceId": req_id,
                "conversationId": conv_id,
                "isStartOfSession": is_start_of_session,
                "clientInfo": {
                    "clientPlatform": "mcmcopilot-web",
                    "clientAppName": "Office",
                    "clientEntrypoint": "mcmcopilot-officeweb",
                    "clientSessionId": session_id,
                    "clientAppType": "Web",
                    "deviceOS": "Windows",
                    "deviceType": "Desktop",
                },
                "message": {
                    "author": "user",
                    "inputMethod": "Keyboard",
                    "text": text,
                    "entityAnnotationTypes": ["People", "File", "Event", "Email", "TeamsMessage"],
                    "requestId": req_id,
                    "locationInfo": {"timeZoneOffset": 9, "timeZone": self.time_zone},
                    "locale": "en-us",
                    "messageType": "Chat",
                    "experienceType": "Default",
                    "adaptiveCards": [],
                    "clientPreferences": {},
                },
                "plugins": [{"Id": "BingWebSearch", "Source": "BuiltIn"}],
                "isSbsSupported": true,
                "tone": "Magic",
                "renderReferencesBehindEOS": true,
            }],
            "invocationId": "0",
            "target": "chat",
            "type": 4,
        });
        let payload_str = payload.to_string();
        let payload_ascii = signalr::ensure_ascii_json(&payload_str);
        format!("{}{}", payload_ascii, SIGNALR_SEP)
    }

    pub async fn chat_stream(
        &self,
        req: &TranslatedRequest,
        session: Option<Arc<std::sync::Mutex<PersistentSession>>>,
    ) -> Result<tokio::sync::mpsc::Receiver<String>, SubstrateError> {
        let text = combine_text(&req.prompt, &req.additional_context);
        let (conv_id, session_id, is_start) = if let Some(session_lock) = &session {
            let mut s = session_lock.lock().unwrap();
            let turn = s.reserve_turn();
            (turn.conversation_id, turn.client_session_id, turn.is_start_of_session)
        } else {
            (uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string(), true)
        };
        let req_id = uuid::Uuid::new_v4().to_string();
        let url = self.ws_url(&conv_id, &session_id, &req_id);
        let ws_url = Url::from_str(&url)?;

        let chat_invoke_msg = self.chat_invoke(&text, &conv_id, &session_id, &req_id, is_start);

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);

        tokio::spawn(async move {
            if let Err(e) = ws_loop(ws_url, chat_invoke_msg, tx).await {
                tracing::error!("ws_loop error: {e}");
            }
        });

        Ok(rx)
    }

    pub async fn chat(
        &self,
        req: &TranslatedRequest,
        session: Option<Arc<std::sync::Mutex<PersistentSession>>>,
    ) -> Result<String, SubstrateError> {
        let mut rx = self.chat_stream(req, session).await?;
        let mut result = String::new();
        while let Some(chunk) = rx.recv().await {
            result.push_str(&chunk);
        }
        Ok(result)
    }
}

async fn ws_loop(
    url: Url,
    chat_invoke_msg: String,
    tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), SubstrateError> {
    let mut request = url.as_str().into_client_request()?;
    request.headers_mut().insert("Origin", "https://m365.cloud.microsoft".parse().unwrap());
    let (mut ws, _) = connect_async(request).await?;

    // negotiate
    ws.send(Message::Text(format!("{}{}", json!({"protocol":"json","version":1}), SIGNALR_SEP))).await?;
    if let Some(Ok(msg)) = ws.next().await {
        let _ = msg;
    }

    // send chat invoke
    ws.send(Message::Text(chat_invoke_msg)).await?;

    // read loop
    let mut fallback_text = String::new();
    let mut yielded = false;

    while let Some(Ok(msg)) = ws.next().await {
        let raw = match msg {
            Message::Text(t) => t,
            Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
            _ => continue,
        };

        for part in signalr::decode_messages(&raw) {
            let m: Value = serde_json::from_str(&part)?;
            let t = m.get("type").and_then(|v| v.as_i64()).unwrap_or(0);

            if t == 6 {
                continue;
            }

            if t == 1 && m.get("target").and_then(|v| v.as_str()) == Some("update") {
                let args = m.get("arguments")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_object());

                if let Some(obj) = args {
                    if let Some(delta) = obj.get("writeAtCursor").and_then(|v| v.as_str()) {
                        if !delta.is_empty() {
                            if !yielded && !fallback_text.is_empty() {
                                if tx.send(fallback_text.clone()).await.is_err() { return Ok(()); }
                            }
                            yielded = true;
                            if tx.send(delta.to_owned()).await.is_err() { return Ok(()); }
                        }
                    }

                    if let Some(msgs) = obj.get("messages") {
                        let entries = if let Some(arr) = msgs.as_array() {
                            arr.clone()
                        } else {
                            vec![msgs.clone()]
                        };
                        for entry in entries.iter().rev() {
                            if entry.get("author").and_then(|v| v.as_str()) != Some("user") {
                                fallback_text = entry.get("text").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                                break;
                            }
                        }
                    }
                }
            }

            if t == 2 {
                if let Some(item_msgs) = m.pointer("/item/messages").and_then(|v| v.as_array()) {
                    for entry in item_msgs.iter().rev() {
                        if entry.get("author").and_then(|v| v.as_str()) != Some("user") {
                            fallback_text = entry.get("text").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                            break;
                        }
                    }
                }
            }

            if t == 3 {
                if !yielded && !fallback_text.is_empty() {
                    let _ = tx.send(fallback_text.clone()).await;
                }
                return Ok(());
            }
        }
    }

    Ok(())
}

pub fn combine_text(prompt: &str, context: &[String]) -> String {
    if context.is_empty() {
        return prompt.to_owned();
    }
    let mut parts = Vec::new();
    for c in context {
        parts.push(c.clone());
    }
    parts.push("---".into());
    parts.push(prompt.to_owned());
    parts.join("\n\n")
}
