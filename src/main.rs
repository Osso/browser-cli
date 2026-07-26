mod broker;
mod cdp;
mod commands;
mod runtime;
mod snapshot;
#[cfg(test)]
mod snapshot_tests;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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

    /// Chromium user data directory used when auto-starting the browser
    #[arg(long)]
    user_data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Request approval to unlock a Secrets Broker scope
    BrokerUnlock {
        #[arg(long)]
        scope: String,
        #[arg(long, default_value = "/run/secrets-broker/broker.sock")]
        socket: PathBuf,
    },
    /// Fill approved credentials through Secrets Broker
    BrokerFill {
        #[arg(long)]
        scope: String,
        #[arg(long, default_value = "/run/secrets-broker/broker.sock")]
        socket: PathBuf,
    },
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
    /// Clear and fill an element. For file inputs, attaches the file path.
    Fill { selector: String, text: String },
    /// Attach one or more files to a file input
    Attach {
        selector: String,
        files: Vec<String>,
    },
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
        /// Dump full DOM tree (all elements, not just accessible/React)
        #[arg(long)]
        full: bool,
        /// Minimized DOM tree (collapses wrapper chains)
        #[arg(long)]
        mini: bool,
    },
    /// Inspect Runtime console and exception events
    Runtime {
        #[command(subcommand)]
        action: RuntimeCommand,
    },
}

#[derive(Subcommand)]
pub enum RuntimeCommand {
    /// Capture console API calls
    Console {
        /// Reload the page before collecting events
        #[arg(long)]
        reload: bool,
        /// Milliseconds to collect events
        #[arg(long, default_value_t = 1500)]
        wait_ms: u64,
    },
    /// Capture runtime exceptions
    Exceptions {
        /// Reload the page before collecting events
        #[arg(long)]
        reload: bool,
        /// Milliseconds to collect events
        #[arg(long, default_value_t = 1500)]
        wait_ms: u64,
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
    let browser = cdp::BrowserConfig {
        port: cli.port,
        user_data_dir: cli.user_data_dir,
    };
    let json = cli.json;

    match cli.command {
        Command::BrokerUnlock { scope, socket } => {
            let scopes = broker::unlock(&socket, &scope)?;
            println!("Unlocked {}", scopes.join(", "));
            Ok(())
        }
        Command::BrokerFill { scope, socket } => {
            let target_id = cdp::active_target_id(&browser).await?;
            let filled = broker::fill(&socket, &scope, &target_id)?;
            println!("Filled {filled} credential fields");
            Ok(())
        }
        Command::Open { url } => commands::cmd_open(&browser, url, json).await,
        Command::Back => commands::cmd_simple_page(&browser, "Page.goBack", "Back").await,
        Command::Forward => commands::cmd_simple_page(&browser, "Page.goForward", "Forward").await,
        Command::Reload => commands::cmd_simple_page(&browser, "Page.reload", "Reloaded").await,
        Command::Close => commands::cmd_simple_page(&browser, "Page.close", "Closed").await,
        Command::Click { selector } => commands::cmd_click(&browser, &selector).await,
        Command::Type { selector, text } => commands::cmd_type(&browser, &selector, &text).await,
        Command::Fill { selector, text } => commands::cmd_fill(&browser, &selector, &text).await,
        Command::Attach { selector, files } => {
            commands::cmd_attach(&browser, &selector, &files).await
        }
        Command::Press { key } => commands::cmd_press(&browser, &key).await,
        Command::Screenshot { path, full } => commands::cmd_screenshot(&browser, &path, full).await,
        Command::Eval { script } => commands::cmd_eval(&browser, &script, json).await,
        Command::Get { what } => commands::cmd_get(&browser, &what, json).await,
        Command::Tabs { action } => commands::cmd_tabs(&browser, &action, json).await,
        Command::Wait { target, url, load } => {
            commands::cmd_wait(&browser, target, url, load).await
        }
        Command::Snapshot {
            interactive,
            compact,
            react,
            depth,
            filter,
            full,
            mini,
        } => {
            let options = snapshot::SnapshotOptions {
                interactive,
                compact,
                react,
                max_depth: depth,
                filter,
                full,
                mini,
            };
            commands::cmd_snapshot(&browser, options).await
        }
        Command::Runtime { action } => runtime::cmd_runtime(&browser, &action, json).await,
    }
}
