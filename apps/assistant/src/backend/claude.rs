//! Claude Code, headless: `claude -p` with stream-json in and out. Only our
//! MCP server's tools exist (`--tools ""` removes the built-in ones and
//! `--strict-mcp-config` ignores other servers), they're pre-approved with
//! `--allowedTools mcp__y7ya`, and `--permission-mode dontAsk` denies anything
//! else instead of prompting.

use std::io::Write;
use std::process::Child;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{Event, Kind, Proc, SYSTEM_PROMPT, Sink, died, mcp_command, tool_result};
use crate::mcp::SERVER;

pub fn args(session: Option<&str>) -> Vec<String> {
    let (exe, mcp_args) = mcp_command();
    let config = json!({ "mcpServers": { SERVER: { "type": "stdio", "command": exe, "args": mcp_args } } });
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--system-prompt",
        SYSTEM_PROMPT,
        "--mcp-config",
        &config.to_string(),
        "--strict-mcp-config",
        "--tools",
        "",
        "--allowedTools",
        &format!("mcp__{SERVER}"),
        "--permission-mode",
        "dontAsk",
        "--disable-slash-commands",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    if let Some(id) = session {
        a.extend(["--resume".into(), id.into()]);
    }
    a
}

/// The user message, as one stream-json line.
pub fn user_message(prompt: &str) -> Value {
    json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": prompt }] } })
}

/// Turns stream-json lines into events.
#[derive(Default)]
pub struct Parser {
    /// Text of the current message already arrived as deltas.
    streamed: bool,
    pub finished: bool,
}

impl Parser {
    /// Events for one line, plus a reply to write back (permission requests).
    pub fn line(&mut self, line: &str) -> (Vec<Event>, Option<Value>) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return (Vec::new(), None) };
        let mut out = Vec::new();
        // Subagent chatter (none expected) isn't the reply.
        if v["parent_tool_use_id"].is_string() {
            return (out, None);
        }
        match v["type"].as_str().unwrap_or_default() {
            "system" if v["subtype"] == "init" => {
                if let Some(id) = v["session_id"].as_str() {
                    out.push(Event::Session(id.into()));
                }
                let ours = v["mcp_servers"].as_array().and_then(|s| s.iter().find(|s| s["name"] == SERVER));
                if let Some(status) = ours.and_then(|s| s["status"].as_str()).filter(|s| *s != "connected" && *s != "pending") {
                    out.push(Event::Error(format!("The settings tools didn't start ({status}).")));
                }
            }
            "stream_event" => {
                let e = &v["event"];
                match e["type"].as_str().unwrap_or_default() {
                    "message_start" => self.streamed = false,
                    "content_block_delta" if e["delta"]["type"] == "text_delta" => {
                        if let Some(t) = e["delta"]["text"].as_str() {
                            self.streamed = true;
                            out.push(Event::Text(t.into()));
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                for block in v["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str().unwrap_or_default() {
                        "text" if !self.streamed => {
                            if let Some(t) = block["text"].as_str() {
                                out.push(Event::Text(t.into()));
                            }
                        }
                        "tool_use" => {
                            let name = block["name"].as_str().unwrap_or_default();
                            let name = name.strip_prefix(&format!("mcp__{SERVER}__")).unwrap_or(name);
                            out.push(Event::ToolStart {
                                id: block["id"].as_str().unwrap_or_default().into(),
                                name: name.into(),
                                args: block["input"].clone(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in v["message"]["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "tool_result") {
                    let text = match &block["content"] {
                        Value::String(s) => s.clone(),
                        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
                        _ => String::new(),
                    };
                    let is_error = block["is_error"].as_bool().unwrap_or(false);
                    out.push(Event::ToolEnd {
                        id: block["tool_use_id"].as_str().unwrap_or_default().into(),
                        result: tool_result(&text, is_error),
                    });
                }
            }
            "result" => {
                self.finished = true;
                if let Some(id) = v["session_id"].as_str() {
                    out.push(Event::Session(id.into()));
                }
                if v["is_error"].as_bool() == Some(true) || v["subtype"].as_str().is_some_and(|s| s != "success") {
                    let msg = v["result"].as_str().map(str::to_string).or_else(|| {
                        v["errors"].as_array().map(|e| e.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" "))
                    });
                    out.push(Event::Error(friendly(msg.as_deref().unwrap_or("Claude couldn't finish."))));
                }
            }
            "control_request" => {
                // Shouldn't happen with dontAsk; never approve anything here.
                let reply = json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": v["request_id"],
                        "response": { "behavior": "deny", "message": "Only the y7ya settings tools are available." },
                    },
                });
                return (out, Some(reply));
            }
            _ => {}
        }
        (out, None)
    }
}

fn friendly(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("/login") || lower.contains("not logged in") || lower.contains("invalid api key") {
        return format!("Claude Code isn't signed in. Run \u{201c}{}\u{201d} in a terminal, then try again.", Kind::Claude.login_command());
    }
    msg.to_string()
}

pub fn run(session: Option<String>, prompt: &str, sink: &Sink, slot: &Arc<Mutex<Option<Child>>>) -> Result<(), String> {
    let mut p = Proc::spawn(Kind::Claude, &args(session.as_deref()), slot)?;
    let mut parser = Parser::default();
    if let Some(stdin) = p.stdin.as_mut() {
        writeln!(stdin, "{}", user_message(prompt)).and_then(|_| stdin.flush()).map_err(|e| format!("Couldn't talk to Claude Code: {e}"))?;
    }
    while let Some(line) = p.next_line() {
        let (events, reply) = parser.line(&line);
        for e in events {
            sink(e);
        }
        if let (Some(r), Some(stdin)) = (reply, p.stdin.as_mut()) {
            let _ = writeln!(stdin, "{r}").and_then(|_| stdin.flush());
        }
        if parser.finished {
            // Closing stdin ends the session; the CLI exits.
            break;
        }
    }
    let finished = parser.finished;
    let err = (!finished).then(|| died(Kind::Claude, &p));
    p.finish();
    err.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::CallResult;

    /// Lines in the shape `claude -p --output-format stream-json --verbose
    /// --include-partial-messages` prints (trimmed of fields we ignore).
    const TRANSCRIPT: &str = r#"{"type":"system","subtype":"init","session_id":"3443214a","tools":["mcp__y7ya__set_appearance"],"mcp_servers":[{"name":"y7ya","status":"connected"}],"model":"claude-sonnet-5-5","permissionMode":"dontAsk"}
{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant","content":[]}},"session_id":"3443214a","parent_tool_use_id":null}
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"mcp__y7ya__set_appearance","input":{}}},"parent_tool_use_id":null}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"mcp__y7ya__set_appearance","input":{"mode":"dark"}}]},"parent_tool_use_id":null,"session_id":"3443214a"}
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_1","type":"tool_result","content":[{"type":"text","text":"{\"ok\":true,\"summary\":\"Changed appearance to Dark\",\"state\":{\"mode\":\"dark\"},\"undo\":{\"tool\":\"set_appearance\",\"args\":{\"mode\":\"auto\"}}}"}]}]},"parent_tool_use_id":null,"session_id":"3443214a"}
{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant","content":[]}},"parent_tool_use_id":null}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Done — "}},"parent_tool_use_id":null}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"you're in Dark mode."}},"parent_tool_use_id":null}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done — you're in Dark mode."}]},"parent_tool_use_id":null}
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}
{"type":"result","subtype":"success","is_error":false,"result":"Done — you're in Dark mode.","session_id":"3443214a"}"#;

    #[test]
    fn parses_a_turn_with_a_tool_call() {
        let mut p = Parser::default();
        let events: Vec<Event> = TRANSCRIPT.lines().flat_map(|l| p.line(l).0).collect();
        assert!(p.finished);
        assert_eq!(events[0], Event::Session("3443214a".into()));
        assert_eq!(events[1], Event::ToolStart { id: "toolu_1".into(), name: "set_appearance".into(), args: json!({ "mode": "dark" }) });
        let Event::ToolEnd { id, result: CallResult::Done(o) } = &events[2] else { panic!("{:?}", events[2]) };
        assert_eq!(id, "toolu_1");
        assert_eq!(o.summary, "Changed appearance to Dark");
        assert_eq!(o.undo.as_ref().unwrap().args, json!({ "mode": "auto" }));
        // Text arrives once, from the deltas (the full message isn't repeated).
        let text: String = events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
        assert_eq!(text, "Done — you're in Dark mode.");
        assert_eq!(events.last(), Some(&Event::Session("3443214a".into())));
    }

    #[test]
    fn errors_and_permission_requests() {
        let mut p = Parser::default();
        let (e, _) = p.line(r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login","session_id":"x"}"#);
        assert!(matches!(&e[1], Event::Error(m) if m.contains("claude auth login")));
        let (_, reply) = p.line(r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf ~"}}}"#);
        let reply = reply.unwrap();
        assert_eq!(reply["response"]["request_id"], "r1");
        assert_eq!(reply["response"]["response"]["behavior"], "deny");
        let (e, _) = p.line(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","is_error":true,"content":"Claude requested permissions to use Bash, but you haven't granted it yet."}]}}"#);
        assert!(matches!(&e[0], Event::ToolEnd { result: CallResult::Failed(m), .. } if m.contains("permissions")));
        let (e, _) = p.line(r#"{"type":"system","subtype":"init","session_id":"s","mcp_servers":[{"name":"y7ya","status":"failed"}]}"#);
        assert!(matches!(&e[1], Event::Error(_)));
    }

    #[test]
    fn locks_the_cli_down() {
        let a = args(Some("abc"));
        let has = |flag: &str, value: &str| a.windows(2).any(|w| w[0] == flag && w[1] == value);
        assert!(has("--tools", ""));
        assert!(has("--allowedTools", "mcp__y7ya"));
        assert!(has("--permission-mode", "dontAsk"));
        assert!(has("--resume", "abc"));
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        let config: Value = serde_json::from_str(&a[a.iter().position(|x| x == "--mcp-config").unwrap() + 1]).unwrap();
        assert_eq!(config["mcpServers"]["y7ya"]["args"], json!(["--mcp"]));
    }
}
