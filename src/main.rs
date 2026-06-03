use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};

use tracing_subscriber::EnvFilter;

use crate::app::{create_router, AppState};
use crate::config::Settings;
use crate::session_store::PersistentSessionStore;
use crate::token_store::AccessTokenStore;

mod app;
mod cdp;
mod config;
mod models;
mod session_store;
mod signalr;
mod substrate;
mod token_store;
mod tools;
mod translator;

#[derive(Parser)]
#[command(name = "copilot-openai-proxy", about = "OpenAI-compatible shim for Microsoft 365 Copilot Chat")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Paste a WebSocket URL or access_token to update .env
    SetToken,
    /// Capture a fresh token from headless Edge (no visible window)
    CaptureToken {
        #[arg(long, default_value_t = 9222)]
        cdp_port: u16,
        #[arg(long, default_value_t = 90)]
        timeout_seconds: u64,
    },
    /// Launch Edge with remote debugging (visible window, for first-time sign-in)
    LaunchEdge {
        #[arg(long, default_value_t = 9222)]
        cdp_port: u16,
    },
    /// Start the HTTP proxy server
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8000)]
        port: u16,
        /// Launch Edge with remote debugging on start
        #[arg(long)]
        launch_edge: bool,
        /// Automatically capture token on startup if missing/expired
        #[arg(long)]
        auto_capture: bool,
        /// CDP port for auto-capture
        #[arg(long, default_value_t = 9222)]
        cdp_port: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::SetToken => {
            set_token_command().await?;
        }
        Commands::CaptureToken {
            cdp_port,
            timeout_seconds,
        } => {
            capture_token_command(cdp_port, timeout_seconds).await?;
        }
        Commands::LaunchEdge { cdp_port } => {
            launch_edge_command(cdp_port);
        }
        Commands::Serve {
            host,
            port,
            launch_edge,
            auto_capture,
            cdp_port,
        } => {
            if launch_edge {
                launch_edge_command(cdp_port);
            }
            serve_command(&host, port, auto_capture, cdp_port).await?;
        }
    }

    Ok(())
}

async fn set_token_command() -> anyhow::Result<()> {
    println!("Paste the full WebSocket URL (or just the access_token value), then press Enter:");
    let mut raw = String::new();
    std::io::stdin().read_line(&mut raw)?;
    let raw = raw.trim();

    let token = if let Some(caps) = regex::Regex::new(r"access_token=([^&\s]+)")?.captures(raw) {
        caps.get(1).unwrap().as_str().to_owned()
    } else {
        raw.to_owned()
    };

    std::fs::write(".env", format!("M365_ACCESS_TOKEN={token}\n"))?;
    println!(".env updated.");
    Ok(())
}

async fn capture_token_command(cdp_port: u16, timeout_seconds: u64) -> anyhow::Result<()> {
    println!("Starting headless Edge to capture token (no visible window)...");
    match cdp::capture_token(cdp_port, timeout_seconds).await {
        Ok(token) => {
            std::fs::write(".env", format!("M365_ACCESS_TOKEN={token}\n"))?;
            println!(".env updated with fresh token.");
            Ok(())
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

fn launch_edge_command(cdp_port: u16) {
    let profile = cdp::profile_dir();
    std::fs::create_dir_all(&profile).ok();
    match cdp::launch_edge_visible(&profile, cdp_port) {
        Some(_) => {
            println!("Edge launched with remote debugging on port {cdp_port}.");
            println!("Dedicated Edge profile: {}", profile.display());
            println!("Sign in to M365 Copilot once; subsequent token captures will reuse this profile.");
        }
        None => {
            eprintln!("Failed to launch Edge. Make sure Microsoft Edge is installed.");
        }
    }
}

fn token_needs_refresh(token: &str) -> bool {
    if token.is_empty() {
        return true;
    }
    // If we can decode it as JWT, check expiry
    if let Ok(claims) = crate::token_store::decode_jwt_payload(token) {
        if !crate::token_store::is_substrate_token_claims(&claims) {
            return true;
        }
        if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            // Refresh if less than 5 minutes remaining
            return exp - now < 300;
        }
    }
    // JWE token: assume still valid (can't check expiry)
    false
}

async fn serve_command(
    host: &str,
    port: u16,
    auto_capture: bool,
    cdp_port: u16,
) -> anyhow::Result<()> {
    let settings = Settings::from_env();
    let token = settings.access_token.clone();
    let env_path = settings.env_path.clone();
    let token_store = Arc::new(AccessTokenStore::new(token, &env_path));
    let session_store = Arc::new(std::sync::RwLock::new(PersistentSessionStore::new()));

    let state = AppState {
        settings,
        token_store: token_store.clone(),
        session_store,
    };

    // Auto-capture on startup if needed
    if auto_capture {
        let ts = token_store.clone();
        let ep = env_path.clone();
        tokio::spawn(async move {
            // Brief delay to let the server start
            tokio::time::sleep(Duration::from_secs(2)).await;
            let current = ts.get();
            if token_needs_refresh(&current) {
                println!("Token missing or expiring soon; capturing fresh token...");
                match cdp::capture_token(cdp_port, 90).await {
                    Ok(new_token) => {
                        if let Err(e) = std::fs::write(&ep, format!("M365_ACCESS_TOKEN={new_token}\n")) {
                            eprintln!("Failed to write .env: {e}");
                        } else {
                            println!("Fresh token captured and written to .env.");
                        }
                    }
                    Err(e) => {
                        println!("Token capture failed: {e}");
                    }
                }
            }
        });
    }

    // Background auto-refresh every 60 seconds
    let ts2 = token_store.clone();
    let ep2 = env_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let current = ts2.get();
            if token_needs_refresh(&current) {
                println!("Token expiring soon; refreshing...");
                match cdp::capture_token(cdp_port, 90).await {
                    Ok(new_token) => {
                        if let Err(e) = std::fs::write(&ep2, format!("M365_ACCESS_TOKEN={new_token}\n")) {
                            eprintln!("Failed to write .env during refresh: {e}");
                        } else {
                            println!("Token refreshed.");
                        }
                    }
                    Err(e) => {
                        eprintln!("Auto-refresh failed: {e}");
                    }
                }
            }
        }
    });

    let app = create_router(state);
    let addr = format!("{host}:{port}");
    tracing::info!("Listening on {addr}");

    if auto_capture {
        println!("Auto-capture is enabled. A headless Edge will attempt to capture a token on startup if needed.");
    }

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
