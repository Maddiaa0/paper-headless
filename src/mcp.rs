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
