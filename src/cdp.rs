use anyhow::{Context, Result, anyhow};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tokio_tungstenite::tungstenite::Message;

#[cfg(unix)]
unsafe extern "C" {
    fn geteuid() -> u32;
    fn setsid() -> i32;
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct TargetJson {
    pub id: String,
    pub title: String,
    pub url: String,
    pub r#type: String,
    pub webSocketDebuggerUrl: Option<String>,
}

pub struct CdpConnection {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: i32,
}

impl CdpConnection {
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
        Ok(Self { ws, next_id: 1 })
    }

    pub async fn send(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({ "id": id, "method": method, "params": params });
        self.ws.send(Message::Text(msg.to_string())).await?;

        while let Some(resp) = self.recv().await? {
            if resp.get("id") != Some(&serde_json::json!(id)) {
                continue;
            }
            if let Some(error) = resp.get("error") {
                return Err(anyhow!("CDP error: {}", error));
            }
            return Ok(resp.get("result").cloned().unwrap_or(serde_json::json!({})));
        }
        Err(anyhow!("No response from CDP"))
    }

    pub async fn recv(&mut self) -> Result<Option<serde_json::Value>> {
        while let Some(msg) = self.ws.next().await {
            let Ok(Message::Text(text)) = msg else {
                continue;
            };
            let mut de = serde_json::Deserializer::from_str(&text);
            de.disable_recursion_limit();
            return Ok(Some(serde_json::Value::deserialize(&mut de)?));
        }
        Ok(None)
    }

    pub async fn eval(&mut self, expression: &str) -> Result<serde_json::Value> {
        let result = self
            .send(
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": expression,
                    "returnByValue": true
                }),
            )
            .await?;

        if let Some(value) = result.get("result").and_then(|r| r.get("value")) {
            Ok(value.clone())
        } else if let Some(desc) = result
            .get("result")
            .and_then(|r| r.get("description"))
            .and_then(|d| d.as_str())
        {
            Ok(serde_json::json!(desc))
        } else {
            Ok(serde_json::json!(null))
        }
    }
}

fn find_chrome_executable() -> Option<&'static str> {
    const CANDIDATES: &[&str] = &[
        "chromium-browser",
        "chromium",
        "google-chrome-stable",
        "google-chrome",
    ];
    CANDIDATES.iter().copied().find(|candidate| {
        Command::new("which")
            .arg(candidate)
            .output()
            .is_ok_and(|output| output.status.success())
    })
}

#[derive(Debug, PartialEq)]
struct WaylandSession {
    runtime_dir: PathBuf,
    display: String,
}

fn resolve_wayland_display<'a>(
    explicit_wayland: Option<&str>,
    x11_display: Option<&str>,
    runtime_entries: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    if let Some(display) = explicit_wayland.filter(|display| !display.is_empty()) {
        return Some(display.to_string());
    }
    if x11_display.is_some_and(|display| !display.is_empty()) {
        return None;
    }

    let mut displays = runtime_entries
        .into_iter()
        .filter(|name| {
            name.strip_prefix("wayland-").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
            })
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    displays.sort();
    displays.into_iter().next()
}

fn discover_wayland_session() -> Option<WaylandSession> {
    let explicit_wayland = std::env::var("WAYLAND_DISPLAY").ok();
    let x11_display = std::env::var("DISPLAY").ok();

    discover_wayland_session_with(
        explicit_wayland.as_deref(),
        x11_display.as_deref(),
        wayland_runtime_dir,
    )
}

fn discover_wayland_session_with(
    explicit_wayland: Option<&str>,
    x11_display: Option<&str>,
    read_runtime_dir: impl FnOnce() -> Option<PathBuf>,
) -> Option<WaylandSession> {
    if x11_display.is_some_and(|display| !display.is_empty())
        && explicit_wayland.is_none_or(|display| display.is_empty())
    {
        return None;
    }

    let runtime_dir = read_runtime_dir()?;
    let socket_names = wayland_socket_names(&runtime_dir);
    let display = resolve_wayland_display(
        explicit_wayland,
        x11_display,
        socket_names.iter().map(String::as_str),
    )?;

    Some(WaylandSession {
        runtime_dir,
        display,
    })
}

fn wayland_runtime_dir() -> Option<PathBuf> {
    let configured_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");

    #[cfg(unix)]
    let effective_user_id = Some(unsafe { geteuid() });
    #[cfg(not(unix))]
    let effective_user_id = None;

    select_wayland_runtime_dir(configured_runtime_dir, effective_user_id)
}

fn select_wayland_runtime_dir(
    configured_runtime_dir: Option<std::ffi::OsString>,
    effective_user_id: Option<u32>,
) -> Option<PathBuf> {
    if let Some(runtime_dir) = configured_runtime_dir.filter(|path| !path.is_empty()) {
        return Some(runtime_dir.into());
    }

    effective_user_id.map(|user_id| PathBuf::from(format!("/run/user/{user_id}")))
}

#[cfg(unix)]
fn wayland_socket_names(runtime_dir: &Path) -> Vec<String> {
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::net::UnixStream;

    let Ok(entries) = std::fs::read_dir(runtime_dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|file_type| file_type.is_socket())
        })
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let suffix = name.strip_prefix("wayland-")?;
            let is_wayland_display =
                !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit());
            is_wayland_display.then_some((name, entry.path()))
        })
        .filter(|(_, path)| UnixStream::connect(path).is_ok())
        .map(|(name, _)| name)
        .collect()
}

#[cfg(not(unix))]
fn wayland_socket_names(_runtime_dir: &Path) -> Vec<String> {
    Vec::new()
}

pub struct BrowserConfig {
    pub port: u16,
    pub user_data_dir: Option<PathBuf>,
}

fn chrome_launch_args(
    port: u16,
    wayland_display: Option<&str>,
    user_data_dir: Option<&Path>,
) -> Vec<OsString> {
    let data_dir = user_data_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/browser-cli-chrome-{}", port)));
    let mut user_data_argument = OsString::from("--user-data-dir=");
    user_data_argument.push(data_dir.as_os_str());
    let mut args = vec![
        OsString::from(format!("--remote-debugging-port={}", port)),
        user_data_argument,
        OsString::from("--no-first-run"),
        OsString::from("--no-default-browser-check"),
        // Suppress the "Restore pages? Chrome didn't shut down correctly" bubble
        // that appears when the profile was left dirty by a prior unclean exit.
        OsString::from("--disable-session-crashed-bubble"),
        OsString::from("--hide-crash-restore-bubble"),
    ];
    if wayland_display.is_some_and(|display| !display.is_empty()) {
        args.push(OsString::from("--ozone-platform=wayland"));
    }
    args.push(OsString::from("about:blank"));
    args
}

fn apply_wayland_session(command: &mut Command, session: Option<&WaylandSession>) {
    if let Some(session) = session {
        command
            .env("XDG_RUNTIME_DIR", &session.runtime_dir)
            .env("WAYLAND_DISPLAY", &session.display);
    }
}

fn start_chrome(config: &BrowserConfig) -> Result<()> {
    let chrome = find_chrome_executable().context("Chrome not found in PATH")?;
    let mut command = Command::new(chrome);
    detach_from_parent(&mut command);

    let wayland_session = discover_wayland_session();
    apply_wayland_session(&mut command, wayland_session.as_ref());

    command
        .args(chrome_launch_args(
            config.port,
            wayland_session
                .as_ref()
                .map(|session| session.display.as_str()),
            config.user_data_dir.as_deref(),
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to start Chrome")?;
    Ok(())
}

#[cfg(unix)]
fn detach_from_parent(command: &mut Command) {
    use std::io;
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            if setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

#[cfg(not(unix))]
fn detach_from_parent(_command: &mut Command) {}

async fn chrome_is_running(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/json/version", port);
    reqwest::get(&url).await.is_ok()
}

async fn get_all_targets(port: u16) -> Result<Vec<TargetJson>> {
    let url = format!("http://127.0.0.1:{}/json", port);
    let targets: Vec<TargetJson> = reqwest::get(&url)
        .await
        .context("Failed to connect to Chrome")?
        .json()
        .await?;
    Ok(targets
        .into_iter()
        .filter(|t| t.r#type == "page" && t.webSocketDebuggerUrl.is_some())
        .collect())
}

pub async fn create_new_tab(port: u16, url: &str) -> Result<TargetJson> {
    let endpoint = format!(
        "http://127.0.0.1:{}/json/new?{}",
        port,
        urlencoding::encode(url)
    );
    let target: TargetJson = reqwest::get(&endpoint)
        .await
        .context("Failed to create new tab")?
        .json()
        .await?;
    Ok(target)
}

pub async fn get_targets(config: &BrowserConfig) -> Result<Vec<TargetJson>> {
    if !chrome_is_running(config.port).await {
        eprintln!(
            "Starting Chrome with remote debugging on port {}...",
            config.port
        );
        start_chrome(config)?;

        for _ in 0..50 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            if chrome_is_running(config.port).await {
                break;
            }
        }

        if !chrome_is_running(config.port).await {
            anyhow::bail!("Chrome started but failed to connect after 5 seconds");
        }
    }

    let mut targets = get_all_targets(config.port).await?;
    if targets.is_empty() {
        let new_target = create_new_tab(config.port, "about:blank").await?;
        targets.push(new_target);
    }
    Ok(targets)
}

pub fn find_active_target(targets: &[TargetJson]) -> Result<&TargetJson> {
    targets
        .iter()
        .find(|t| !t.url.starts_with("about:") && !t.url.starts_with("chrome://"))
        .or(targets.first())
        .context("No pages found. Open a tab in Chrome first.")
}

/// Connect CDP to the active target
pub async fn connect_active(config: &BrowserConfig) -> Result<CdpConnection> {
    let targets = get_targets(config).await?;
    let target = find_active_target(&targets)?;
    let ws_url = target.webSocketDebuggerUrl.as_ref().unwrap();
    CdpConnection::connect(ws_url).await
}

#[cfg(test)]
mod tests {
    use super::{
        WaylandSession, apply_wayland_session, chrome_launch_args, discover_wayland_session_with,
        resolve_wayland_display, select_wayland_runtime_dir, wayland_socket_names,
    };
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn create_test_runtime_dir() -> PathBuf {
        let runtime_dir = std::env::temp_dir().join(format!(
            "browser-cli-wayland-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&runtime_dir).expect("create test runtime directory");
        runtime_dir
    }

    #[test]
    fn chrome_launch_args_include_debug_port_and_profile() {
        let args = chrome_launch_args(9222, None, None);

        assert!(args.contains(&OsString::from("--remote-debugging-port=9222")));
        assert!(args.contains(&OsString::from(
            "--user-data-dir=/tmp/browser-cli-chrome-9222"
        )));
        assert!(args.contains(&OsString::from("--no-first-run")));
        assert!(args.contains(&OsString::from("--no-default-browser-check")));
        assert_eq!(
            args.last().and_then(|arg| arg.to_str()),
            Some("about:blank")
        );
    }

    #[test]
    fn chrome_launch_args_use_explicit_profile() {
        let args = chrome_launch_args(9222, None, Some(Path::new("/home/osso/.config/chromium")));

        assert!(args.contains(&OsString::from(
            "--user-data-dir=/home/osso/.config/chromium"
        )));
        assert!(!args.contains(&OsString::from(
            "--user-data-dir=/tmp/browser-cli-chrome-9222"
        )));
    }

    #[test]
    fn chrome_launch_args_preserve_non_utf8_profile_path() {
        let path = PathBuf::from(OsString::from_vec(b"/tmp/profile-\xff".to_vec()));

        let args = chrome_launch_args(9222, None, Some(&path));
        let profile_arg = args
            .iter()
            .find(|arg| arg.as_os_str().as_bytes().starts_with(b"--user-data-dir="))
            .expect("profile argument");

        assert_eq!(
            profile_arg.as_os_str().as_bytes(),
            b"--user-data-dir=/tmp/profile-\xff"
        );
    }

    #[test]
    fn chrome_launch_args_use_native_wayland_when_available() {
        let args = chrome_launch_args(9222, Some("wayland-1"), None);

        assert!(args.contains(&OsString::from("--ozone-platform=wayland")));
    }

    #[test]
    fn chrome_launch_args_preserve_x11_without_wayland() {
        let args = chrome_launch_args(9222, None, None);

        assert!(
            !args
                .iter()
                .any(|arg| arg.to_string_lossy().starts_with("--ozone-platform="))
        );
    }

    #[test]
    fn chrome_launch_args_preserve_x11_for_empty_wayland_display() {
        let args = chrome_launch_args(9222, Some(""), None);

        assert!(
            !args
                .iter()
                .any(|arg| arg.to_string_lossy().starts_with("--ozone-platform="))
        );
    }

    #[test]
    fn configured_runtime_directory_wins_over_user_fallback() {
        let runtime_dir = select_wayland_runtime_dir(Some("/custom/runtime".into()), Some(1000));

        assert_eq!(runtime_dir.as_deref(), Some(Path::new("/custom/runtime")));
    }

    #[test]
    fn missing_runtime_directory_uses_effective_user_directory() {
        let runtime_dir = select_wayland_runtime_dir(None, Some(1000));

        assert_eq!(runtime_dir.as_deref(), Some(Path::new("/run/user/1000")));
    }

    #[test]
    fn wayland_session_is_added_to_chromium_environment() {
        let mut command = Command::new("chromium");
        let session = WaylandSession {
            runtime_dir: PathBuf::from("/run/user/1000"),
            display: "wayland-1".to_string(),
        };

        apply_wayland_session(&mut command, Some(&session));
        let environment = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(ToOwned::to_owned)))
            .collect::<Vec<_>>();

        assert!(environment.contains(&("XDG_RUNTIME_DIR".into(), Some("/run/user/1000".into()))));
        assert!(environment.contains(&("WAYLAND_DISPLAY".into(), Some("wayland-1".into()))));
    }

    #[test]
    fn explicit_wayland_display_wins_over_runtime_discovery() {
        let display =
            resolve_wayland_display(Some("wayland-9"), None, ["wayland-1", "wayland-1.lock"]);

        assert_eq!(display.as_deref(), Some("wayland-9"));
    }

    #[test]
    fn x11_display_disables_wayland_socket_discovery() {
        let runtime_dir = create_test_runtime_dir();
        let unrelated_listener =
            UnixListener::bind(runtime_dir.join("pipewire-0")).expect("bind unrelated socket");
        unrelated_listener
            .set_nonblocking(true)
            .expect("make unrelated socket nonblocking");

        let session = discover_wayland_session_with(None, Some(":0"), || Some(runtime_dir.clone()));

        assert_eq!(session, None);
        let accept_error = unrelated_listener
            .accept()
            .expect_err("X11 selection must not connect to runtime sockets");
        assert_eq!(accept_error.kind(), std::io::ErrorKind::WouldBlock);
        drop(unrelated_listener);
        std::fs::remove_dir_all(runtime_dir).expect("remove test runtime directory");
    }

    #[test]
    fn missing_desktop_environment_uses_wayland_runtime_socket() {
        let display =
            resolve_wayland_display(None, None, ["wayland-1.lock", "pipewire-0", "wayland-1"]);

        assert_eq!(display.as_deref(), Some("wayland-1"));
    }

    #[test]
    fn runtime_discovery_ignores_non_socket_names() {
        let display =
            resolve_wayland_display(None, None, ["wayland-1.lock", "niri.wayland-1.sock"]);

        assert_eq!(display, None);
    }

    #[test]
    fn runtime_discovery_ignores_stale_wayland_sockets() {
        let runtime_dir = create_test_runtime_dir();
        let stale_path = runtime_dir.join("wayland-0");
        let stale_listener = UnixListener::bind(&stale_path).expect("bind stale socket");
        drop(stale_listener);
        let live_listener =
            UnixListener::bind(runtime_dir.join("wayland-1")).expect("bind live socket");
        let unrelated_listener =
            UnixListener::bind(runtime_dir.join("pipewire-0")).expect("bind unrelated socket");
        unrelated_listener
            .set_nonblocking(true)
            .expect("make unrelated socket nonblocking");

        let names = wayland_socket_names(&runtime_dir);

        assert_eq!(names, vec!["wayland-1"]);
        let accept_error = unrelated_listener
            .accept()
            .expect_err("discovery must not connect to unrelated sockets");
        assert_eq!(accept_error.kind(), std::io::ErrorKind::WouldBlock);
        drop(live_listener);
        drop(unrelated_listener);
        std::fs::remove_dir_all(runtime_dir).expect("remove test runtime directory");
    }
}
