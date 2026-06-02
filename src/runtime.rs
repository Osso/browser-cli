use anyhow::Result;
use serde_json::Value;
use tokio::time::{Duration, Instant, timeout};

use crate::cdp;

pub async fn cmd_runtime(port: u16, action: &crate::RuntimeCommand, json: bool) -> Result<()> {
    let (kind, reload, wait_ms) = match action {
        crate::RuntimeCommand::Console { reload, wait_ms } => ("console", *reload, *wait_ms),
        crate::RuntimeCommand::Exceptions { reload, wait_ms } => ("exceptions", *reload, *wait_ms),
    };
    let events = collect_runtime_events(port, kind, reload, wait_ms).await?;
    print_runtime_events(kind, &events, json)?;
    Ok(())
}

async fn collect_runtime_events(
    port: u16,
    kind: &str,
    reload: bool,
    wait_ms: u64,
) -> Result<Vec<Value>> {
    let mut cdp = cdp::connect_active(port).await?;
    cdp.send("Runtime.enable", serde_json::json!({})).await?;
    if reload {
        cdp.send("Page.reload", serde_json::json!({})).await?;
    }

    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Some(message) = timeout(remaining, cdp.recv()).await.unwrap_or(Ok(None))? else {
            break;
        };
        if let Some(event) = format_runtime_event(kind, &message) {
            events.push(event);
        }
    }
    Ok(events)
}

pub(crate) fn format_runtime_event(kind: &str, message: &Value) -> Option<Value> {
    let method = message.get("method")?.as_str()?;
    let params = message.get("params")?;
    match (kind, method) {
        ("console", "Runtime.consoleAPICalled") => Some(format_console_event(params)),
        ("exceptions", "Runtime.exceptionThrown") => Some(format_exception_event(params)),
        _ => None,
    }
}

fn format_console_event(params: &Value) -> Value {
    let args = params
        .get("args")
        .and_then(Value::as_array)
        .map(|args| args.iter().map(remote_object_text).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut event = serde_json::json!({
        "type": params.get("type").and_then(Value::as_str).unwrap_or("log"),
        "text": args.join(" "),
        "args": args,
    });
    add_stack_location(&mut event, params);
    event
}

fn format_exception_event(params: &Value) -> Value {
    let details = params.get("exceptionDetails").unwrap_or(params);
    let exception = details.get("exception").unwrap_or(&Value::Null);
    let mut event = serde_json::json!({
        "text": details.get("text").and_then(Value::as_str).unwrap_or("exception"),
        "message": remote_object_text(exception),
        "url": details.get("url").and_then(Value::as_str).unwrap_or(""),
        "lineNumber": details.get("lineNumber").and_then(Value::as_i64).unwrap_or(0),
        "columnNumber": details.get("columnNumber").and_then(Value::as_i64).unwrap_or(0),
    });
    add_stack_location(&mut event, details);
    event
}

fn remote_object_text(value: &Value) -> String {
    if let Some(text) = value.get("value").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(value) = value.get("value") {
        return value.to_string();
    }
    if let Some(text) = value.get("description").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(text) = value.get("unserializableValue").and_then(Value::as_str) {
        return text.to_string();
    }
    String::new()
}

fn add_stack_location(event: &mut Value, params: &Value) {
    let Some(frame) = params
        .get("stackTrace")
        .and_then(|trace| trace.get("callFrames"))
        .and_then(Value::as_array)
        .and_then(|frames| frames.first())
    else {
        return;
    };
    event["url"] = frame
        .get("url")
        .cloned()
        .unwrap_or(Value::String(String::new()));
    event["lineNumber"] = frame
        .get("lineNumber")
        .cloned()
        .unwrap_or(Value::Number(0.into()));
    event["columnNumber"] = frame
        .get("columnNumber")
        .cloned()
        .unwrap_or(Value::Number(0.into()));
}

fn print_runtime_events(kind: &str, events: &[Value], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(events)?);
        return Ok(());
    }
    for event in events {
        let text = event
            .get("message")
            .or_else(|| event.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let label = event.get("type").and_then(Value::as_str).unwrap_or(kind);
        println!("[{label}] {text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::format_runtime_event;
    use serde_json::json;

    #[test]
    fn formats_console_api_events() {
        let message = json!({
            "method": "Runtime.consoleAPICalled",
            "params": {
                "type": "error",
                "args": [
                    { "type": "string", "value": "failed" },
                    { "type": "number", "value": 42 }
                ],
                "stackTrace": {
                    "callFrames": [{
                        "url": "https://example.com/app.js",
                        "lineNumber": 10,
                        "columnNumber": 4
                    }]
                }
            }
        });

        let event = format_runtime_event("console", &message).expect("console event");

        assert_eq!(
            event,
            json!({
                "type": "error",
                "text": "failed 42",
                "args": ["failed", "42"],
                "url": "https://example.com/app.js",
                "lineNumber": 10,
                "columnNumber": 4
            })
        );
    }

    #[test]
    fn formats_runtime_exception_events() {
        let message = json!({
            "method": "Runtime.exceptionThrown",
            "params": {
                "exceptionDetails": {
                    "text": "Uncaught",
                    "url": "https://example.com/app.js",
                    "lineNumber": 20,
                    "columnNumber": 8,
                    "exception": {
                        "type": "object",
                        "description": "Error: boom"
                    }
                }
            }
        });

        let event = format_runtime_event("exceptions", &message).expect("exception event");

        assert_eq!(
            event,
            json!({
                "text": "Uncaught",
                "message": "Error: boom",
                "url": "https://example.com/app.js",
                "lineNumber": 20,
                "columnNumber": 8
            })
        );
    }

    #[test]
    fn ignores_unrequested_runtime_events() {
        let message = json!({
            "method": "Runtime.consoleAPICalled",
            "params": { "type": "log", "args": [] }
        });

        assert_eq!(format_runtime_event("exceptions", &message), None);
    }
}
