use std::path::PathBuf;

#[derive(Clone)]
pub struct Settings {
    pub access_token: String,
    pub time_zone: String,
    pub model_alias: String,
    pub oid: String,
    pub tid: String,
    pub env_path: PathBuf,
}

impl Settings {
    pub fn from_env() -> Self {
        let env_path = PathBuf::from(".env");
        dotenvy::dotenv().ok();

        Self {
            access_token: std::env::var("M365_ACCESS_TOKEN").unwrap_or_default(),
            time_zone: std::env::var("M365_TIME_ZONE").unwrap_or_else(|_| "Asia/Tokyo".into()),
            model_alias: std::env::var("M365_MODEL_ALIAS").unwrap_or_else(|_| "m365-copilot".into()),
            oid: std::env::var("M365_OID").unwrap_or_else(|_| "00000000-0000-0000-0000-000000000000".into()),
            tid: std::env::var("M365_TID").unwrap_or_else(|_| "00000000-0000-0000-0000-000000000000".into()),
            env_path,
        }
    }
}
