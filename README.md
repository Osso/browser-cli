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

Chrome must be running with remote debugging enabled:

```bash
google-chrome-stable --remote-debugging-port=9222
```

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

### Global options

```bash
browser-cli --port 9222 ...            # CDP port (default: 9222)
browser-cli --json ...                 # JSON output
```

## Example

```bash
# Start Chrome with remote debugging
google-chrome-stable --remote-debugging-port=9222 &

# Navigate and interact
browser-cli open https://example.com
browser-cli fill "input[name=search]" "hello world"
browser-cli click "button[type=submit]"
browser-cli wait 2000
browser-cli screenshot result.jpg
browser-cli get title
```

## License

MIT
