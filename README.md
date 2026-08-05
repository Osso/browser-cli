# browser-cli

Browser automation CLI using Chrome DevTools Protocol. Connects directly to Chrome via CDP without requiring Node.js or Playwright.

## Installation

### From releases

Download the latest binary from [releases](https://github.com/Osso/browser-cli/releases):

```bash
# Linux amd64
curl -L https://github.com/Osso/browser-cli/releases/latest/download/browser-cli-linux-amd64 -o browser-cli
chmod +x browser-cli
sudo mv browser-cli /usr/local/bin/
```

### From source

```bash
cargo install --git https://github.com/Osso/browser-cli
```

## Prerequisites

`browser-cli` connects to an existing Chrome/Chromium instance with remote
debugging enabled when one is available. If none is listening, it starts the
browser itself. To start one manually:

```bash
google-chrome-stable --remote-debugging-port=9222
```

When `browser-cli` starts a browser itself, it uses
`/home/osso/.config/chromium` when `--user-data-dir` is omitted. Use the global
`--user-data-dir PATH` option to select another profile:

```bash
browser-cli --user-data-dir ~/.local/share/browser-cli/profile open https://example.com
```

Remote debugging exposes the connected profile's authenticated browser state
to automation. Before asking `browser-cli` to auto-start a browser with a
profile, fully close every Chromium process using that profile; concurrent
access can corrupt or lock it.

When `browser-cli` starts Chrome itself, it uses native Chromium Wayland when
`WAYLAND_DISPLAY` is non-empty. If desktop variables are missing (for example,
a restarted agent process), it discovers an active `wayland-*` socket under
`XDG_RUNTIME_DIR` or `/run/user/<uid>` and supplies the required environment to
Chromium. It also restores `DBUS_SESSION_BUS_ADDRESS` from the runtime bus when
the launching process lacks desktop-session variables, allowing Chromium to
reach keyring and native desktop integrations. An explicit non-empty
`WAYLAND_DISPLAY` takes precedence. Otherwise, a non-empty `DISPLAY` preserves
X11 behavior and disables socket discovery.

## Secrets Broker integration

`browser-cli` uses published `secrets-broker-client` v0.3.0, pinned to exact git revision `9439cfa62098a6213daa466f69449bc8a99805fc`, for socket framing and protocol transport. It never opens the credential store or accepts credential values as arguments. The broker service listens on `/run/secrets-broker/broker.sock` by default.

```bash
browser-cli broker-unlock --scope browser:citi
browser-cli broker-fill --scope browser:citi
```

`broker-fill` discovers the active CDP target and retains browser-cli's exact-host mapping. With `--current-origin`, create a mode-`0600` `credential-scopes.json` mapping exact lowercase HTTPS hostnames to broker scopes; it never contains credentials:

```json
{
  "online.citi.com": "browser:citi"
}
```

```bash
browser-cli broker-unlock --current-origin
browser-cli broker-fill --current-origin
```

`$XDG_CONFIG_HOME/browser-cli/credential-scopes.json` takes precedence when `XDG_CONFIG_HOME` is set. Use `--scope-config PATH` to select another mapping file. Supplying both `--current-origin` and `--scope` requires an exact match. Unmapped hosts, insecure URLs, symlinks, non-regular files, files not owned by the current user, malformed mappings, and modes other than `0600` fail closed.

`broker-fill` sends only the resolved scope and CDP target ID through the shared client. If the broker is already authorized, it performs one fill request and does not unlock. If the broker replies `Locked`, browser-cli requests approval once for that exact resolved scope, then retries the fill exactly once. Approval denial, approval unavailability, and a second `Locked` response fail closed without another fill or unlock attempt. `broker-unlock` remains an optional explicit approval command.

The broker verifies the exact registered browser origin, fills registered selectors through CDP, and returns only filled-count metadata. Credentials never enter browser-cli output, arguments, or responses. Broker fill does not submit forms; the browser operator clicks the sign-in control, and MFA/CAPTCHA remain user-driven. Use `--socket PATH` only when the broker is deployed at a different socket path.

`click` resolves the target rectangle, rejects missing or zero-size elements, and dispatches `mouseMoved`, `mousePressed`, and `mouseReleased` through CDP at the element center. This supports custom controls such as browser-rendered listboxes that do not respond to DOM `.click()`.

Deploy this revision with:

```bash
./deploy.sh
```

The script builds the release binary, installs it at `$HOME/.cargo/bin/browser-cli`, and verifies the installed hash matches the build artifact.

## Usage

### Navigation

```bash
browser-cli open <url>       # Navigate (aliases: goto, navigate)
browser-cli back             # Go back
browser-cli forward          # Go forward
browser-cli reload           # Reload page
browser-cli close            # Close tab (aliases: quit, exit)
```

### Interactions

```bash
browser-cli click <selector>           # Real CDP pointer click at visible element center
browser-cli type <selector> <text>     # Append text to element
browser-cli fill <selector> <text>     # Clear and fill element
browser-cli attach <selector> <file>   # Attach file(s) to input[type=file]
browser-cli press <key>                # Press key (alias: key)
```

### Get information

```bash
browser-cli get title                  # Get page title
browser-cli get url                    # Get current URL
browser-cli get text [selector]        # Get element/page text
browser-cli get html <selector>        # Get innerHTML
browser-cli get value <selector>       # Get input value
browser-cli get attr <selector> <name> # Get attribute
browser-cli get count <selector>       # Count matching elements
```

### Tab management

```bash
browser-cli tabs list                  # List open tabs
browser-cli tabs new [url]             # Open new tab
browser-cli tabs close [index]         # Close tab (default: 0)
browser-cli tabs switch <index>        # Switch to tab
```

### Screenshots

```bash
browser-cli screenshot                 # Save to /tmp/claude/screenshot.jpg
browser-cli screenshot path.jpg        # Save to path
browser-cli screenshot --full path.jpg # Full page
```

### Wait

```bash
browser-cli wait 2000                  # Wait milliseconds
browser-cli wait <selector>            # Wait for element
```

### JavaScript

```bash
browser-cli eval "document.title"      # Run JavaScript
```

### Runtime diagnostics

```bash
browser-cli --json runtime console --reload       # Capture console API calls during reload
browser-cli --json runtime exceptions --reload    # Capture runtime exceptions during reload
browser-cli --json runtime console --wait-ms 3000 # Collect future console events for 3s
```

### Global options

Global options go before the subcommand:

```bash
browser-cli --port 9222 ...            # CDP port (default: 9222)
browser-cli --json ...                 # JSON output
browser-cli --user-data-dir PATH ...   # Chromium profile for auto-started browser
```

When `--user-data-dir` is omitted, an auto-started browser uses
`/home/osso/.config/chromium`. Never use the same user-data directory for
concurrently running Chromium processes.

## Example

```bash
# Start Chrome with remote debugging
google-chrome-stable --remote-debugging-port=9222 &

# Navigate and interact
browser-cli open https://example.com
browser-cli fill "input[name=search]" "hello world"
browser-cli attach "input[type=file]" ./document.pdf
browser-cli click "button[type=submit]"
browser-cli wait 2000
browser-cli screenshot result.jpg
browser-cli get title
```

## License

MIT
