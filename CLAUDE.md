# browser-cli

Rust CLI for browser automation via Chrome DevTools Protocol.

## Structure

```
src/
  main.rs       - CLI arguments and command dispatch
  broker.rs     - credential-free Secrets Broker socket client
  cdp.rs        - CDP target discovery and browser interaction
  commands.rs   - direct browser command handlers
```

Broker integration is deliberately credential-free: browser-cli sends only a named scope and active CDP target ID to Secrets Broker.

## Architecture

- Connects to Chrome via CDP HTTP endpoint (`/json`) to discover targets
- Uses WebSocket for CDP commands (tokio-tungstenite)
- `CdpConnection` struct manages WebSocket and message IDs
- Commands use CSS selectors for element targeting
- `click` rejects zero-size targets and dispatches real CDP pointer events; do not replace it with DOM `.click()` for custom controls
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
- `Runtime.evaluate` (for DOM inspection and value injection)
- `Input.dispatchMouseEvent` (for real pointer clicks at visible element centers)
- `Input.dispatchKeyEvent`
- `Target.createTarget`, `Target.closeTarget`, `Target.activateTarget`

## Secrets Broker integration

- `broker-unlock --scope browser:citi` requests human approval through authd.
- `broker-fill --scope browser:citi` sends only the scope and active target ID to `/run/secrets-broker/broker.sock`.
- `--current-origin` resolves the active tab's exact HTTPS hostname through `$XDG_CONFIG_HOME/browser-cli/credential-scopes.json` or `~/.config/browser-cli/credential-scopes.json`.
- The mapping file is a mode-`0600`, current-user-owned regular JSON file from lowercase exact hostname to broker scope. Never add credentials, wildcards, suffix matching, symlinks, or permissive modes.
- When `--scope` and `--current-origin` are combined, require equality; re-resolve immediately before fill and fail closed for missing, malformed, insecure, or unmapped state.
- The broker verifies the exact registered origin, fills approved selectors through CDP, and returns only filled-field metadata.
- Browser-cli never opens the credential store, accepts credential values for this path, or submits the form during broker fill. The agent clicks submit; MFA and CAPTCHA remain user-driven.

## Deployment

`./deploy.sh` builds the release binary, installs `$HOME/.cargo/bin/browser-cli`, and verifies the installed hash matches the build artifact.

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
