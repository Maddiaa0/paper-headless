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
    pub(crate) fn click_first_button(&mut self) -> Result<(f64, f64)> {
        let rect = self.evaluate(
            "(() => { const b = document.querySelector('button'); if (!b) return null; \
             const r = b.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; })()",
        )?;
        if rect.is_null() {
            return Err(Error::msg("no <button> found on the sign-in page"));
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
        Ok((x, y))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::http;
    fn session(replies: Vec<Value>, noise: bool) -> (Session, std::thread::JoinHandle<Vec<Value>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let mut requests = Vec::new();
            for mut reply in replies {
                let request: Value =
                    serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
                if noise {
                    ws.send(Message::text(json!({"method":"Page.event"}).to_string()))
                        .unwrap();
                    ws.send(Message::text(json!({"id":999,"result":{}}).to_string()))
                        .unwrap();
                }
                reply["id"] = request["id"].clone();
                ws.send(Message::text(reply.to_string())).unwrap();
                requests.push(request);
            }
            requests
        });
        (Session::connect(&url).unwrap(), worker)
    }

    mod when_listing_targets {
        use super::*;
        #[test]
        fn keeps_only_page_targets() {
            let (url, w) = http(
                200,
                "",
                r#"[{"type":"page","url":"https://app.paper.design/","webSocketDebuggerUrl":"ws://example"},{"type":"worker","url":"worker"}]"#,
            );
            let targets = pages(&url).unwrap();
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].ws_url.as_deref(), Some("ws://example"));
            assert!(w.join().unwrap().starts_with("GET /json "));
        }
        #[test]
        fn rejects_malformed_target_data() {
            let (url, w) = http(200, "", "not json");
            assert!(pages(&url).is_err());
            w.join().unwrap();
        }
    }
    mod when_evaluating_a_script {
        use super::*;
        #[test]
        fn ignores_events_and_unrelated_replies() {
            let (mut s, w) = session(vec![json!({"result":{"result":{"value":42}}})], true);
            assert_eq!(s.evaluate("6 * 7").unwrap(), json!(42));
            let reqs = w.join().unwrap();
            assert_eq!(reqs[0]["method"], "Runtime.evaluate");
            assert_eq!(reqs[0]["params"]["expression"], "6 * 7");
        }
        #[test]
        fn reports_protocol_errors() {
            let (mut s, w) = session(vec![json!({"error":{"message":"denied"}})], false);
            assert!(s.evaluate("1").unwrap_err().to_string().contains("denied"));
            w.join().unwrap();
        }
        #[test]
        fn reports_javascript_exceptions() {
            let (mut s, w) = session(
                vec![json!({"result":{"exceptionDetails":{"text":"ReferenceError"}}})],
                false,
            );
            assert!(
                s.evaluate("missing")
                    .unwrap_err()
                    .to_string()
                    .contains("ReferenceError")
            );
            w.join().unwrap();
        }
    }
    mod when_clicking_sign_in {
        use super::*;
        #[test]
        fn sends_trusted_events_at_the_button_center() {
            let (mut s, w) = session(
                vec![
                    json!({"result":{"result":{"value":{"x":10,"y":20,"w":80,"h":40}}}}),
                    json!({"result":{}}),
                    json!({"result":{}}),
                    json!({"result":{}}),
                ],
                false,
            );
            assert_eq!(s.click_first_button().unwrap(), (50.0, 40.0));
            let reqs = w.join().unwrap();
            for (req, kind) in reqs[1..]
                .iter()
                .zip(["mouseMoved", "mousePressed", "mouseReleased"])
            {
                assert_eq!(req["method"], "Input.dispatchMouseEvent");
                assert_eq!(req["params"]["type"], kind);
                assert_eq!(req["params"]["x"], 50.0);
                assert_eq!(req["params"]["y"], 40.0);
            }
            assert_eq!(reqs[2]["params"]["button"], "left");
        }
        #[test]
        fn rejects_a_missing_button() {
            let (mut s, w) = session(vec![json!({"result":{"result":{"value":null}}})], false);
            assert!(
                s.click_first_button()
                    .unwrap_err()
                    .to_string()
                    .contains("no <button>")
            );
            assert_eq!(w.join().unwrap().len(), 1);
        }
    }
}
