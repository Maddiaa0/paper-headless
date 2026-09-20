//! One request against Paper's MCP endpoint: `initialize`. Paper answers it
//! when a renderer is available, including an authentication error page.
//! This checks the transport, not sign-in. Agents talk to the endpoint
//! directly; this crate never proxies or re-implements MCP tools.

use crate::{Error, Result};
use serde_json::{Value, json};
use std::time::Duration;

pub(crate) enum Probe {
    /// Paper accepted `initialize`; the server reported this name/version.
    Ready { server: String },
    /// The port answered but rejected the handshake (typically: signed out).
    Rejected { status: u16, detail: String },
}

pub(crate) fn probe(mcp_url: &str) -> Result<Probe> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build(),
    );
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "paper-headless", "version": env!("CARGO_PKG_VERSION") }
        }
    });
    let mut response = agent
        .post(mcp_url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .send(request.to_string().as_bytes())?;
    let status = response.status().as_u16();
    let session = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response.body_mut().read_to_string()?;
    let message = parse_message(&body);
    let server = message
        .as_ref()
        .and_then(|m| m.pointer("/result/serverInfo"))
        .map(|info| {
            format!(
                "{} {}",
                info.get("name").and_then(Value::as_str).unwrap_or("?"),
                info.get("version").and_then(Value::as_str).unwrap_or("")
            )
        });
    match (session, server) {
        (Some(_), Some(server)) if status < 400 => Ok(Probe::Ready { server }),
        _ => Ok(Probe::Rejected {
            status,
            detail: message
                .and_then(|m| {
                    m.pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| body.chars().take(300).collect()),
        }),
    }
}

/// The endpoint answers either with plain JSON or as an SSE stream of
/// `data:` lines; either way we want the single JSON-RPC message inside.
fn parse_message(body: &str) -> Option<Value> {
    let trimmed = body.trim();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).ok();
    }
    trimmed
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .find(|value| value.get("result").is_some() || value.get("error").is_some())
}

pub(crate) fn describe(probe: &Probe) -> String {
    match probe {
        Probe::Ready { server } => format!("MCP ready ({server})"),
        Probe::Rejected { status, detail } => {
            format!("MCP endpoint answered HTTP {status} but refused initialize: {detail}")
        }
    }
}

impl Probe {
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self, Probe::Ready { .. })
    }
}

pub(crate) fn require_ready(mcp_url: &str) -> Result<String> {
    match probe(mcp_url)? {
        Probe::Ready { server } => Ok(server),
        rejected => Err(Error::msg(describe(&rejected))),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::http;
    const READY: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"serverInfo":{"name":"paper-desktop","version":"test"}}}"#;
    mod when_parsing_a_response {
        use super::*;
        #[test]
        fn accepts_plain_json() {
            assert_eq!(
                parse_message(READY).unwrap()["result"]["serverInfo"]["name"],
                "paper-desktop"
            );
        }
        #[test]
        fn accepts_sse_after_unrelated_events() {
            let body = format!("event: message\ndata: {{}}\ndata: invalid\ndata: {READY}\n\n");
            assert!(parse_message(&body).unwrap().get("result").is_some());
        }
        #[test]
        fn ignores_malformed_data() {
            for body in ["", "{broken", "data: not json", "event: ping"] {
                assert!(parse_message(body).is_none());
            }
        }
    }
    mod when_probing_an_endpoint {
        use super::*;
        #[test]
        fn sends_the_initialize_contract() {
            let (url, worker) = http(200, "mcp-session-id: test\r\n", READY);
            assert_eq!(require_ready(&url).unwrap(), "paper-desktop test");
            let request = worker.join().unwrap();
            assert!(request.starts_with("POST / HTTP/1.1"));
            let body: Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(body["method"], "initialize");
            assert_eq!(body["params"]["protocolVersion"], "2025-03-26");
            assert_eq!(body["params"]["clientInfo"]["name"], "paper-headless");
        }
        #[test]
        fn accepts_an_sse_handshake() {
            let (url, worker) = http(
                200,
                "mcp-session-id: test\r\nContent-Type: text/event-stream\r\n",
                &format!("data: {READY}\n\n"),
            );
            assert!(probe(&url).unwrap().is_ready());
            worker.join().unwrap();
        }
        #[test]
        fn rejects_a_missing_session() {
            let (url, worker) = http(200, "", READY);
            assert!(!probe(&url).unwrap().is_ready());
            worker.join().unwrap();
        }
        #[test]
        fn reports_a_protocol_error() {
            let (url, worker) = http(200, "", r#"{"error":{"message":"Sign in required"}}"#);
            let err = require_ready(&url).unwrap_err();
            assert!(err.to_string().contains("Sign in required"));
            worker.join().unwrap();
        }
        #[test]
        fn reports_an_http_error() {
            let (url, worker) = http(500, "", "renderer unavailable");
            let result = probe(&url).unwrap();
            assert!(describe(&result).contains("HTTP 500"));
            assert!(describe(&result).contains("renderer unavailable"));
            worker.join().unwrap();
        }
        #[test]
        fn propagates_transport_errors() {
            assert!(probe("not a url").is_err());
        }
    }
}
