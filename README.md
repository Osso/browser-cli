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

`browser-cli` can request a broker-approved browser credential fill without opening the credential store or accepting credential values as arguments. The broker service is installed at `/usr/bin/secrets-broker` and listens on `/run/secrets-broker/broker.sock`.

```bash
browser-cli broker-unlock --scope browser:citi
browser-cli broker-fill --scope browser:citi
```

`broker-unlock` requests human approval through authd. `broker-fill` identifies the active CDP target and sends only the scope and target ID to the broker. The broker verifies the target's exact registered origin, fills the registered selectors through CDP, and returns only the number of fields filled. It does not return credential values or submit the form. Use `--socket PATH` only when the broker is deployed at a different socket path.

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
browser-cli click <selector>           # Click element
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
