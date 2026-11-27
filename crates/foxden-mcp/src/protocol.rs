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
    /// `client` is the cleaned client name; `waiting` is called now and then while the app takes long.
    fn call(&mut self, tool: &str, args: &Value, client: &str, waiting: &mut dyn FnMut()) -> Result<String, String>;
}

pub struct Server<R: Relay> {
    relay: R,
    client: String,
}

/// The client's name as the app may show it: printable, one line, at most 40 characters.
pub fn clean_name(name: &str) -> String {
    let printable: String = name.chars().filter(|c| !c.is_control() && !invisible(*c)).collect();
    let one_line = printable.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(40).collect::<String>().trim_end().to_string()
}

/// Format characters that can hide or reorder text.
fn invisible(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

impl<R: Relay> Server<R> {
    pub fn new(relay: R) -> Self {
        Server { relay, client: String::new() }
    }

    /// The reply to one line, or `None` for a notification or a response. `notify` sends progress.
    pub fn handle(&mut self, line: &str, notify: &mut dyn FnMut(Value)) -> Option<Value> {
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
        Some(match self.dispatch(method, &params, notify) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error(id, code, &message),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Value, notify: &mut dyn FnMut(Value)) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                self.client = clean_name(params["clientInfo"]["name"].as_str().unwrap_or_default());
                Ok(initialize(params))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools::list()})),
            "tools/call" => self.call(params, notify),
            _ => Err((METHOD_NOT_FOUND, format!("Method not found: {method}"))),
        }
    }

    fn call(&mut self, params: &Value, notify: &mut dyn FnMut(Value)) -> Result<Value, (i64, String)> {
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
        let token = params["_meta"]["progressToken"].clone();
        let mut ticks = 0;
        let mut waiting = || {
            if !token.is_null() {
                ticks += 1;
                notify(json!({"jsonrpc": "2.0", "method": "notifications/progress",
                    "params": {"progressToken": token, "progress": ticks, "message": "Waiting for the person at the PC"}}));
            }
        };
        let (text, failed) = match self.relay.call(name, &args, &self.client, &mut waiting) {
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
        "instructions": "Controls the FoxDen app on this PC: lighting effects, layers (including keys lit while a game is in front), devices, key maps, profiles, everyday settings, the audio mixer and key and mouse macros, and reads sensors and batteries. Fan curves can be proposed; the person at the PC approves them on screen, as they do any macro that would send input.",
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
        fn call(&mut self, tool: &str, args: &Value, client: &str, waiting: &mut dyn FnMut()) -> Result<String, String> {
            match tool {
                "pause" => Err("nope".into()),
                "propose_fan_curves" => {
                    waiting();
                    waiting();
                    Ok(format!("from {client}"))
                }
                _ => Ok(format!("{tool} {args}")),
            }
        }
    }

    fn ask(server: &mut Server<impl Relay>, msg: Value) -> Value {
        server.handle(&msg.to_string(), &mut |_| {}).expect("a reply")
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
        assert_eq!(s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &mut |_| {}), None);
        assert_eq!(s.handle(r#"{"jsonrpc":"2.0","id":5,"result":{}}"#, &mut |_| {}), None);
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
        assert!(names.contains(&"get_fan_config") && names.contains(&"propose_fan_curves"));
        assert!(!names.iter().any(|n| n.contains("set_fan") || n.contains("shutdown")));

        let get = tools.iter().find(|t| t["name"] == "get_fan_config").unwrap();
        assert_eq!(get["annotations"]["readOnlyHint"], true);
        let propose = tools.iter().find(|t| t["name"] == "propose_fan_curves").unwrap();
        assert_eq!(propose["annotations"]["readOnlyHint"], false);
        let schema = &propose["inputSchema"];
        assert_eq!(schema["required"], json!(["curves", "rules", "reason"]));
        let curve = &schema["properties"]["curves"]["items"];
        assert_eq!(curve["properties"]["points"]["items"]["minItems"], 2);
        assert_eq!(curve["required"], json!(["name", "sensors", "points"]));
        let rule = &schema["properties"]["rules"]["items"];
        assert_eq!(rule["properties"]["max_duty"]["minimum"], 20);
        assert_eq!(rule["required"], json!(["fan", "curve"]));
        let text = propose["description"].as_str().unwrap();
        assert!(text.contains("approve") && text.contains("20%") && text.contains("pump"), "{text}");
    }

    #[test]
    fn the_client_name_reaches_the_app_cleaned() {
        let mut s = Server::new(Echo);
        let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "propose_fan_curves"}});
        assert_eq!(ask(&mut s, call.clone())["result"]["content"][0]["text"], "from ");
        let name = format!("Claude\u{202e}\n Desktop\u{200b}\x07 {}", "x".repeat(80));
        ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": name}}}));
        let text = ask(&mut s, call)["result"]["content"][0]["text"].as_str().unwrap().to_string();
        let shown = text.strip_prefix("from ").unwrap();
        assert!(shown.starts_with("Claude Desktop xxx"), "{shown}");
        assert_eq!(shown.chars().count(), 40);
        assert_eq!(clean_name("  \u{7}  "), "");
        assert_eq!(clean_name("Zed ✓"), "Zed ✓");
    }

    #[test]
    fn waiting_sends_progress_only_when_asked_for() {
        let mut s = Server::new(Echo);
        let mut sent = Vec::new();
        let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "propose_fan_curves", "_meta": {"progressToken": "t1"}}});
        s.handle(&call.to_string(), &mut |n| sent.push(n));
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1]["method"], "notifications/progress");
        assert_eq!(sent[1]["params"]["progressToken"], "t1");
        assert_eq!(sent[1]["params"]["progress"], 2);
        sent.clear();
        let call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "propose_fan_curves"}});
        s.handle(&call.to_string(), &mut |n| sent.push(n));
        assert!(sent.is_empty());
    }

    #[test]
    fn errors_follow_json_rpc() {
        let mut s = Server::new(Echo);
        assert_eq!(s.handle("{oops", &mut |_| {}).unwrap()["error"]["code"], PARSE_ERROR);
        assert_eq!(s.handle("[1]", &mut |_| {}).unwrap()["error"]["code"], INVALID_REQUEST);
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
