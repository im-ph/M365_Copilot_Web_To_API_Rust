use std::{
    fs,
    path::Path,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use serde_json::Value;

pub const SUBSTRATE_AUDIENCE_PREFIX: &str = "https://substrate.office.com/";

fn b64_decode_json(s: &str) -> Result<Value, String> {
    let padded = format!("{}{}", s, "=".repeat((4 - s.len() % 4) % 4));
    let bytes = base64::engine::general_purpose::URL_SAFE
        .decode(padded.as_bytes())
        .map_err(|e| format!("base64: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("json: {e}"))
}

pub fn decode_jwt_payload(token: &str) -> Result<Value, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() == 5 {
        b64_decode_json(parts[0])
    } else if parts.len() == 3 {
        b64_decode_json(parts[1])
    } else {
        Err(format!("unexpected token parts: {}", parts.len()))
    }
}

pub fn is_jwe_token(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 5 {
        return false;
    }
    match b64_decode_json(parts[0]) {
        Ok(header) => header.get("enc").is_some(),
        Err(_) => false,
    }
}

pub fn is_substrate_token_claims(claims: &Value) -> bool {
    claims
        .get("aud")
        .and_then(|v| v.as_str())
        .map(|s| s.starts_with(SUBSTRATE_AUDIENCE_PREFIX))
        .unwrap_or(false)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_env_token(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut in_value = false;
    let mut parts: Vec<String> = Vec::new();
    for line in text.lines() {
        let stripped = line.trim().to_string();
        if !in_value {
            if stripped.is_empty() || stripped.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = stripped.split_once('=') {
                if key.trim() == "M365_ACCESS_TOKEN" {
                    let v = value.trim().trim_matches(|c| c == '\'' || c == '"');
                    parts.push(v.to_owned());
                    in_value = true;
                }
            }
        } else {
            if stripped.contains('=') || stripped.is_empty() || stripped.starts_with('#') {
                break;
            }
            parts.push(stripped);
        }
    }
    if parts.is_empty() { None } else { Some(parts.concat()) }
}

pub struct AccessTokenStore {
    token: Arc<RwLock<String>>,
    env_path: Arc<RwLock<String>>,
    mtime_ns: Arc<RwLock<u64>>,
}

impl AccessTokenStore {
    pub fn new(token: String, env_path: &Path) -> Self {
        let mtime = env_path
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64)
            .unwrap_or(0);
        Self {
            token: Arc::new(RwLock::new(token)),
            env_path: Arc::new(RwLock::new(env_path.to_string_lossy().to_string())),
            mtime_ns: Arc::new(RwLock::new(mtime)),
        }
    }

    pub fn get(&self) -> String {
        self.reload_if_changed();
        self.token.read().unwrap().clone()
    }

    pub fn status(&self) -> crate::models::TokenStatus {
        let token = self.get();
        if token.is_empty() {
            return crate::models::TokenStatus {
                valid: false,
                error: Some("no token configured".into()),
                expires_at: None,
                seconds_remaining: 0,
            };
        }
        if is_jwe_token(&token) {
            return crate::models::TokenStatus {
                valid: true,
                error: None,
                expires_at: None,
                seconds_remaining: -1,
            };
        }
        match decode_jwt_payload(&token) {
            Ok(claims) => {
                if !is_substrate_token_claims(&claims) {
                    return crate::models::TokenStatus {
                        valid: false,
                        error: Some("not a substrate token".into()),
                        expires_at: None,
                        seconds_remaining: 0,
                    };
                }
                let exp = claims.get("exp").and_then(|v| v.as_i64()).unwrap_or(0);
                let remaining = (exp - now() as i64).max(0);
                crate::models::TokenStatus {
                    valid: remaining > 0,
                    error: None,
                    expires_at: if remaining > 0 {
                        // ISO format not critical; just return the raw timestamp
                        Some(format!("{exp}"))
                    } else {
                        None
                    },
                    seconds_remaining: remaining,
                }
            }
            Err(e) => crate::models::TokenStatus {
                valid: false,
                error: Some(format!("cannot decode: {e}")),
                expires_at: None,
                seconds_remaining: 0,
            },
        }
    }

    fn reload_if_changed(&self) {
        let path_str = self.env_path.read().unwrap().clone();
        let path = Path::new(&path_str);
        let current_mtime = path
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64)
            .unwrap_or(0);
        if current_mtime == *self.mtime_ns.read().unwrap() {
            return;
        }
        if let Some(token) = read_env_token(path) {
            *self.token.write().unwrap() = token;
            *self.mtime_ns.write().unwrap() = current_mtime;
        }
    }
}
