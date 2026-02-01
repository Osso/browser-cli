use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::Message;

const DEFAULT_CDP_PORT: u16 = 9222;

#[derive(Parser)]
#[command(name = "browser-cli")]
#[command(about = "Browser automation CLI using Chrome DevTools Protocol")]
struct Cli {
    /// CDP port to connect to
    #[arg(long, default_value_t = DEFAULT_CDP_PORT)]
    port: u16,

    /// Output as JSON
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Navigate to a URL
    #[command(visible_alias = "goto", visible_alias = "navigate")]
    Open {
        url: String,
    },

    /// Go back in history
    Back,

    /// Go forward in history
    Forward,

    /// Reload current page
    Reload,

    /// Close browser/tab
    #[command(visible_alias = "quit", visible_alias = "exit")]
    Close,

    /// Click an element
    Click {
        selector: String,
    },

    /// Type text into an element
    Type {
        selector: String,
        text: String,
    },

    /// Clear and fill an element
    Fill {
        selector: String,
        text: String,
    },

    /// Press a key
    #[command(visible_alias = "key")]
    Press {
        key: String,
    },

    /// Take a screenshot (JPEG format, quality 15)
    Screenshot {
        /// Output path
        #[arg(default_value = "/tmp/claude/screenshot.jpg")]
        path: String,
        /// Full page screenshot
        #[arg(short, long)]
        full: bool,
    },

    /// Evaluate JavaScript
    Eval {
        script: String,
    },

    /// Get page information
    Get {
        #[command(subcommand)]
        what: GetCommand,
    },

    /// Manage tabs
    Tabs {
        #[command(subcommand)]
        action: TabsCommand,
    },

    /// Wait for element, time, or condition
    Wait {
        /// Selector or milliseconds
        target: Option<String>,
        /// Wait for URL pattern
        #[arg(short, long)]
        url: Option<String>,
        /// Wait for load state
        #[arg(short, long)]
        load: Option<String>,
    },
}

#[derive(Subcommand)]
enum GetCommand {
    /// Get page title
    Title,
    /// Get current URL
    Url,
    /// Get element text
    Text { selector: Option<String> },
    /// Get element HTML
    Html { selector: String },
    /// Get input value
    Value { selector: String },
    /// Get element attribute
    Attr { selector: String, name: String },
    /// Count matching elements
    Count { selector: String },
}

#[derive(Subcommand)]
enum TabsCommand {
    /// List open tabs
    List,
    /// Open new tab
    New { url: Option<String> },
    /// Close tab
    Close { index: Option<usize> },
    /// Switch to tab by index
    Switch { index: usize },
}

// --- CDP Communication ---

#[derive(Deserialize)]
#[allow(non_snake_case, dead_code)]
struct TargetJson {
    id: String,
    title: String,
    url: String,
    r#type: String,
    webSocketDebuggerUrl: Option<String>,
}

async fn get_targets(port: u16) -> Result<Vec<TargetJson>> {
    let url = format!("http://127.0.0.1:{}/json", port);
    let targets: Vec<TargetJson> = reqwest::get(&url)
        .await
        .with_context(|| format!("Failed to connect to Chrome on port {}", port))?
        .json()
        .await?;
    Ok(targets
        .into_iter()
        .filter(|t| t.r#type == "page" && t.webSocketDebuggerUrl.is_some())
        .collect())
}

fn find_active_target(targets: &[TargetJson]) -> Result<&TargetJson> {
    targets
        .iter()
        .find(|t| !t.url.starts_with("about:") && !t.url.starts_with("chrome://"))
        .or(targets.first())
        .context("No pages found. Open a tab in Chrome first.")
}

struct CdpConnection {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: i32,
}

impl CdpConnection {
    async fn connect(ws_url: &str) -> Result<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
        Ok(Self { ws, next_id: 1 })
    }

    async fn send(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({
            "id": id,
            "method": method,
            "params": params
        });

        self.ws
            .send(Message::Text(msg.to_string()))
            .await?;

        while let Some(msg) = self.ws.next().await {
            if let Ok(Message::Text(text)) = msg {
                let resp: serde_json::Value = serde_json::from_str(&text)?;
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

    async fn eval(&mut self, expression: &str) -> Result<serde_json::Value> {
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

// --- Main ---

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Open { url } => {
            let url = if url.contains("://") {
                url
            } else {
                format!("https://{}", url)
            };

            // Use CDP to create new target
            let targets = get_targets(cli.port).await?;
            let any_target = targets.first().context("No browser targets")?;
            let ws_url = any_target.webSocketDebuggerUrl.as_ref().unwrap();
            let mut cdp = CdpConnection::connect(ws_url).await?;

            // Navigate current page instead of creating new tab
            cdp.send("Page.navigate", serde_json::json!({ "url": url }))
                .await?;

            // Wait for load
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            let title = cdp.eval("document.title").await?;
            let final_url = cdp.eval("window.location.href").await?;

            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "title": title,
                        "url": final_url
                    })
                );
            } else {
                println!("✓ {}", title.as_str().unwrap_or(""));
                println!("  {}", final_url.as_str().unwrap_or(""));
            }
        }

        Command::Back => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;
            cdp.send("Page.goBack", serde_json::json!({})).await?;
            println!("✓ Back");
        }

        Command::Forward => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;
            cdp.send("Page.goForward", serde_json::json!({})).await?;
            println!("✓ Forward");
        }

        Command::Reload => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;
            cdp.send("Page.reload", serde_json::json!({})).await?;
            println!("✓ Reloaded");
        }

        Command::Close => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;
            cdp.send("Page.close", serde_json::json!({})).await?;
            println!("✓ Closed");
        }

        Command::Click { selector } => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;

            let script = format!(
                r#"(() => {{
                    const el = document.querySelector({});
                    if (!el) throw new Error('Element not found');
                    el.click();
                    return true;
                }})()"#,
                serde_json::to_string(&selector)?
            );
            cdp.eval(&script).await?;
            println!("✓ Clicked");
        }

        Command::Type { selector, text } => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;

            let script = format!(
                r#"(() => {{
                    const el = document.querySelector({});
                    if (!el) throw new Error('Element not found');
                    el.focus();
                    el.value += {};
                    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                    return true;
                }})()"#,
                serde_json::to_string(&selector)?,
                serde_json::to_string(&text)?
            );
            cdp.eval(&script).await?;
            println!("✓ Typed");
        }

        Command::Fill { selector, text } => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;

            let script = format!(
                r#"(() => {{
                    const el = document.querySelector({});
                    if (!el) throw new Error('Element not found');
                    el.focus();
                    el.value = {};
                    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                    return true;
                }})()"#,
                serde_json::to_string(&selector)?,
                serde_json::to_string(&text)?
            );
            cdp.eval(&script).await?;
            println!("✓ Filled");
        }

        Command::Press { key } => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;

            // Simple key dispatch via Input.dispatchKeyEvent
            cdp.send(
                "Input.dispatchKeyEvent",
                serde_json::json!({
                    "type": "keyDown",
                    "key": key
                }),
            )
            .await?;
            cdp.send(
                "Input.dispatchKeyEvent",
                serde_json::json!({
                    "type": "keyUp",
                    "key": key
                }),
            )
            .await?;
            println!("✓ Pressed {}", key);
        }

        Command::Screenshot { path, full } => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;

            let mut params = serde_json::json!({
                "format": "jpeg",
                "quality": 15
            });
            if full {
                params["captureBeyondViewport"] = serde_json::json!(true);
            }

            let result = cdp.send("Page.captureScreenshot", params).await?;
            let data = result
                .get("data")
                .and_then(|d| d.as_str())
                .context("No screenshot data")?;

            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;

            std::fs::write(&path, bytes)?;
            println!("✓ Screenshot saved to {}", path);
        }

        Command::Eval { script } => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await?;

            let result = cdp.eval(&script).await?;
            if cli.json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }

        Command::Get { what } => {
            let targets = get_targets(cli.port).await?;
            let target = find_active_target(&targets)?;

            match what {
                GetCommand::Title => {
                    if cli.json {
                        println!("{}", serde_json::json!({ "title": target.title }));
                    } else {
                        println!("{}", target.title);
                    }
                }
                GetCommand::Url => {
                    if cli.json {
                        println!("{}", serde_json::json!({ "url": target.url }));
                    } else {
                        println!("{}", target.url);
                    }
                }
                GetCommand::Text { selector } => {
                    let mut cdp =
                        CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap())
                            .await?;
                    let script = match selector {
                        Some(sel) => format!(
                            "document.querySelector({})?.innerText || ''",
                            serde_json::to_string(&sel)?
                        ),
                        None => "document.body.innerText".to_string(),
                    };
                    let result = cdp.eval(&script).await?;
                    if let Some(text) = result.as_str() {
                        println!("{}", text);
                    }
                }
                GetCommand::Html { selector } => {
                    let mut cdp =
                        CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap())
                            .await?;
                    let script = format!(
                        "document.querySelector({})?.innerHTML || ''",
                        serde_json::to_string(&selector)?
                    );
                    let result = cdp.eval(&script).await?;
                    if let Some(html) = result.as_str() {
                        println!("{}", html);
                    }
                }
                GetCommand::Value { selector } => {
                    let mut cdp =
                        CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap())
                            .await?;
                    let script = format!(
                        "document.querySelector({})?.value || ''",
                        serde_json::to_string(&selector)?
                    );
                    let result = cdp.eval(&script).await?;
                    if let Some(value) = result.as_str() {
                        println!("{}", value);
                    }
                }
                GetCommand::Attr { selector, name } => {
                    let mut cdp =
                        CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap())
                            .await?;
                    let script = format!(
                        "document.querySelector({})?.getAttribute({}) || ''",
                        serde_json::to_string(&selector)?,
                        serde_json::to_string(&name)?
                    );
                    let result = cdp.eval(&script).await?;
                    if let Some(attr) = result.as_str() {
                        println!("{}", attr);
                    }
                }
                GetCommand::Count { selector } => {
                    let mut cdp =
                        CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap())
                            .await?;
                    let script = format!(
                        "document.querySelectorAll({}).length",
                        serde_json::to_string(&selector)?
                    );
                    let result = cdp.eval(&script).await?;
                    println!("{}", result);
                }
            }
        }

        Command::Tabs { action } => {
            let targets = get_targets(cli.port).await?;

            match action {
                TabsCommand::List => {
                    if cli.json {
                        let tabs: Vec<_> = targets
                            .iter()
                            .map(|t| {
                                serde_json::json!({
                                    "title": t.title,
                                    "url": t.url,
                                    "id": t.id
                                })
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&tabs)?);
                    } else {
                        for (i, target) in targets.iter().enumerate() {
                            println!("{}: {} - {}", i, target.title, target.url);
                        }
                    }
                }
                TabsCommand::New { url } => {
                    let target = targets.first().context("No browser targets")?;
                    let mut cdp =
                        CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap())
                            .await?;
                    let url = url.unwrap_or_else(|| "about:blank".to_string());
                    cdp.send("Target.createTarget", serde_json::json!({ "url": url }))
                        .await?;
                    println!("✓ New tab created");
                }
                TabsCommand::Close { index } => {
                    let idx = index.unwrap_or(0);
                    let target = targets.get(idx).context("Tab index out of range")?;
                    let any_target = targets.first().unwrap();
                    let mut cdp =
                        CdpConnection::connect(any_target.webSocketDebuggerUrl.as_ref().unwrap())
                            .await?;
                    cdp.send(
                        "Target.closeTarget",
                        serde_json::json!({ "targetId": target.id }),
                    )
                    .await?;
                    println!("✓ Tab closed");
                }
                TabsCommand::Switch { index } => {
                    let target = targets.get(index).context("Tab index out of range")?;
                    let any_target = targets.first().unwrap();
                    let mut cdp =
                        CdpConnection::connect(any_target.webSocketDebuggerUrl.as_ref().unwrap())
                            .await?;
                    cdp.send(
                        "Target.activateTarget",
                        serde_json::json!({ "targetId": target.id }),
                    )
                    .await?;
                    println!("✓ Switched to tab {}: {}", index, target.title);
                }
            }
        }

        Command::Wait { target, url, load } => {
            let targets = get_targets(cli.port).await?;
            let t = find_active_target(&targets)?;
            let mut cdp =
                CdpConnection::connect(t.webSocketDebuggerUrl.as_ref().unwrap()).await?;

            if let Some(ms) = target.as_ref().and_then(|s| s.parse::<u64>().ok()) {
                tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
                println!("✓ Waited {}ms", ms);
            } else if let Some(selector) = target {
                let script = format!(
                    r#"new Promise((resolve, reject) => {{
                        const check = () => {{
                            if (document.querySelector({})) resolve(true);
                            else setTimeout(check, 100);
                        }};
                        setTimeout(() => reject('Timeout'), 30000);
                        check();
                    }})"#,
                    serde_json::to_string(&selector)?
                );
                cdp.eval(&script).await?;
                println!("✓ Element found");
            } else if let Some(_url_pattern) = url {
                // TODO: implement URL wait
                println!("URL wait not implemented");
            } else if let Some(_state) = load {
                // TODO: implement load state wait
                println!("Load state wait not implemented");
            }
        }
    }

    Ok(())
}
