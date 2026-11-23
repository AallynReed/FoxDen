//! JSON-RPC 2.0 messages of the Model Context Protocol.

use serde_json::{Map, Value, json};

use crate::tools;

pub const LATEST: &str = "2025-06-18";
const SUPPORTED: [&str; 3] = [LATEST, "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Carries one tool call to the app: the reply text, or the reason it failed.
pub trait Relay {
    fn call(&mut self, tool: &str, args: &Value) -> Result<String, String>;
}

pub struct Server<R: Relay> {
    relay: R,
}

impl<R: Relay> Server<R> {
    pub fn new(relay: R) -> Self {
        Server { relay }
    }

    /// The reply to one line, or `None` for a notification or a response.
    pub fn handle(&mut self, line: &str) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(msg) => msg,
            Err(_) => return Some(error(Value::Null, PARSE_ERROR, "Parse error")),
        };
        let Some(obj) = msg.as_object() else {
            return Some(error(Value::Null, INVALID_REQUEST, "Expected a single JSON-RPC object"));
        };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            // A response to something we never sent, or nonsense.
            if obj.contains_key("result") || obj.contains_key("error") {
                return None;
            }
            return Some(error(id.unwrap_or(Value::Null), INVALID_REQUEST, "Missing method"));
        };
        let id = id?;
        let params = obj.get("params").cloned().unwrap_or(Value::Object(Map::new()));
        Some(match self.dispatch(method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error(id, code, &message),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools::list()})),
            "tools/call" => self.call(params),
            _ => Err((METHOD_NOT_FOUND, format!("Method not found: {method}"))),
        }
    }

    fn call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let bad = |m: &str| (INVALID_PARAMS, m.to_string());
        let name = params["name"].as_str().ok_or_else(|| bad("Missing tool name"))?;
        if !tools::known(name) {
            return Err(bad(&format!("Unknown tool: {name}")));
        }
        let args = match params.get("arguments") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(args @ Value::Object(_)) => args.clone(),
            Some(_) => return Err(bad("Tool arguments must be an object")),
        };
        let (text, failed) = match self.relay.call(name, &args) {
            Ok(text) => (text, false),
            Err(text) => (text, true),
        };
        Ok(json!({"content": [{"type": "text", "text": text}], "isError": failed}))
    }
}

fn initialize(params: &Value) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or(LATEST);
    let version = SUPPORTED.iter().find(|v| **v == asked).copied().unwrap_or(LATEST);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "foxden", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Controls the FoxDen app on this PC: lighting, profiles and the audio mixer, and reads sensors and batteries.",
    })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipe::AppPipe;

    struct Echo;

    impl Relay for Echo {
        fn call(&mut self, tool: &str, args: &Value) -> Result<String, String> {
            if tool == "pause" {
                return Err("nope".into());
            }
            Ok(format!("{tool} {args}"))
        }
    }

    fn ask(server: &mut Server<impl Relay>, msg: Value) -> Value {
        server.handle(&msg.to_string()).expect("a reply")
    }

    #[test]
    fn initialize_names_the_server_and_its_tools_capability() {
        let mut s = Server::new(Echo);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}));
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "foxden");
        assert_eq!(r["result"]["capabilities"], json!({"tools": {}}));

        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}));
        assert_eq!(r["result"]["protocolVersion"], LATEST);
    }

    #[test]
    fn notifications_and_responses_get_no_reply() {
        let mut s = Server::new(Echo);
        assert_eq!(s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), None);
        assert_eq!(s.handle(r#"{"jsonrpc":"2.0","id":5,"result":{}}"#), None);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": "p", "method": "ping"}));
        assert_eq!(r, json!({"jsonrpc": "2.0", "id": "p", "result": {}}));
    }

    #[test]
    fn tools_list_gives_every_tool_a_schema() {
        let mut s = Server::new(Echo);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
        let tools = r["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 15);
        for t in tools {
            assert!(t["name"].is_string() && t["description"].is_string(), "{t}");
            assert_eq!(t["inputSchema"]["type"], "object", "{t}");
            assert!(t["inputSchema"]["properties"].is_object(), "{t}");
        }
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for n in ["get_status", "set_effect", "set_brightness", "get_sensors", "assign_app"] {
            assert!(names.contains(&n), "{n}");
        }
        assert!(!names.iter().any(|n| n.contains("fan_curve") || n.contains("shutdown")));
    }

    #[test]
    fn errors_follow_json_rpc() {
        let mut s = Server::new(Echo);
        assert_eq!(s.handle("{oops").unwrap()["error"]["code"], PARSE_ERROR);
        assert_eq!(s.handle("[1]").unwrap()["error"]["code"], INVALID_REQUEST);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}));
        assert_eq!(r["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(r["id"], 3);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "set_fan_curve"}}));
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "pause", "arguments": 3}}));
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn tool_calls_carry_text_and_the_error_flag() {
        let mut s = Server::new(Echo);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "set_brightness", "arguments": {"value": 40}}}));
        assert_eq!(r["result"], json!({"content": [{"type": "text", "text": "set_brightness {\"value\":40}"}], "isError": false}));
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "pause"}}));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["text"], "nope");
    }

    #[test]
    fn a_call_without_the_app_says_why() {
        let gone = AppPipe::with(r"\\.\pipe\foxden-mcp-test-nobody-here".into(), || false);
        let mut s = Server::new(gone);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "get_status"}}));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["content"][0]["text"], crate::pipe::NOT_RUNNING);

        let off = AppPipe::with(r"\\.\pipe\foxden-mcp-test-nobody-here".into(), || true);
        let mut s = Server::new(off);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "get_status"}}));
        assert_eq!(r["result"]["content"][0]["text"], crate::pipe::TURNED_OFF);
    }
}
