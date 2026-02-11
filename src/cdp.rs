use anyhow::{Context, Result, anyhow};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::Message;

#[derive(Deserialize)]
#[allow(non_snake_case, dead_code)]
pub struct TargetJson {
    pub id: String,
    pub title: String,
    pub url: String,
    pub r#type: String,
    pub webSocketDebuggerUrl: Option<String>,
}

pub struct CdpConnection {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: i32,
}

impl CdpConnection {
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
        Ok(Self { ws, next_id: 1 })
    }

    pub async fn send(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({ "id": id, "method": method, "params": params });
        self.ws.send(Message::Text(msg.to_string())).await?;

        while let Some(msg) = self.ws.next().await {
            if let Ok(Message::Text(text)) = msg {
                let mut de = serde_json::Deserializer::from_str(&text);
                de.disable_recursion_limit();
                let resp = serde_json::Value::deserialize(&mut de)?;
                if resp.get("id") == Some(&serde_json::json!(id)) {
                    if let Some(error) = resp.get("error") {
                        return Err(anyhow!("CDP error: {}", error));
                    }
                    return Ok(resp.get("result").cloned().unwrap_or(serde_json::json!({})));
                }
            }
        }
        Err(anyhow!("No response from CDP"))
    }

    pub async fn eval(&mut self, expression: &str) -> Result<serde_json::Value> {
        let result = self
            .send(
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": expression,
                    "returnByValue": true
                }),
            )
            .await?;

        if let Some(value) = result.get("result").and_then(|r| r.get("value")) {
            Ok(value.clone())
        } else if let Some(desc) = result
            .get("result")
            .and_then(|r| r.get("description"))
            .and_then(|d| d.as_str())
        {
            Ok(serde_json::json!(desc))
        } else {
            Ok(serde_json::json!(null))
        }
    }
}

fn find_chrome_executable() -> Option<&'static str> {
    const CANDIDATES: &[&str] = &[
        "google-chrome-stable",
        "google-chrome",
        "chromium",
        "chromium-browser",
    ];
    for candidate in CANDIDATES {
        if std::process::Command::new("which")
            .arg(candidate)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    None
}

fn start_chrome(port: u16) -> Result<()> {
    let chrome = find_chrome_executable().context("Chrome not found in PATH")?;
    let data_dir = format!("/tmp/browser-cli-chrome-{}", port);
    std::process::Command::new(chrome)
        .arg(format!("--remote-debugging-port={}", port))
        .arg(format!("--user-data-dir={}", data_dir))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("about:blank")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to start Chrome")?;
    Ok(())
}

async fn chrome_is_running(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/json/version", port);
    reqwest::get(&url).await.is_ok()
}

async fn get_all_targets(port: u16) -> Result<Vec<TargetJson>> {
    let url = format!("http://127.0.0.1:{}/json", port);
    let targets: Vec<TargetJson> = reqwest::get(&url)
        .await
        .context("Failed to connect to Chrome")?
        .json()
        .await?;
    Ok(targets
        .into_iter()
        .filter(|t| t.r#type == "page" && t.webSocketDebuggerUrl.is_some())
        .collect())
}

pub async fn create_new_tab(port: u16, url: &str) -> Result<TargetJson> {
    let endpoint = format!(
        "http://127.0.0.1:{}/json/new?{}",
        port,
        urlencoding::encode(url)
    );
    let target: TargetJson = reqwest::get(&endpoint)
        .await
        .context("Failed to create new tab")?
        .json()
        .await?;
    Ok(target)
}

pub async fn get_targets(port: u16) -> Result<Vec<TargetJson>> {
    if !chrome_is_running(port).await {
        eprintln!("Starting Chrome with remote debugging on port {}...", port);
        start_chrome(port)?;

        for _ in 0..50 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            if chrome_is_running(port).await {
                break;
            }
        }

        if !chrome_is_running(port).await {
            anyhow::bail!("Chrome started but failed to connect after 5 seconds");
        }
    }

    let mut targets = get_all_targets(port).await?;
    if targets.is_empty() {
        let new_target = create_new_tab(port, "about:blank").await?;
        targets.push(new_target);
    }
    Ok(targets)
}

pub fn find_active_target(targets: &[TargetJson]) -> Result<&TargetJson> {
    targets
        .iter()
        .find(|t| !t.url.starts_with("about:") && !t.url.starts_with("chrome://"))
        .or(targets.first())
        .context("No pages found. Open a tab in Chrome first.")
}

/// Connect CDP to the active target
pub async fn connect_active(port: u16) -> Result<CdpConnection> {
    let targets = get_targets(port).await?;
    let target = find_active_target(&targets)?;
    let ws_url = target.webSocketDebuggerUrl.as_ref().unwrap();
    CdpConnection::connect(ws_url).await
}
