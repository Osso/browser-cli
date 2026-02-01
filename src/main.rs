use anyhow::{anyhow, Context, Result};
use chromiumoxide::cdp::browser_protocol::target::{
    AttachToTargetParams, GetTargetsParams, TargetInfo,
};
use chromiumoxide::Browser;
use clap::{Parser, Subcommand};
use futures::StreamExt;
use serde::Deserialize;

const DEFAULT_CDP_PORT: u16 = 9222;

#[derive(Parser)]
#[command(name = "browser-cli")]
#[command(about = "Browser automation CLI using Chrome DevTools Protocol")]
struct Cli {
    /// CDP port to connect to
    #[arg(long, default_value_t = DEFAULT_CDP_PORT)]
    port: u16,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Navigate to a URL (opens new tab)
    Open { url: String },
    /// Take a screenshot
    Screenshot { path: Option<String> },
    /// Get page title
    Title,
    /// Get current URL
    Url,
    /// Evaluate JavaScript
    Eval { script: String },
    /// Get page text content
    Text,
    /// List open tabs
    Tabs,
}

async fn connect_browser(port: u16) -> Result<Browser> {
    let url = format!("http://127.0.0.1:{}", port);
    let (browser, mut handler) = Browser::connect(&url)
        .await
        .with_context(|| format!("Failed to connect to Chrome on port {}. Is Chrome running with --remote-debugging-port={}?", port, port))?;

    tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if event.is_err() {
                break;
            }
        }
    });

    Ok(browser)
}

/// Get all page targets from Chrome
async fn get_targets(browser: &Browser) -> Result<Vec<TargetInfo>> {
    let resp = browser.execute(GetTargetsParams::default()).await?;
    Ok(resp
        .target_infos
        .clone()
        .into_iter()
        .filter(|t| t.r#type == "page")
        .collect())
}

/// Get the first non-blank page target
fn find_active_target(targets: &[TargetInfo]) -> Result<&TargetInfo> {
    targets
        .iter()
        .find(|t| !t.url.starts_with("about:") && !t.url.starts_with("chrome://"))
        .or(targets.first())
        .context("No pages found. Open a tab in Chrome first.")
}

/// Execute JavaScript on a target via HTTP endpoint
async fn eval_on_target(port: u16, target_id: &str, expression: &str) -> Result<serde_json::Value> {
    let client = reqwest::Client::new();

    // Get WebSocket URL for target
    let targets_url = format!("http://127.0.0.1:{}/json", port);
    let targets: Vec<TargetJson> = client.get(&targets_url).send().await?.json().await?;

    let target = targets
        .iter()
        .find(|t| t.id == target_id)
        .context("Target not found")?;

    let ws_url = target.webSocketDebuggerUrl.as_ref()
        .context("No WebSocket URL for target")?;

    // Connect via WebSocket and evaluate
    use tokio_tungstenite::connect_async;
    use futures::{SinkExt, StreamExt as _};

    let (mut ws, _) = connect_async(ws_url).await?;

    let msg = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true
        }
    });

    ws.send(tokio_tungstenite::tungstenite::Message::Text(msg.to_string())).await?;

    while let Some(msg) = ws.next().await {
        if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
            let resp: serde_json::Value = serde_json::from_str(&text)?;
            if resp.get("id") == Some(&serde_json::json!(1)) {
                if let Some(result) = resp.get("result").and_then(|r| r.get("result")).and_then(|r| r.get("value")) {
                    return Ok(result.clone());
                }
                return Err(anyhow!("Eval failed: {:?}", resp));
            }
        }
    }

    Err(anyhow!("No response from WebSocket"))
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct TargetJson {
    id: String,
    title: String,
    url: String,
    webSocketDebuggerUrl: Option<String>,
}

/// Get targets via HTTP
async fn get_targets_http(port: u16) -> Result<Vec<TargetJson>> {
    let url = format!("http://127.0.0.1:{}/json", port);
    let targets: Vec<TargetJson> = reqwest::get(&url).await?.json().await?;
    Ok(targets.into_iter().filter(|t| t.webSocketDebuggerUrl.is_some()).collect())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Open { url } => {
            let browser = connect_browser(cli.port).await?;
            let url = if url.contains("://") {
                url
            } else {
                format!("https://{}", url)
            };
            let page = browser.new_page(&url).await?;
            page.wait_for_navigation().await?;
            let title = page.get_title().await?.unwrap_or_default();
            let final_url = page.url().await?.unwrap_or_default();
            println!("✓ {}", title);
            println!("  {}", final_url);
        }
        Command::Tabs => {
            let targets = get_targets_http(cli.port).await?;
            for (i, target) in targets.iter().enumerate() {
                println!("{}: {} - {}", i, target.title, target.url);
            }
        }
        Command::Title => {
            let targets = get_targets_http(cli.port).await?;
            let target = targets
                .iter()
                .find(|t| !t.url.starts_with("about:") && !t.url.starts_with("chrome://"))
                .or(targets.first())
                .context("No pages found")?;
            println!("{}", target.title);
        }
        Command::Url => {
            let targets = get_targets_http(cli.port).await?;
            let target = targets
                .iter()
                .find(|t| !t.url.starts_with("about:") && !t.url.starts_with("chrome://"))
                .or(targets.first())
                .context("No pages found")?;
            println!("{}", target.url);
        }
        Command::Eval { script } => {
            let targets = get_targets_http(cli.port).await?;
            let target = targets
                .iter()
                .find(|t| !t.url.starts_with("about:") && !t.url.starts_with("chrome://"))
                .or(targets.first())
                .context("No pages found")?;
            let result = eval_on_target(cli.port, &target.id, &script).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Text => {
            let targets = get_targets_http(cli.port).await?;
            let target = targets
                .iter()
                .find(|t| !t.url.starts_with("about:") && !t.url.starts_with("chrome://"))
                .or(targets.first())
                .context("No pages found")?;
            let result = eval_on_target(cli.port, &target.id, "document.body.innerText").await?;
            if let Some(text) = result.as_str() {
                println!("{}", text);
            }
        }
        Command::Screenshot { path: _ } => {
            // TODO: implement via CDP
            println!("Screenshot not yet implemented for existing tabs");
        }
    }

    Ok(())
}
