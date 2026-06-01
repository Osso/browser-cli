use anyhow::{Context, Result};

use crate::cdp::{self, CdpConnection};
use crate::snapshot::{self, SnapshotOptions};

const WAIT_SELECTOR_SCRIPT_TEMPLATE: &str = r#"new Promise((resolve, reject) => {
    const check = () => {
        if (document.querySelector(__SELECTOR__)) resolve(true);
        else setTimeout(check, 100);
    };
    setTimeout(() => reject('Timeout'), 30000);
    check();
})"#;

pub async fn cmd_open(port: u16, url: String, json: bool) -> Result<()> {
    let url = if url.contains("://") {
        url
    } else {
        format!("https://{}", url)
    };
    let targets = cdp::get_targets(port).await?;
    let any_target = targets.first().context("No browser targets")?;
    let ws_url = any_target.webSocketDebuggerUrl.as_ref().unwrap();
    let mut cdp = CdpConnection::connect(ws_url).await?;

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

pub async fn cmd_simple_page(port: u16, method: &str, label: &str) -> Result<()> {
    let mut cdp = cdp::connect_active(port).await?;
    cdp.send(method, serde_json::json!({})).await?;
    println!("✓ {}", label);
    Ok(())
}

pub async fn cmd_click(port: u16, selector: &str) -> Result<()> {
    let mut cdp = cdp::connect_active(port).await?;
    let script = format!(
        r#"(() => {{
            const el = document.querySelector({});
            if (!el) throw new Error('Element not found');
            el.click();
            return true;
        }})()"#,
        serde_json::to_string(selector)?
    );
    cdp.eval(&script).await?;
    println!("✓ Clicked");
    Ok(())
}

async fn set_input_value(port: u16, selector: &str, text: &str, append: bool) -> Result<()> {
    let mut cdp = cdp::connect_active(port).await?;
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

pub async fn cmd_type(port: u16, selector: &str, text: &str) -> Result<()> {
    set_input_value(port, selector, text, true).await?;
    println!("✓ Typed");
    Ok(())
}

pub async fn cmd_fill(port: u16, selector: &str, text: &str) -> Result<()> {
    set_input_value(port, selector, text, false).await?;
    println!("✓ Filled");
    Ok(())
}

pub async fn cmd_press(port: u16, key: &str) -> Result<()> {
    let mut cdp = cdp::connect_active(port).await?;
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

pub async fn cmd_screenshot(port: u16, path: &str, full: bool) -> Result<()> {
    let mut cdp = cdp::connect_active(port).await?;
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

pub async fn cmd_eval(port: u16, script: &str, json: bool) -> Result<()> {
    let mut cdp = cdp::connect_active(port).await?;
    let result = cdp.eval(script).await?;
    if json {
        println!("{}", serde_json::to_string(&result)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

pub async fn cmd_snapshot(
    port: u16,
    interactive: bool,
    compact: bool,
    react: bool,
    depth: Option<usize>,
    filter: Option<String>,
    full: bool,
    mini: bool,
) -> Result<()> {
    let mut cdp = cdp::connect_active(port).await?;
    let opts = SnapshotOptions {
        interactive,
        compact,
        react,
        max_depth: depth,
        filter,
        full,
        mini,
    };
    let output = snapshot::take_snapshot(&mut cdp, &opts).await?;
    println!("{}", output);
    Ok(())
}

pub async fn cmd_get(port: u16, what: &crate::GetCommand, json: bool) -> Result<()> {
    let targets = cdp::get_targets(port).await?;
    let target = cdp::find_active_target(&targets)?;
    let ws = target.webSocketDebuggerUrl.as_ref().unwrap();

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

pub async fn cmd_tabs(port: u16, action: &crate::TabsCommand, json: bool) -> Result<()> {
    let targets = cdp::get_targets(port).await?;

    match action {
        crate::TabsCommand::List => print_tab_list(&targets, json)?,
        crate::TabsCommand::New { url } => {
            create_tab(&targets, url.as_deref()).await?;
        }
        crate::TabsCommand::Close { index } => {
            close_tab(&targets, index.unwrap_or(0)).await?;
        }
        crate::TabsCommand::Switch { index } => {
            switch_tab(&targets, *index).await?;
        }
    }
    Ok(())
}

fn print_tab_list(targets: &[cdp::TargetJson], json: bool) -> Result<()> {
    if json {
        let tabs: Vec<_> = targets
            .iter()
            .map(|t| serde_json::json!({ "title": t.title, "url": t.url, "id": t.id }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&tabs)?);
        return Ok(());
    }
    for (i, target) in targets.iter().enumerate() {
        println!("{}: {} - {}", i, target.title, target.url);
    }
    Ok(())
}

async fn create_tab(targets: &[cdp::TargetJson], url: Option<&str>) -> Result<()> {
    let mut cdp = connect_target_session(targets).await?;
    let url = url.unwrap_or("about:blank");
    cdp.send("Target.createTarget", serde_json::json!({ "url": url }))
        .await?;
    println!("✓ New tab created");
    Ok(())
}

async fn close_tab(targets: &[cdp::TargetJson], idx: usize) -> Result<()> {
    let target = targets.get(idx).context("Tab index out of range")?;
    let mut cdp = connect_target_session(targets).await?;
    cdp.send(
        "Target.closeTarget",
        serde_json::json!({ "targetId": target.id }),
    )
    .await?;
    println!("✓ Tab closed");
    Ok(())
}

async fn switch_tab(targets: &[cdp::TargetJson], idx: usize) -> Result<()> {
    let target = targets.get(idx).context("Tab index out of range")?;
    let mut cdp = connect_target_session(targets).await?;
    cdp.send(
        "Target.activateTarget",
        serde_json::json!({ "targetId": target.id }),
    )
    .await?;
    println!("✓ Switched to tab {}: {}", idx, target.title);
    Ok(())
}

async fn connect_target_session(targets: &[cdp::TargetJson]) -> Result<CdpConnection> {
    let target = targets.first().context("No browser targets")?;
    CdpConnection::connect(target.webSocketDebuggerUrl.as_ref().unwrap()).await
}

pub async fn cmd_wait(
    port: u16,
    target: Option<String>,
    url: Option<String>,
    load: Option<String>,
) -> Result<()> {
    let mut cdp = cdp::connect_active(port).await?;

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
    cdp.eval(&script).await?;
    Ok(())
}
