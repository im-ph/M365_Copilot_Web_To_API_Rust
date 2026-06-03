use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use regex::Regex;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const WS_READ_TIMEOUT: Duration = Duration::from_millis(800);

fn find_edge() -> Option<PathBuf> {
    for path in [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ] {
        let p = Path::new(path);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    None
}

pub fn profile_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".m365-copilot-openai-proxy")
        .join("edge-profile")
}

pub fn launch_edge_visible(profile_dir: &Path, cdp_port: u16) -> Option<Child> {
    let path = find_edge()?;
    Command::new(path)
        .arg(format!("--remote-debugging-port={cdp_port}"))
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--no-first-run")
        .arg("https://m365.cloud.microsoft/chat")
        .spawn()
        .ok()
}

fn launch_edge_headless(profile_dir: &Path, cdp_port: u16) -> Option<Child> {
    let path = find_edge()?;
    Command::new(path)
        .arg(format!("--remote-debugging-port={cdp_port}"))
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--no-first-run")
        .arg("--headless=new")
        .arg("https://m365.cloud.microsoft/chat")
        .spawn()
        .ok()
}

async fn http_get(cdp_port: u16, path: &str) -> Result<String, String> {
    let addr = format!("127.0.0.1:{cdp_port}");
    let mut stream =
        TcpStream::connect(&addr).await.map_err(|e| format!("tcp: {e}"))?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{cdp_port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| format!("write: {e}"))?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("read: {e}"))?;
    let text = String::from_utf8_lossy(&buf);
    text.split("\r\n\r\n")
        .nth(1)
        .map(|s| s.to_owned())
        .ok_or_else(|| "no body".into())
}

async fn is_running(cdp_port: u16) -> bool {
    http_get(cdp_port, "/json/version").await.is_ok()
}

async fn wait_for_browser(cdp_port: u16, deadline: std::time::Instant) -> bool {
    while std::time::Instant::now() < deadline {
        if is_running(cdp_port).await {
            return true;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    false
}

async fn get_tabs(cdp_port: u16) -> Result<Vec<Value>, String> {
    let body = http_get(cdp_port, "/json").await?;
    serde_json::from_str(&body).map_err(|e| format!("json: {e}"))
}

fn find_m365(tabs: &[Value]) -> Option<Value> {
    tabs.iter()
        .find(|t| {
            t.get("url")
                .and_then(|u| u.as_str())
                .map(|u| u.starts_with("https://m365.cloud.microsoft/"))
                .unwrap_or(false)
        })
        .cloned()
}

async fn wait_for_page(cdp_port: u16, deadline: std::time::Instant) -> Option<Value> {
    while std::time::Instant::now() < deadline {
        if let Ok(tabs) = get_tabs(cdp_port).await {
            if let Some(tab) = find_m365(&tabs) {
                return Some(tab);
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    None
}

async fn cdp_send<S>(write: &mut S, id: u32, method: &str, params: Option<Value>) -> Result<(), String>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let msg = if let Some(p) = params {
        json!({"id": id, "method": method, "params": p})
    } else {
        json!({"id": id, "method": method})
    };
    write
        .send(Message::Text(msg.to_string()))
        .await
        .map_err(|e| format!("send: {e}"))
}

fn is_substrate_ws(url: &str) -> bool {
    url.contains("substrate.office.com") && url.contains("access_token=")
}

fn extract_token(url: &str) -> Option<String> {
    let re = Regex::new(r"[?&]access_token=([^&]+)").ok()?;
    re.captures(url)?.get(1).map(|m| m.as_str().to_owned())
}

/// Capture a Substrate access token by launching a headless Edge.
///
/// 1. Launches msedge.exe --headless=new with a dedicated profile
/// 2. Navigates to M365 Copilot Chat
/// 3. Intercepts the `Network.webSocketCreated` event for substrate.office.com
/// 4. Extracts the `access_token` from the WebSocket URL
/// 5. Kills the headless Edge and returns the token
pub async fn capture_token(
    cdp_port: u16,
    timeout_secs: u64,
) -> Result<String, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    let profile = profile_dir();
    std::fs::create_dir_all(&profile).map_err(|e| format!("profile dir: {e}"))?;

    let mut guard: Option<Child> = None;

    if !is_running(cdp_port).await {
        let child = launch_edge_headless(&profile, cdp_port)
            .ok_or("Failed to launch Edge. Make sure Microsoft Edge is installed at the default path.")?;
        guard = Some(child);

        let wait_deadline = std::time::Instant::now() + Duration::from_secs(15);
        if !wait_for_browser(cdp_port, wait_deadline).await {
            kill(&mut guard);
            return Err("Edge did not respond on the CDP port in time.\n\
                       This may be the first run. Run `launch-edge` once to sign in to M365 Copilot."
                .into());
        }
    }

    let tab = wait_for_page(cdp_port, deadline)
        .await
        .ok_or_else(|| {
            kill(&mut guard);
            "No M365 Copilot page found.\n\
             The dedicated profile may not be signed in.\n\
             Run `launch-edge` once to sign in."
                .to_string()
        })?;

    let ws_url_str = tab
        .get("webSocketDebuggerUrl")
        .and_then(|u| u.as_str())
        .ok_or_else(|| {
            kill(&mut guard);
            "No debugger WebSocket URL"
        })?
        .to_string();

    let (ws, _) = connect_async(&ws_url_str).await.map_err(|e| {
        kill(&mut guard);
        format!("cdp ws: {e}")
    })?;
    let (mut tx, mut rx) = ws.split();

    cdp_send(&mut tx, 1, "Network.enable", None).await.map_err(|e| {
        kill(&mut guard);
        e
    })?;

    // Wait briefly for Network.enable to take effect, then reload to trigger a new WS
    tokio::time::sleep(Duration::from_millis(300)).await;
    cdp_send(&mut tx, 2, "Page.reload", Some(json!({"ignoreCache": true})))
        .await
        .map_err(|e| {
            kill(&mut guard);
            e
        })?;

    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(WS_READ_TIMEOUT, rx.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(msg) = serde_json::from_str::<Value>(&text) {
                    if msg.get("method")
                        .and_then(|m| m.as_str())
                        == Some("Network.webSocketCreated")
                    {
                        if let Some(url) = msg.pointer("/params/url").and_then(|u| u.as_str()) {
                            if is_substrate_ws(url) {
                                if let Some(token) = extract_token(url) {
                                    kill(&mut guard);
                                    return Ok(token);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    kill(&mut guard);
    Err("Timed out waiting for the Substrate WebSocket.\n\
         The dedicated profile may not be signed in.\n\
         Run `launch-edge` once, sign in to M365 Copilot, then retry."
        .into())
}

fn kill(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}
