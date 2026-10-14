use anyhow::{Context, Result, anyhow, bail};
use std::path::PathBuf;

use crate::cdp::{self, BrowserConfig, CdpConnection};
use crate::snapshot::{self, SnapshotOptions};
use crate::tab_target::{MISSING_SELECTOR, SessionStore, TabSelector};

const WAIT_SELECTOR_SCRIPT_TEMPLATE: &str = r#"new Promise((resolve, reject) => {
    const check = () => {
        if (document.querySelector(__SELECTOR__)) resolve(true);
        else setTimeout(check, 100);
    };
    setTimeout(() => reject('Timeout'), 30000);
    check();
})"#;

pub async fn cmd_open(config: &BrowserConfig, url: String, json: bool) -> Result<()> {
    let url = if url.contains("://") {
        url
    } else {
        format!("https://{}", url)
    };
    let mut cdp = cdp::connect_selected(config).await?;

    cdp.send("Page.navigate", serde_json::json!({ "url": url }))
        .await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let title = cdp.eval("document.title").await?;
    let final_url = cdp.eval("window.location.href").await?;

    if json {
        println!(
            "{}",
            serde_json::json!({ "title": title, "url": final_url })
        );
    } else {
        println!("✓ {}", title.as_str().unwrap_or(""));
        println!("  {}", final_url.as_str().unwrap_or(""));
    }
    Ok(())
}

pub async fn cmd_simple_page(config: &BrowserConfig, method: &str, label: &str) -> Result<()> {
    let mut cdp = cdp::connect_selected(config).await?;
    cdp.send(method, serde_json::json!({})).await?;
    println!("✓ {}", label);
    Ok(())
}

pub async fn cmd_click(config: &BrowserConfig, selector: &str) -> Result<()> {
    let mut cdp = cdp::connect_selected(config).await?;
    let script = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            const rect = el.getBoundingClientRect();
            return {{ x: rect.x, y: rect.y, width: rect.width, height: rect.height }};
        }})()"#,
        serde_json::to_string(selector)?
    );
    let point = click_point(&cdp.eval(&script).await?)?;
    for (_, params) in mouse_click_events(point) {
        cdp.send("Input.dispatchMouseEvent", params).await?;
    }
    println!("✓ Clicked");
    Ok(())
}

fn click_point(rect: &serde_json::Value) -> Result<(f64, f64)> {
    let value = |name| {
        rect.get(name)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| anyhow!("Click target rectangle is missing {name}"))
    };
    let x = value("x")?;
    let y = value("y")?;
    let width = value("width")?;
    let height = value("height")?;
    if width <= 0.0 || height <= 0.0 {
        return Err(anyhow!("Click target is not visible"));
    }
    Ok((x + width / 2.0, y + height / 2.0))
}

fn mouse_click_events(point: (f64, f64)) -> Vec<(&'static str, serde_json::Value)> {
    let (x, y) = point;
    vec![
        (
            "mousePressed",
            serde_json::json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        ),
        (
            "mouseReleased",
            serde_json::json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        ),
    ]
}

async fn set_input_value(
    config: &BrowserConfig,
    selector: &str,
    text: &str,
    append: bool,
) -> Result<()> {
    let mut cdp = cdp::connect_selected(config).await?;
    if !append && is_file_input(&mut cdp, selector).await? {
        set_file_input_files(&mut cdp, selector, &[text.to_string()]).await?;
        return Ok(());
    }

    let op = if append { "+=" } else { "=" };
    let script = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            el.focus();
            el.value {} {};
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
            return true;
        }})()"#,
        serde_json::to_string(selector)?,
        op,
        serde_json::to_string(text)?
    );
    cdp.eval(&script).await?;
    Ok(())
}

pub async fn cmd_type(config: &BrowserConfig, selector: &str, text: &str) -> Result<()> {
    set_input_value(config, selector, text, true).await?;
    println!("✓ Typed");
    Ok(())
}

pub async fn cmd_fill(config: &BrowserConfig, selector: &str, text: &str) -> Result<()> {
    set_input_value(config, selector, text, false).await?;
    println!("✓ Filled");
    Ok(())
}

pub async fn cmd_attach(config: &BrowserConfig, selector: &str, files: &[String]) -> Result<()> {
    if files.is_empty() {
        return Err(anyhow!("At least one file path is required"));
    }

    let mut cdp = cdp::connect_selected(config).await?;
    if !is_file_input(&mut cdp, selector).await? {
        return Err(anyhow!("Element is not an input[type=file]"));
    }

    set_file_input_files(&mut cdp, selector, files).await?;
    println!(
        "✓ Attached {} file{}",
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

async fn is_file_input(cdp: &mut CdpConnection, selector: &str) -> Result<bool> {
    let script = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            return el instanceof HTMLInputElement && el.type === 'file';
        }})()"#,
        serde_json::to_string(selector)?
    );
    Ok(cdp.eval(&script).await?.as_bool().unwrap_or(false))
}

async fn set_file_input_files(
    cdp: &mut CdpConnection,
    selector: &str,
    files: &[String],
) -> Result<()> {
    let files = files
        .iter()
        .map(|file| {
            let path = PathBuf::from(file);
            if !path.exists() {
                return Err(anyhow!("File not found: {}", file));
            }
            path.canonicalize()
                .with_context(|| format!("Failed to resolve file path: {}", file))
                .map(|path| path.to_string_lossy().into_owned())
        })
        .collect::<Result<Vec<_>>>()?;

    let document = cdp
        .send("DOM.getDocument", serde_json::json!({ "depth": 0 }))
        .await?;
    let root_node_id = document
        .get("root")
        .and_then(|root| root.get("nodeId"))
        .and_then(|node_id| node_id.as_i64())
        .context("DOM.getDocument did not return a root node")?;
    let node = cdp
        .send(
            "DOM.querySelector",
            serde_json::json!({
                "nodeId": root_node_id,
                "selector": selector,
            }),
        )
        .await?;
    let node_id = node
        .get("nodeId")
        .and_then(|node_id| node_id.as_i64())
        .filter(|node_id| *node_id != 0)
        .context("Element not found")?;

    cdp.send(
        "DOM.setFileInputFiles",
        serde_json::json!({
            "nodeId": node_id,
            "files": files,
        }),
    )
    .await?;

    let event_script = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
            return Array.from(el.files || []).map((file) => file.name);
        }})()"#,
        serde_json::to_string(selector)?
    );
    cdp.eval(&event_script).await?;
    Ok(())
}

pub async fn cmd_press(config: &BrowserConfig, key: &str) -> Result<()> {
    let mut cdp = cdp::connect_selected(config).await?;
    cdp.send(
        "Input.dispatchKeyEvent",
        serde_json::json!({ "type": "keyDown", "key": key }),
    )
    .await?;
    cdp.send(
        "Input.dispatchKeyEvent",
        serde_json::json!({ "type": "keyUp", "key": key }),
    )
    .await?;
    println!("✓ Pressed {}", key);
    Ok(())
}

pub async fn cmd_screenshot(config: &BrowserConfig, path: &str, full: bool) -> Result<()> {
    let mut cdp = cdp::connect_selected(config).await?;
    let mut params = serde_json::json!({ "format": "jpeg", "quality": 15 });
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
    std::fs::write(path, bytes)?;
    println!("✓ Screenshot saved to {}", path);
    Ok(())
}

pub async fn cmd_eval(config: &BrowserConfig, script: &str, json: bool) -> Result<()> {
    let mut cdp = cdp::connect_selected(config).await?;
    let result = cdp.eval(script).await?;
    if json {
        println!("{}", serde_json::to_string(&result)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

pub async fn cmd_snapshot(config: &BrowserConfig, options: SnapshotOptions) -> Result<()> {
    let mut cdp = cdp::connect_selected(config).await?;
    let output = snapshot::take_snapshot(&mut cdp, &options).await?;
    println!("{}", output);
    Ok(())
}

pub async fn cmd_get(config: &BrowserConfig, what: &crate::GetCommand, json: bool) -> Result<()> {
    let target = cdp::selected_target(config).await?;
    let ws = target
        .web_socket_debugger_url
        .as_deref()
        .context("Selected tab has no debugger URL")?;

    match what {
        crate::GetCommand::Title => print_field(json, "title", &target.title),
        crate::GetCommand::Url => print_field(json, "url", &target.url),
        crate::GetCommand::Text { selector } => {
            eval_and_print_str(ws, &build_text_script(selector)?).await?;
        }
        crate::GetCommand::Html { selector } => {
            eval_selector_field(ws, selector, "innerHTML").await?;
        }
        crate::GetCommand::Value { selector } => {
            eval_selector_field(ws, selector, "value").await?;
        }
        crate::GetCommand::Attr { selector, name } => {
            eval_selector_attr(ws, selector, name).await?;
        }
        crate::GetCommand::Count { selector } => {
            eval_selector_count(ws, selector).await?;
        }
    }
    Ok(())
}

async fn eval_selector_field(ws_url: &str, selector: &str, field: &str) -> Result<()> {
    let script = format!(
        "document.querySelector({})?.{} || ''",
        serde_json::to_string(selector)?,
        field
    );
    eval_and_print_str(ws_url, &script).await
}

async fn eval_selector_attr(ws_url: &str, selector: &str, name: &str) -> Result<()> {
    let script = format!(
        "document.querySelector({})?.getAttribute({}) || ''",
        serde_json::to_string(selector)?,
        serde_json::to_string(name)?
    );
    eval_and_print_str(ws_url, &script).await
}

async fn eval_selector_count(ws_url: &str, selector: &str) -> Result<()> {
    let script = format!(
        "document.querySelectorAll({}).length",
        serde_json::to_string(selector)?
    );
    let result = CdpConnection::connect(ws_url).await?.eval(&script).await?;
    println!("{}", result);
    Ok(())
}

fn build_text_script(selector: &Option<String>) -> Result<String> {
    Ok(match selector {
        Some(sel) => format!(
            "document.querySelector({})?.innerText || ''",
            serde_json::to_string(sel)?
        ),
        None => "document.body.innerText".to_string(),
    })
}

async fn eval_and_print_str(ws_url: &str, script: &str) -> Result<()> {
    let mut cdp = CdpConnection::connect(ws_url).await?;
    print_eval_str(&mut cdp, script).await
}

fn print_field(json: bool, key: &str, value: &str) {
    if json {
        println!("{}", serde_json::json!({ key: value }));
    } else {
        println!("{}", value);
    }
}

async fn print_eval_str(cdp: &mut CdpConnection, script: &str) -> Result<()> {
    let result = cdp.eval(script).await?;
    if let Some(text) = result.as_str() {
        println!("{}", text);
    }
    Ok(())
}

pub async fn cmd_tabs(
    config: &BrowserConfig,
    action: &crate::TabsCommand,
    json: bool,
) -> Result<()> {
    match action {
        crate::TabsCommand::List => print_tab_list(config, json).await,
        crate::TabsCommand::New { url } => create_tab(config, url.as_deref()).await,
        crate::TabsCommand::Close => close_tab(config).await,
        crate::TabsCommand::Switch { index } => bind_session_tab(config, *index).await,
    }
}

async fn print_tab_list(config: &BrowserConfig, json: bool) -> Result<()> {
    let targets = cdp::get_targets(config).await?;
    let selected = selected_tab_id(config)?;
    if json {
        let tabs: Vec<_> = targets
            .iter()
            .map(|t| {
                serde_json::json!({
                    "title": t.title,
                    "url": t.url,
                    "id": t.id,
                    "selected": selected.as_deref() == Some(t.id.as_str()),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&tabs)?);
        return Ok(());
    }
    for (i, target) in targets.iter().enumerate() {
        let mark = if selected.as_deref() == Some(target.id.as_str()) {
            "*"
        } else {
            " "
        };
        println!(
            "{mark}{i}: [{}] {} - {}",
            target.id, target.title, target.url
        );
    }
    Ok(())
}

/// The tab this invocation would act on, without creating one.
fn selected_tab_id(config: &BrowserConfig) -> Result<Option<String>> {
    match &config.tab {
        Some(TabSelector::Tab(id)) => Ok(Some(id.clone())),
        Some(TabSelector::Session(session)) => {
            SessionStore::default_location()?.bound_target(session)
        }
        None => Ok(None),
    }
}

async fn create_tab(config: &BrowserConfig, url: Option<&str>) -> Result<()> {
    let target = cdp::create_background_tab(config, url.unwrap_or("about:blank")).await?;
    if let Some(TabSelector::Session(session)) = &config.tab {
        SessionStore::default_location()?.bind(session, &target.id)?;
    }
    println!("✓ New tab {}", target.id);
    Ok(())
}

async fn close_tab(config: &BrowserConfig) -> Result<()> {
    let target = cdp::selected_target(config).await?;
    let mut browser = cdp::connect_browser(config.port).await?;
    browser
        .send(
            "Target.closeTarget",
            serde_json::json!({ "targetId": target.id }),
        )
        .await?;
    println!("✓ Tab {} closed", target.id);
    Ok(())
}

async fn bind_session_tab(config: &BrowserConfig, idx: usize) -> Result<()> {
    let session = match &config.tab {
        Some(TabSelector::Session(session)) => session,
        Some(TabSelector::Tab(_)) => {
            bail!("tabs switch rebinds a --session; --tab already names the tab")
        }
        None => bail!(MISSING_SELECTOR),
    };
    let targets = cdp::get_targets(config).await?;
    let target = targets.get(idx).context("Tab index out of range")?;
    SessionStore::default_location()?.bind(session, &target.id)?;
    println!(
        "✓ Session now uses tab {}: {} [{}]",
        idx, target.title, target.id
    );
    Ok(())
}

pub async fn cmd_wait(
    config: &BrowserConfig,
    target: Option<String>,
    url: Option<String>,
    load: Option<String>,
) -> Result<()> {
    let mut cdp = cdp::connect_selected(config).await?;

    if let Some(ms) = target.as_ref().and_then(|s| s.parse::<u64>().ok()) {
        tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
        println!("✓ Waited {}ms", ms);
        return Ok(());
    }
    if let Some(selector) = target {
        wait_for_selector(&mut cdp, &selector).await?;
        println!("✓ Element found");
        return Ok(());
    }
    if url.is_some() {
        println!("URL wait not implemented");
        return Ok(());
    }
    if load.is_some() {
        println!("Load state wait not implemented");
    }
    Ok(())
}

async fn wait_for_selector(cdp: &mut CdpConnection, selector: &str) -> Result<()> {
    let quoted_selector = serde_json::to_string(selector)?;
    let script = WAIT_SELECTOR_SCRIPT_TEMPLATE.replace("__SELECTOR__", &quoted_selector);
    cdp.eval_await(&script).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::time::{Duration, timeout};
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    async fn selector_wait_response(response: Value) -> (bool, Result<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_seen, request_received) = oneshot::channel();
        let (release_response, response_released) = oneshot::channel();
        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let request = ws.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
            assert_eq!(request["method"], "Runtime.evaluate");
            let awaits_promise = request["params"]["awaitPromise"] == true;
            request_seen.send(()).unwrap();
            if !awaits_promise {
                ws.send(Message::Text(
                    json!({"id": request["id"], "result": {"result": {
                        "type": "object", "className": "Promise", "description": "Promise"
                    }}})
                    .to_string(),
                ))
                .await
                .unwrap();
            }
            response_released.await.unwrap();
            if awaits_promise {
                let mut response = response;
                response["id"] = request["id"].clone();
                ws.send(Message::Text(response.to_string())).await.unwrap();
            }
        };
        let client = async {
            let mut cdp = CdpConnection::connect(&format!("ws://{address}"))
                .await
                .unwrap();
            let wait = wait_for_selector(&mut cdp, "#delayed");
            tokio::pin!(wait);
            let pending_check = async {
                request_received.await.unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            // Poll the production wait while the fixture receives its request.
            let early = tokio::select! {
                result = &mut wait => Some(result),
                () = pending_check => None,
            };
            release_response.send(()).unwrap();
            match early {
                Some(result) => (true, result),
                None => (false, wait.await),
            }
        };
        let (_, result) = timeout(Duration::from_secs(5), async {
            tokio::join!(server, client)
        })
        .await
        .unwrap();
        result
    }

    #[tokio::test]
    async fn selector_wait_stays_pending_until_resolution() {
        let (early, result) = selector_wait_response(json!({
            "result": {"result": {"type": "boolean", "value": true}}
        }))
        .await;
        assert!(!early, "selector wait completed before Promise resolution");
        result.unwrap();
    }

    #[tokio::test]
    async fn selector_wait_stays_pending_until_timeout_rejection() {
        let (early, result) = selector_wait_response(json!({
            "result": {
                "result": {"type": "string", "value": "Timeout"},
                "exceptionDetails": {"text": "Uncaught (in promise) Timeout"}
            }
        }))
        .await;
        assert!(!early, "selector wait completed before Promise rejection");
        assert_eq!(
            result.unwrap_err().to_string(),
            "JavaScript exception: Uncaught (in promise) Timeout"
        );
    }

    #[tokio::test]
    async fn selector_wait_propagates_exception_description() {
        let (early, result) = selector_wait_response(json!({
            "result": {
                "result": {"type": "undefined"},
                "exceptionDetails": {
                    "text": "Uncaught (in promise)",
                    "exception": {"description": "SyntaxError: invalid selector"}
                }
            }
        }))
        .await;
        assert!(!early, "selector wait completed before Promise rejection");
        assert_eq!(
            result.unwrap_err().to_string(),
            "JavaScript exception: SyntaxError: invalid selector"
        );
    }

    #[tokio::test]
    async fn selector_wait_propagates_protocol_error() {
        let (early, result) = selector_wait_response(json!({
            "error": {"code": -32000, "message": "Execution context was destroyed"}
        }))
        .await;
        assert!(
            !early,
            "selector wait completed before delayed protocol error"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Execution context was destroyed")
        );
    }

    #[test]
    fn builds_real_pointer_click_for_visible_element_center() {
        let point =
            click_point(&serde_json::json!({"x": 10.0, "y": 20.0, "width": 40.0, "height": 20.0}))
                .unwrap();
        assert_eq!((30.0, 30.0), point);
        assert_eq!(
            vec!["mousePressed", "mouseReleased"],
            mouse_click_events(point)
                .iter()
                .map(|(event, _)| *event)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn rejects_zero_size_click_target() {
        assert!(
            click_point(&serde_json::json!({"x": 0, "y": 0, "width": 0, "height": 20})).is_err()
        );
    }
}
