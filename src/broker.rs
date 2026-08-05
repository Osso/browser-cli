use anyhow::{Context, Result, anyhow, bail};
use secrets_broker_client::{Client, ClientError};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub fn unlock(socket: &Path, scope: &str) -> Result<Vec<String>> {
    let change = Client::new(socket)
        .unlock(scope)
        .map_err(|error| anyhow!("unlock broker scope: {error}"))?;
    Ok(change.scopes)
}

pub fn fill(socket: &Path, scope: &str, target_id: &str) -> Result<usize> {
    let client = Client::new(socket);
    match client.request_browser_fill(scope, target_id) {
        Ok(result) => Ok(result.filled_count),
        Err(ClientError::Locked) => {
            client
                .unlock(scope)
                .map_err(|error| anyhow!("request browser broker approval: {error}"))?;
            retry_browser_fill_after_approval(&client, scope, target_id)
        }
        Err(error) => Err(anyhow!("request browser credential fill: {error}")),
    }
}

fn retry_browser_fill_after_approval(
    client: &Client,
    scope: &str,
    target_id: &str,
) -> Result<usize> {
    match client.request_browser_fill(scope, target_id) {
        Ok(result) => Ok(result.filled_count),
        Err(ClientError::Locked) => bail!("broker access remained locked after approval"),
        Err(error) => Err(anyhow!(
            "retry browser credential fill after approval: {error}"
        )),
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

pub fn read_mapped_scope_for_url(
    config_path: &Path,
    current_url: &str,
    explicit_scope: Option<&str>,
) -> Result<String> {
    read_and_validate_scope_config_metadata(config_path)?;
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

fn read_and_validate_scope_config_metadata(config_path: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    #[derive(Debug, Deserialize)]
    struct BrokerRequest {
        version: u16,
        request_id: String,
        operation: BrokerOperation,
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "type", content = "parameters", rename_all = "snake_case")]
    enum BrokerOperation {
        Unlock(BrokerScopeRequest),
        BrowserFill(BrokerBrowserFill),
    }

    #[derive(Debug, Deserialize)]
    struct BrokerScopeRequest {
        scope: String,
    }

    #[derive(Debug, Deserialize)]
    struct BrokerBrowserFill {
        scope: String,
        target_id: String,
    }

    #[derive(Serialize)]
    struct BrokerResponse {
        version: u16,
        request_id: String,
        payload: BrokerPayload,
    }

    #[derive(Serialize)]
    #[serde(tag = "type", content = "data", rename_all = "snake_case")]
    enum BrokerPayload {
        Unlocked { scopes: Vec<String> },
        BrowserFilled { filled_count: usize },
        Error(BrokerErrorResponse),
    }

    #[derive(Serialize)]
    struct BrokerErrorResponse {
        code: BrokerErrorCode,
        message: String,
    }

    #[derive(Clone, Copy, Serialize)]
    #[serde(rename_all = "snake_case")]
    enum BrokerErrorCode {
        ApprovalDenied,
        Locked,
        Unavailable,
    }

    fn broker_socket_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "browser-cli-broker-test-{}-{:?}.sock",
            std::process::id(),
            thread::current().id()
        ))
    }

    fn spawn_broker_sequence(
        socket: &Path,
        responses: Vec<BrokerPayload>,
    ) -> thread::JoinHandle<Vec<BrokerRequest>> {
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        thread::spawn(move || {
            let mut requests = Vec::new();
            for payload in responses {
                let Some(mut stream) = accept_broker_connection(&listener) else {
                    break;
                };
                let mut encoded = Vec::new();
                stream.read_to_end(&mut encoded).unwrap();
                assert!(!encoded.windows(8).any(|value| value == b"username"));
                assert!(!encoded.windows(8).any(|value| value == b"password"));

                let request: BrokerRequest = rmp_serde::from_slice(&encoded).unwrap();
                assert_eq!(request.version, 1);
                assert!(!request.request_id.is_empty());
                let response = BrokerResponse {
                    version: 1,
                    request_id: request.request_id.clone(),
                    payload,
                };
                stream
                    .write_all(&rmp_serde::to_vec_named(&response).unwrap())
                    .unwrap();
                requests.push(request);
            }
            requests
        })
    }

    fn accept_broker_connection(listener: &UnixListener) -> Option<std::os::unix::net::UnixStream> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        loop {
            match listener.accept() {
                Ok((stream, _)) => return Some(stream),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return None;
                    }
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("accept broker connection: {error}"),
            }
        }
    }

    fn assert_fill_request(request: &BrokerRequest, scope: &str, target_id: &str) {
        let BrokerOperation::BrowserFill(fill) = &request.operation else {
            panic!("expected browser-fill request");
        };
        assert_eq!(fill.scope, scope);
        assert_eq!(fill.target_id, target_id);
    }

    fn assert_unlock_request(request: &BrokerRequest, scope: &str) {
        let BrokerOperation::Unlock(unlock) = &request.operation else {
            panic!("expected unlock request");
        };
        assert_eq!(unlock.scope, scope);
    }

    fn locked_response(message: &str) -> BrokerPayload {
        BrokerPayload::Error(BrokerErrorResponse {
            code: BrokerErrorCode::Locked,
            message: message.to_string(),
        })
    }

    #[test]
    fn authorized_fill_skips_approval() {
        let socket = broker_socket_path();
        let server = spawn_broker_sequence(
            &socket,
            vec![BrokerPayload::BrowserFilled { filled_count: 2 }],
        );

        let result = fill(&socket, "browser:citi", "citi-login");
        let requests = server.join().unwrap();

        assert_eq!(result.unwrap(), 2);
        assert_eq!(requests.len(), 1);
        assert_fill_request(&requests[0], "browser:citi", "citi-login");
        std::fs::remove_file(socket).unwrap();
    }

    #[test]
    fn locked_fill_unlocks_exact_scope_and_retries_once() {
        let socket = broker_socket_path();
        let server = spawn_broker_sequence(
            &socket,
            vec![
                locked_response("remote locked message"),
                BrokerPayload::Unlocked {
                    scopes: vec!["browser:citi".to_string()],
                },
                BrokerPayload::BrowserFilled { filled_count: 2 },
            ],
        );

        let result = fill(&socket, "browser:citi", "citi-login");
        let requests = server.join().unwrap();

        assert_eq!(result.unwrap(), 2);
        assert_eq!(requests.len(), 3);
        assert_fill_request(&requests[0], "browser:citi", "citi-login");
        assert_unlock_request(&requests[1], "browser:citi");
        assert_fill_request(&requests[2], "browser:citi", "citi-login");
        std::fs::remove_file(socket).unwrap();
    }

    #[test]
    fn approval_denial_stops_before_retrying_fill() {
        let socket = broker_socket_path();
        let server = spawn_broker_sequence(
            &socket,
            vec![
                locked_response("remote locked message"),
                BrokerPayload::Error(BrokerErrorResponse {
                    code: BrokerErrorCode::ApprovalDenied,
                    message: "password=remote-secret".to_string(),
                }),
            ],
        );

        let error = fill(&socket, "browser:citi", "citi-login")
            .unwrap_err()
            .to_string();
        let requests = server.join().unwrap();

        assert!(error.contains("approval was denied"));
        assert!(!error.contains("remote-secret"));
        assert_eq!(requests.len(), 2);
        assert_fill_request(&requests[0], "browser:citi", "citi-login");
        assert_unlock_request(&requests[1], "browser:citi");
        std::fs::remove_file(socket).unwrap();
    }

    #[test]
    fn approval_unavailable_stops_before_retrying_fill() {
        let socket = broker_socket_path();
        let server = spawn_broker_sequence(
            &socket,
            vec![
                locked_response("remote locked message"),
                BrokerPayload::Error(BrokerErrorResponse {
                    code: BrokerErrorCode::Unavailable,
                    message: "password=remote-secret".to_string(),
                }),
            ],
        );

        let error = fill(&socket, "browser:citi", "citi-login")
            .unwrap_err()
            .to_string();
        let requests = server.join().unwrap();

        assert!(error.contains("operation was unavailable"));
        assert!(!error.contains("remote-secret"));
        assert_eq!(requests.len(), 2);
        assert_fill_request(&requests[0], "browser:citi", "citi-login");
        assert_unlock_request(&requests[1], "browser:citi");
        std::fs::remove_file(socket).unwrap();
    }

    #[test]
    fn second_locked_response_stops_after_one_retry() {
        let socket = broker_socket_path();
        let server = spawn_broker_sequence(
            &socket,
            vec![
                locked_response("first remote locked message"),
                BrokerPayload::Unlocked {
                    scopes: vec!["browser:citi".to_string()],
                },
                locked_response("second remote locked message"),
            ],
        );

        let error = fill(&socket, "browser:citi", "citi-login")
            .unwrap_err()
            .to_string();
        let requests = server.join().unwrap();

        assert!(error.contains("remained locked after approval"));
        assert!(!error.contains("remote locked message"));
        assert_eq!(requests.len(), 3);
        assert_fill_request(&requests[0], "browser:citi", "citi-login");
        assert_unlock_request(&requests[1], "browser:citi");
        assert_fill_request(&requests[2], "browser:citi", "citi-login");
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
            read_mapped_scope_for_url(&path, "https://online.citi.com/login", Some("browser:citi"))
                .unwrap(),
            "browser:citi"
        );

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_suffix_insecure_mismatch_and_permissive_config() {
        let path = write_scope_config(r#"{"online.citi.com":"browser:citi"}"#, 0o600);

        assert!(
            read_mapped_scope_for_url(&path, "https://evilonline.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("not mapped")
        );
        assert!(
            read_mapped_scope_for_url(&path, "http://online.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("HTTPS")
        );
        assert!(
            read_mapped_scope_for_url(
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
            read_mapped_scope_for_url(&path, "https://online.citi.com/login", None)
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
            read_mapped_scope_for_url(&missing, "https://online.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("metadata")
        );

        let malformed = write_scope_config("not json", 0o600);
        assert!(
            read_mapped_scope_for_url(&malformed, "https://online.citi.com/login", None)
                .unwrap_err()
                .to_string()
                .contains("parse")
        );
        std::fs::remove_file(&malformed).unwrap();

        let target = write_scope_config(r#"{"online.citi.com":"browser:citi"}"#, 0o600);
        let symlink = target.with_extension("symlink.json");
        std::os::unix::fs::symlink(&target, &symlink).unwrap();
        assert!(
            read_mapped_scope_for_url(&symlink, "https://online.citi.com/login", None)
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
