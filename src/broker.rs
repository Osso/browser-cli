use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

#[derive(Serialize)]
struct Request<'a> {
    version: u16,
    request_id: &'a str,
    operation: Operation<'a>,
}
#[derive(Serialize)]
#[serde(tag = "type", content = "parameters", rename_all = "snake_case")]
enum Operation<'a> {
    BrowserFill(BrowserFill<'a>),
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
    BrowserFilled { filled_count: usize },
    Error(ErrorResponse),
}
#[derive(Deserialize)]
struct ErrorResponse {
    message: String,
}

pub fn fill(socket: &Path, scope: &str, target_id: &str) -> Result<usize> {
    let request = Request {
        version: 1,
        request_id: "browser-cli",
        operation: Operation::BrowserFill(BrowserFill { scope, target_id }),
    };
    let bytes = rmp_serde::to_vec_named(&request)?;
    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("connect broker socket {}", socket.display()))?;
    stream.write_all(&bytes)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    match rmp_serde::from_slice::<Response>(&response)?.payload {
        Payload::BrowserFilled { filled_count } => Ok(filled_count),
        Payload::Error(error) => bail!("broker denied fill: {}", error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::os::unix::net::UnixListener;
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
}
