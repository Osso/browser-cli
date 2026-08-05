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

- Socket/protocol transport belongs to published `secrets-broker-client` v0.3.0, pinned by exact git revision `9439cfa62098a6213daa466f69449bc8a99805fc`.
- `broker.rs` retains exact-host scope mapping and delegates transport to the shared client; `main.rs` retains CDP active-target discovery and passes only scope plus target ID.
- `broker-unlock --scope SCOPE` requests optional human approval through authd. `--current-origin` resolves the active tab's exact HTTPS hostname through `$XDG_CONFIG_HOME/browser-cli/credential-scopes.json` or `~/.config/browser-cli/credential-scopes.json`.
- The mapping file is a mode-`0600`, current-user-owned regular JSON file from lowercase exact hostname to broker scope. Never add credentials, wildcards, suffix matching, symlinks, or permissive modes.
- When `--scope` and `--current-origin` are combined, require exact equality. Fail closed for missing, malformed, insecure, unmapped, symlinked, non-regular, foreign-owned, or permissive modes.
- `broker-fill` first requests the fill. An already-authorized request skips unlock. On `Locked`, request approval once for the resolved exact scope and retry the fill exactly once. Approval denial, approval unavailability, or a second `Locked` response fails closed; never retry approval or fill again.
- The broker verifies the exact registered origin, fills approved selectors through CDP, and returns only filled-count metadata.
- Browser-cli never opens the credential store, accepts credential values for this path, or submits the form during broker fill. The operator clicks submit; MFA and CAPTCHA remain user-driven.


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
