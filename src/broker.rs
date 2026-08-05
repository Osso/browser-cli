use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct Request<'a> {
    version: u16,
    request_id: &'a str,
    operation: Operation<'a>,
}
#[derive(Serialize)]
#[serde(tag = "type", content = "parameters", rename_all = "snake_case")]
enum Operation<'a> {
    Unlock(ScopeRequest<'a>),
    BrowserFill(BrowserFill<'a>),
}
#[derive(Serialize)]
struct ScopeRequest<'a> {
    scope: &'a str,
}
#[derive(Serialize)]
struct BrowserFill<'a> {
    scope: &'a str,
    target_id: &'a str,
}
#[derive(Deserialize)]
struct Response {
    #[allow(dead_code)]
    version: u16,
    #[allow(dead_code)]
    request_id: String,
    payload: Payload,
}
#[derive(Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
enum Payload {
    Unlocked { scopes: Vec<String> },
    BrowserFilled { filled_count: usize },
    Error(ErrorResponse),
}
#[derive(Deserialize)]
struct ErrorResponse {
    message: String,
}

pub fn unlock(socket: &Path, scope: &str) -> Result<Vec<String>> {
    let request = Request {
        version: 1,
        request_id: "browser-cli",
        operation: Operation::Unlock(ScopeRequest { scope }),
    };
    match exchange(socket, &request)? {
        Payload::Unlocked { scopes } => Ok(scopes),
        Payload::Error(error) => bail!("broker denied unlock: {}", error.message),
        _ => bail!("broker returned an unexpected unlock response"),
    }
}

pub fn fill(socket: &Path, scope: &str, target_id: &str) -> Result<usize> {
    let request = Request {
        version: 1,
        request_id: "browser-cli",
        operation: Operation::BrowserFill(BrowserFill { scope, target_id }),
    };
    match exchange(socket, &request)? {
        Payload::BrowserFilled { filled_count } => Ok(filled_count),
        Payload::Error(error) => bail!("broker denied fill: {}", error.message),
        _ => bail!("broker returned an unexpected fill response"),
    }
}

pub fn default_scope_config_path() -> Result<PathBuf> {
    let xdg_config_home = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    scope_config_path(xdg_config_home.as_deref(), home.as_deref())
}

fn scope_config_path(xdg_config_home: Option<&str>, home: Option<&str>) -> Result<PathBuf> {
    if let Some(config_home) = xdg_config_home.filter(|value| !value.is_empty()) {
        return Ok(Path::new(config_home)
            .join("browser-cli")
            .join("credential-scopes.json"));
    }
    let home = home
        .filter(|value| !value.is_empty())
        .context("cannot resolve credential scope config: HOME is not set")?;
    Ok(Path::new(home)
        .join(".config")
        .join("browser-cli")
        .join("credential-scopes.json"))
}

pub fn resolve_scope_for_url(
    config_path: &Path,
    current_url: &str,
    explicit_scope: Option<&str>,
) -> Result<String> {
    validate_scope_config_file(config_path)?;
    let mappings: HashMap<String, String> = serde_json::from_str(
        &fs::read_to_string(config_path)
            .with_context(|| format!("read credential scope config {}", config_path.display()))?,
    )
    .with_context(|| format!("parse credential scope config {}", config_path.display()))?;
    let url = reqwest::Url::parse(current_url).context("parse active browser URL")?;
    if url.scheme() != "https" {
        bail!("mapped broker authentication requires an HTTPS origin");
    }
    let hostname = url
        .host_str()
        .context("active browser URL has no hostname")?
        .to_ascii_lowercase();
    let mapped_scope = mappings
        .get(&hostname)
        .filter(|scope| !scope.trim().is_empty())
        .with_context(|| format!("hostname {hostname} is not mapped to a broker scope"))?;
    if let Some(scope) = explicit_scope
        && scope != mapped_scope
    {
        bail!("explicit broker scope {scope} does not match mapped scope {mapped_scope}");
    }
    Ok(mapped_scope.clone())
}

fn validate_scope_config_file(config_path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(config_path).with_context(|| {
        format!(
            "read credential scope config metadata {}",
            config_path.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        bail!("credential scope config must be a regular file");
    }
    if metadata.mode() & 0o777 != 0o600 {
        bail!("credential scope config must have mode 0600");
    }
    let effective_user = unsafe { libc::geteuid() };
    if metadata.uid() != effective_user {
        bail!("credential scope config must be owned by the current user");
    }
    Ok(())
}

fn exchange(socket: &Path, request: &Request<'_>) -> Result<Payload> {
    let bytes = rmp_serde::to_vec_named(request)?;
    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("connect broker socket {}", socket.display()))?;
    stream.write_all(&bytes)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    Ok(rmp_serde::from_slice::<Response>(&response)?.payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    #[derive(Deserialize)]
    struct BrokerRequest {
        version: u16,
        request_id: String,
        operation: BrokerOperation,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", content = "parameters", rename_all = "snake_case")]
    enum BrokerOperation {
        BrowserFill(BrokerBrowserFill),
    }

    #[derive(Deserialize)]
    struct BrokerBrowserFill {
        scope: String,
        target_id: String,
    }

    #[derive(Serialize)]
    struct BrokerResponse<'a> {
        version: u16,
        request_id: &'a str,
        payload: BrokerPayload,
    }

    #[derive(Serialize)]
    #[serde(tag = "type", content = "data", rename_all = "snake_case")]
    enum BrokerPayload {
        BrowserFilled { filled_count: usize },
    }

    #[test]
    fn round_trips_actual_broker_browser_fill_without_credentials() {
        let socket = std::env::temp_dir().join(format!(
            "browser-cli-broker-test-{}-{:?}.sock",
            std::process::id(),
            thread::current().id()
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            assert!(!bytes.windows(8).any(|value| value == b"username"));
            assert!(!bytes.windows(8).any(|value| value == b"password"));

            let request: BrokerRequest = rmp_serde::from_slice(&bytes).unwrap();
            assert_eq!(request.version, 1);
            assert_eq!(request.request_id, "browser-cli");
            let BrokerOperation::BrowserFill(fill) = request.operation;
            assert_eq!(fill.scope, "browser:citi");
            assert_eq!(fill.target_id, "citi-login");

            let response = BrokerResponse {
                version: 1,
                request_id: "browser-cli",
                payload: BrokerPayload::BrowserFilled { filled_count: 2 },
            };
            stream
                .write_all(&rmp_serde::to_vec_named(&response).unwrap())
                .unwrap();
        });

        assert_eq!(fill(&socket, "browser:citi", "citi-login").unwrap(), 2);
        server.join().unwrap();
        std::fs::remove_file(socket).unwrap();
    }

    fn write_scope_config(contents: &str, mode: u32) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "browser-cli-scope-config-{}-{:?}.json",
            std::process::id(),
            thread::current().id()
        ));
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn resolves_exact_https_hostname_and_matching_explicit_scope() {
        let path = write_scope_config(r#"{"online.citi.com":"browser:citi"}"#, 0o600);

        assert_eq!(
            resolve_scope_for_url(&path, "https://online.citi.com/login", Some("browser:citi"))
                .unwrap(),
            "browser:citi"
        );

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_suffix_insecure_mismatch_and_permissive_config() {
        let path = write_scope_config(r#"{"online.citi.com":"browser:citi"}"#, 0o600);

        assert!(
            resolve_scope_for_url(&path, "https://evilonline.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("not mapped")
        );
        assert!(
            resolve_scope_for_url(&path, "http://online.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("HTTPS")
        );
        assert!(
            resolve_scope_for_url(
                &path,
                "https://online.citi.com/login",
                Some("browser:other")
            )
            .unwrap_err()
            .to_string()
            .contains("does not match")
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            resolve_scope_for_url(&path, "https://online.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("0600")
        );

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_missing_malformed_and_symlinked_config() {
        let missing = std::env::temp_dir().join(format!(
            "browser-cli-missing-scope-config-{}-{:?}.json",
            std::process::id(),
            thread::current().id()
        ));
        assert!(
            resolve_scope_for_url(&missing, "https://online.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("metadata")
        );

        let malformed = write_scope_config("not json", 0o600);
        assert!(
            resolve_scope_for_url(&malformed, "https://online.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("parse")
        );
        std::fs::remove_file(&malformed).unwrap();

        let target = write_scope_config(r#"{"online.citi.com":"browser:citi"}"#, 0o600);
        let symlink = target.with_extension("symlink.json");
        std::os::unix::fs::symlink(&target, &symlink).unwrap();
        assert!(
            resolve_scope_for_url(&symlink, "https://online.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
        std::fs::remove_file(symlink).unwrap();
        std::fs::remove_file(target).unwrap();
    }

    #[test]
    fn chooses_xdg_then_home_scope_config_path() {
        assert_eq!(
            scope_config_path(Some("/tmp/xdg"), Some("/home/test")).unwrap(),
            PathBuf::from("/tmp/xdg/browser-cli/credential-scopes.json")
        );
        assert_eq!(
            scope_config_path(None, Some("/home/test")).unwrap(),
            PathBuf::from("/home/test/.config/browser-cli/credential-scopes.json")
        );
        assert!(scope_config_path(None, None).is_err());
    }
}
