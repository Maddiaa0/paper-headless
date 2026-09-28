//! The smallest Chrome DevTools Protocol client that can find Paper's sign-in
//! page and click its button with a trusted input event. Nothing else about
//! the app is driven through DevTools.

use crate::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::TcpStream;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

#[derive(Debug, Deserialize)]
pub(crate) struct Target {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub(crate) ws_url: Option<String>,
}

/// Pages currently open in the app, or an error if DevTools is unreachable.
pub(crate) fn pages(cdp_url: &str) -> Result<Vec<Target>> {
    let agent = agent();
    let mut response = agent.get(format!("{cdp_url}/json")).call()?;
    let body = response.body_mut().read_to_string()?;
    let targets: Vec<Target> = serde_json::from_str(&body)?;
    Ok(targets.into_iter().filter(|t| t.kind == "page").collect())
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .build(),
    )
}

pub(crate) struct Session {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl Session {
    pub(crate) fn connect(ws_url: &str) -> Result<Self> {
        let (socket, _response) = tungstenite::connect(ws_url)?;
        if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        }
        Ok(Self { socket, next_id: 0 })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({ "id": id, "method": method, "params": params });
        self.socket.send(Message::text(request.to_string()))?;
        loop {
            let message = self.socket.read()?;
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(text.as_str())?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // an event, or a reply to something else
            }
            if let Some(error) = value.get("error") {
                return Err(Error::msg(format!("DevTools {method} failed: {error}")));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Evaluate a JavaScript expression in the page and return its value.
    pub(crate) fn evaluate(&mut self, expression: &str) -> Result<Value> {
        let result = self.call(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(Error::msg(format!("page script threw: {exception}")));
        }
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Click the centre of the first `<button>` on the page with trusted
    /// mouse events, which is what the sign-in page's handler requires.
    /// Returns false if the page has no button yet.
    pub(crate) fn click_first_button(&mut self) -> Result<bool> {
        let rect = self.evaluate(
            "(() => { const b = document.querySelector('button'); if (!b) return null; \
             const r = b.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; })()",
        )?;
        if rect.is_null() {
            return Ok(false);
        }
        let field = |name: &str| rect.get(name).and_then(Value::as_f64).unwrap_or(0.0);
        let x = (field("x") + field("w") / 2.0).round();
        let y = (field("y") + field("h") / 2.0).round();
        self.call(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": x, "y": y }),
        )?;
        self.call(
            "Input.dispatchMouseEvent",
            json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        )?;
        self.call(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        )?;
        Ok(true)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
    }
}
