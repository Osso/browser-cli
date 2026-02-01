# browser-cli

Rust CLI for browser automation via Chrome DevTools Protocol.

## Structure

```
src/
  main.rs       - CLI args (clap), CDP communication, command handlers
```

Single-file implementation. All logic in main.rs.

## Architecture

- Connects to Chrome via CDP HTTP endpoint (`/json`) to discover targets
- Uses WebSocket for CDP commands (tokio-tungstenite)
- `CdpConnection` struct manages WebSocket and message IDs
- Commands use CSS selectors for element targeting
- Screenshots output JPEG quality 15

## CDP Communication

```rust
CdpConnection::connect(ws_url)  // Connect to target's WebSocket
cdp.send(method, params)        // Send CDP command, await response
cdp.eval(expression)            // Shorthand for Runtime.evaluate
```

Key CDP methods used:
- `Page.navigate`, `Page.goBack`, `Page.goForward`, `Page.reload`, `Page.close`
- `Page.captureScreenshot`
- `Runtime.evaluate` (for DOM interactions via JavaScript)
- `Input.dispatchKeyEvent`
- `Target.createTarget`, `Target.closeTarget`, `Target.activateTarget`

## Adding a new command

1. Add variant to `Command` enum with clap attributes
2. Add match arm in `main()` function
3. Use `CdpConnection::send()` for CDP methods or `cdp.eval()` for JS execution

## Prerequisites

Chrome must run with `--remote-debugging-port=9222`

## Build

```bash
cargo build --release
```
