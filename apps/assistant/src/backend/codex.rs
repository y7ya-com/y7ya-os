//! Codex through `codex app-server`: JSON-RPC over stdio, as T3 Code uses it.
//! Our MCP server replaces every other one (`-c mcp_servers=...`), the shell,
//! patching, web search and the other built-in tools are switched off, the
//! sandbox is read-only, and any approval request is declined.

use std::collections::HashSet;
use std::io::Write;
use std::process::Child;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{Event, Kind, Proc, SYSTEM_PROMPT, Sink, died, mcp_command, tool_result, workdir};
use crate::mcp::SERVER;

/// Built-in features that could act outside our tools.
const DISABLED_FEATURES: [&str; 16] = [
    "shell_tool",
    "unified_exec",
    "apps",
    "plugins",
    "multi_agent",
    "image_generation",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "in_app_browser",
    "view_image",
    "hooks",
    "tool_suggest",
    "skill_search",
    "goals",
    "memories",
];

pub fn args() -> Vec<String> {
    let (exe, mcp_args) = mcp_command();
    // A JSON string is a valid TOML basic string.
    let toml_str = |s: &str| Value::from(s).to_string();
    let mcp_args: Vec<String> = mcp_args.iter().map(|a| toml_str(a)).collect();
    let servers = format!("mcp_servers={{ {SERVER} = {{ command = {}, args = [{}], startup_timeout_sec = 20, tool_timeout_sec = 90 }} }}", toml_str(&exe), mcp_args.join(", "));
    let mut a: Vec<String> = vec!["app-server".into(), "--listen".into(), "stdio://".into(), "-c".into(), servers];
    for f in DISABLED_FEATURES {
        a.extend(["-c".into(), format!("features.{f}=false")]);
    }
    a.extend(["-c".into(), "web_search=\"disabled\"".into()]);
    a
}

const INIT: i64 = 1;
const THREAD: i64 = 2;
const NEW_THREAD: i64 = 3;
const TURN: i64 = 4;

/// The client side of one turn, as a pure state machine over JSON-RPC messages.
pub struct Client {
    resume: Option<String>,
    prompt: String,
    thread: Option<String>,
    streamed: HashSet<String>,
    pub done: bool,
}

fn request(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn thread_params(thread: Option<&str>) -> Value {
    let mut p = json!({
        "cwd": workdir().to_string_lossy(),
        "sandbox": "read-only",
        "approvalPolicy": "never",
        "baseInstructions": SYSTEM_PROMPT,
    });
    match thread {
        Some(id) => p["threadId"] = json!(id),
        None => p["serviceName"] = json!("y7ya-assistant"),
    }
    p
}

impl Client {
    pub fn new(resume: Option<String>, prompt: String) -> Client {
        Client { resume, prompt, thread: None, streamed: HashSet::new(), done: false }
    }

    pub fn start(&self) -> Vec<Value> {
        vec![request(INIT, "initialize", json!({ "clientInfo": { "name": "y7ya-assistant", "title": "Assistant", "version": env!("CARGO_PKG_VERSION") } }))]
    }

    /// Handles one message from the server: events for the UI and messages to send back.
    pub fn on_message(&mut self, msg: &Value) -> (Vec<Event>, Vec<Value>) {
        let mut events = Vec::new();
        let mut send = Vec::new();
        let method = msg["method"].as_str();
        match (msg.get("id").filter(|i| !i.is_null()), method) {
            // A response to one of our requests.
            (Some(id), None) => {
                let id = id.as_i64().unwrap_or(-1);
                let error = msg.get("error").filter(|e| !e.is_null()).map(|e| e["message"].as_str().unwrap_or("error").to_string());
                match (id, error) {
                    (INIT, None) => {
                        send.push(json!({ "jsonrpc": "2.0", "method": "initialized" }));
                        send.push(match &self.resume {
                            Some(t) => request(THREAD, "thread/resume", thread_params(Some(t))),
                            None => request(NEW_THREAD, "thread/start", thread_params(None)),
                        });
                    }
                    // The old thread is gone: start a fresh one.
                    (THREAD, Some(_)) => send.push(request(NEW_THREAD, "thread/start", thread_params(None))),
                    (THREAD | NEW_THREAD, None) => {
                        let id = msg["result"]["thread"]["id"].as_str().unwrap_or_default().to_string();
                        events.push(Event::Session(id.clone()));
                        send.push(request(
                            TURN,
                            "turn/start",
                            json!({ "threadId": id, "input": [{ "type": "text", "text": self.prompt, "text_elements": [] }] }),
                        ));
                        self.thread = Some(id);
                    }
                    (TURN, None) => {}
                    (_, Some(e)) => {
                        events.push(Event::Error(friendly(&e)));
                        self.done = true;
                    }
                    _ => {}
                }
            }
            // A request from the server: approvals and questions. Nothing outside our tools is allowed.
            (Some(id), Some(m)) => {
                let result = match m {
                    "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({ "decision": "decline" }),
                    "execCommandApproval" | "applyPatchApproval" => json!({ "decision": "denied" }),
                    "item/permissions/requestApproval" => json!({ "permissions": {}, "scope": "turn" }),
                    "item/tool/requestUserInput" => json!({ "answers": {} }),
                    // Our own server's tools may run: they ask the user themselves when it matters.
                    "mcpServer/elicitation/request" if msg["params"]["serverName"] == SERVER => json!({ "action": "accept", "content": {} }),
                    "mcpServer/elicitation/request" => json!({ "action": "decline" }),
                    _ => {
                        send.push(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Not supported" } }));
                        return (events, send);
                    }
                };
                send.push(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            (None, Some(m)) => self.notification(m, &msg["params"], &mut events),
            _ => {}
        }
        (events, send)
    }

    fn notification(&mut self, method: &str, p: &Value, events: &mut Vec<Event>) {
        if p["threadId"].as_str().is_some_and(|t| Some(t) != self.thread.as_deref()) {
            return;
        }
        let item = &p["item"];
        match method {
            "item/agentMessage/delta" => {
                if let Some(d) = p["delta"].as_str() {
                    self.streamed.insert(p["itemId"].as_str().unwrap_or_default().to_string());
                    events.push(Event::Text(d.into()));
                }
            }
            "item/started" if item["type"] == "mcpToolCall" => events.push(Event::ToolStart {
                id: item["id"].as_str().unwrap_or_default().into(),
                name: item["tool"].as_str().unwrap_or_default().into(),
                args: item["arguments"].clone(),
            }),
            "item/completed" if item["type"] == "mcpToolCall" => {
                let failed = item["status"] == "failed";
                let text = item["result"]["content"]
                    .as_array()
                    .map(|c| c.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"))
                    .filter(|t| !t.is_empty())
                    .or_else(|| item["error"]["message"].as_str().map(str::to_string))
                    .unwrap_or_default();
                events.push(Event::ToolEnd { id: item["id"].as_str().unwrap_or_default().into(), result: tool_result(&text, failed) });
            }
            "item/completed" if item["type"] == "agentMessage" => {
                let id = item["id"].as_str().unwrap_or_default();
                if !self.streamed.contains(id) {
                    if let Some(t) = item["text"].as_str().filter(|t| !t.is_empty()) {
                        events.push(Event::Text(t.into()));
                    }
                }
            }
            "mcpServer/startupStatus/updated" if p["name"] == SERVER || p["serverName"] == SERVER => {
                let status = p["status"].as_str().unwrap_or_default();
                if status.contains("fail") || status.contains("error") {
                    events.push(Event::Error("The settings tools didn't start.".into()));
                }
            }
            "error" if p["willRetry"].as_bool() != Some(true) => {
                events.push(Event::Error(friendly(p["error"]["message"].as_str().unwrap_or("Codex hit an error."))));
            }
            "turn/completed" => {
                let turn = &p["turn"];
                if turn["status"] == "failed" {
                    events.push(Event::Error(friendly(turn["error"]["message"].as_str().unwrap_or("Codex couldn't finish."))));
                }
                self.done = true;
            }
            _ => {}
        }
    }
}

fn friendly(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("login") || lower.contains("not logged in") || lower.contains("unauthorized") || lower.contains("401") {
        return format!("Codex isn't signed in. Run \u{201c}{}\u{201d} in a terminal, then try again.", Kind::Codex.login_command());
    }
    msg.to_string()
}

pub fn run(session: Option<String>, prompt: &str, sink: &Sink, slot: &Arc<Mutex<Option<Child>>>) -> Result<(), String> {
    let mut p = Proc::spawn(Kind::Codex, &args(), slot)?;
    let mut client = Client::new(session, prompt.to_string());
    let write = |p: &mut Proc, msgs: Vec<Value>| {
        if let Some(stdin) = p.stdin.as_mut() {
            for m in msgs {
                let _ = writeln!(stdin, "{m}");
            }
            let _ = stdin.flush();
        }
    };
    let first = client.start();
    write(&mut p, first);
    while let Some(line) = p.next_line() {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let (events, send) = client.on_message(&msg);
        for e in events {
            sink(e);
        }
        write(&mut p, send);
        if client.done {
            break;
        }
    }
    let err = (!client.done).then(|| died(Kind::Codex, &p));
    p.finish();
    err.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::CallResult;

    fn feed(c: &mut Client, msg: Value) -> (Vec<Event>, Vec<Value>) {
        c.on_message(&msg)
    }

    #[test]
    fn handshake_resume_and_turn() {
        let mut c = Client::new(Some("old".into()), "Set volume to 30%".into());
        assert_eq!(c.start()[0]["method"], "initialize");
        let (_, send) = feed(&mut c, json!({ "id": 1, "result": { "userAgent": "codex" } }));
        assert_eq!(send[0]["method"], "initialized");
        assert_eq!(send[1]["method"], "thread/resume");
        assert_eq!(send[1]["params"]["threadId"], "old");
        assert_eq!(send[1]["params"]["sandbox"], "read-only");
        // The thread can't be resumed: start a new one.
        let (_, send) = feed(&mut c, json!({ "id": 2, "error": { "code": -32600, "message": "thread not found" } }));
        assert_eq!(send[0]["method"], "thread/start");
        let (events, send) = feed(&mut c, json!({ "id": 3, "result": { "thread": { "id": "t1" }, "model": "gpt" } }));
        assert_eq!(events, [Event::Session("t1".into())]);
        assert_eq!(send[0]["method"], "turn/start");
        assert_eq!(send[0]["params"]["input"][0]["text"], "Set volume to 30%");

        let (e, _) = feed(&mut c, json!({ "method": "item/started", "params": { "threadId": "t1", "turnId": "u", "item": { "type": "mcpToolCall", "id": "call_1", "server": "y7ya", "tool": "set_volume", "arguments": { "percent": 30 }, "status": "inProgress" } } }));
        assert_eq!(e, [Event::ToolStart { id: "call_1".into(), name: "set_volume".into(), args: json!({ "percent": 30 }) }]);
        let result = r#"{"ok":true,"summary":"Set volume to 30%","state":{"volume_percent":30}}"#;
        let (e, _) = feed(&mut c, json!({ "method": "item/completed", "params": { "threadId": "t1", "turnId": "u", "item": { "type": "mcpToolCall", "id": "call_1", "server": "y7ya", "tool": "set_volume", "status": "completed", "result": { "content": [{ "type": "text", "text": result }] } } } }));
        assert!(matches!(&e[0], Event::ToolEnd { result: CallResult::Done(o), .. } if o.summary == "Set volume to 30%"));
        let (e, _) = feed(&mut c, json!({ "method": "item/agentMessage/delta", "params": { "threadId": "t1", "turnId": "u", "itemId": "m1", "delta": "Volume is at 30%." } }));
        assert_eq!(e, [Event::Text("Volume is at 30%.".into())]);
        // The completed message isn't repeated after streaming.
        let (e, _) = feed(&mut c, json!({ "method": "item/completed", "params": { "threadId": "t1", "item": { "type": "agentMessage", "id": "m1", "text": "Volume is at 30%." } } }));
        assert!(e.is_empty());
        // Other threads' traffic is ignored.
        let (e, _) = feed(&mut c, json!({ "method": "item/agentMessage/delta", "params": { "threadId": "other", "itemId": "x", "delta": "hi" } }));
        assert!(e.is_empty());
        assert!(!c.done);
        feed(&mut c, json!({ "method": "turn/completed", "params": { "threadId": "t1", "turn": { "id": "u", "status": "completed", "items": [] } } }));
        assert!(c.done);
    }

    #[test]
    fn approvals_are_declined_and_failures_reported() {
        let mut c = Client::new(None, "hi".into());
        let (_, send) = feed(&mut c, json!({ "id": 1, "result": {} }));
        assert_eq!(send[1]["method"], "thread/start");
        feed(&mut c, json!({ "id": 3, "result": { "thread": { "id": "t" } } }));
        let (_, send) = feed(&mut c, json!({ "id": 99, "method": "item/commandExecution/requestApproval", "params": { "threadId": "t", "command": "rm -rf /" } }));
        assert_eq!(send[0], json!({ "jsonrpc": "2.0", "id": 99, "result": { "decision": "decline" } }));
        let (_, send) = feed(&mut c, json!({ "id": 100, "method": "mcpServer/elicitation/request", "params": { "serverName": "evil", "message": "?" } }));
        assert_eq!(send[0]["result"]["action"], "decline");
        let (_, send) = feed(&mut c, json!({ "id": 101, "method": "something/new", "params": {} }));
        assert_eq!(send[0]["error"]["code"], -32601);
        let (e, _) = feed(&mut c, json!({ "method": "turn/completed", "params": { "threadId": "t", "turn": { "status": "failed", "error": { "message": "401 Unauthorized" } } } }));
        assert!(matches!(&e[0], Event::Error(m) if m.contains("codex login")));
    }

    #[test]
    fn locks_the_cli_down() {
        let a = args();
        assert_eq!(a[0], "app-server");
        assert!(a.contains(&"features.shell_tool=false".to_string()));
        assert!(a.contains(&"features.unified_exec=false".to_string()));
        let servers = a.iter().find(|x| x.starts_with("mcp_servers=")).unwrap();
        assert!(servers.contains("args = [\"--mcp\"]"), "{servers}");
    }
}
