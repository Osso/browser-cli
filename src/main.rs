mod cdp;
mod commands;
mod snapshot;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
    Open { url: String },
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
    Click { selector: String },
    /// Type text into an element
    Type { selector: String, text: String },
    /// Clear and fill an element
    Fill { selector: String, text: String },
    /// Press a key
    #[command(visible_alias = "key")]
    Press { key: String },
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
    Eval { script: String },
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
    /// Get page accessibility/React tree snapshot
    Snapshot {
        /// Only include interactive elements
        #[arg(short, long)]
        interactive: bool,
        /// Remove structural elements without meaningful content
        #[arg(short, long)]
        compact: bool,
        /// Use React fiber tree instead of ARIA tree
        #[arg(short, long)]
        react: bool,
        /// Maximum tree depth
        #[arg(short, long)]
        depth: Option<usize>,
        /// Filter by component/element name (substring or glob with *)
        #[arg(short, long)]
        filter: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum GetCommand {
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
pub enum TabsCommand {
    /// List open tabs
    List,
    /// Open new tab
    New { url: Option<String> },
    /// Close tab
    Close { index: Option<usize> },
    /// Switch to tab by index
    Switch { index: usize },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let port = cli.port;
    let json = cli.json;

    match cli.command {
        Command::Open { url } => commands::cmd_open(port, url, json).await,
        Command::Back => commands::cmd_simple_page(port, "Page.goBack", "Back").await,
        Command::Forward => commands::cmd_simple_page(port, "Page.goForward", "Forward").await,
        Command::Reload => commands::cmd_simple_page(port, "Page.reload", "Reloaded").await,
        Command::Close => commands::cmd_simple_page(port, "Page.close", "Closed").await,
        Command::Click { selector } => commands::cmd_click(port, &selector).await,
        Command::Type { selector, text } => commands::cmd_type(port, &selector, &text).await,
        Command::Fill { selector, text } => commands::cmd_fill(port, &selector, &text).await,
        Command::Press { key } => commands::cmd_press(port, &key).await,
        Command::Screenshot { path, full } => commands::cmd_screenshot(port, &path, full).await,
        Command::Eval { script } => commands::cmd_eval(port, &script, json).await,
        Command::Get { what } => commands::cmd_get(port, &what, json).await,
        Command::Tabs { action } => commands::cmd_tabs(port, &action, json).await,
        Command::Wait { target, url, load } => commands::cmd_wait(port, target, url, load).await,
        Command::Snapshot {
            interactive,
            compact,
            react,
            depth,
            filter,
        } => commands::cmd_snapshot(port, interactive, compact, react, depth, filter).await,
    }
}
