//! `assistant --mcp`: the settings tools as a Model Context Protocol server
//! on stdio (newline-delimited JSON-RPC). The Claude and Codex CLIs start it.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::tools::{self, CallResult, Ctx, Effect};

/// The server's name in the CLIs' MCP config; Claude prefixes tools with `mcp__y7ya__`.
pub const SERVER: &str = "y7ya";

const PROTOCOLS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

pub fn serve(ctx: &Ctx) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(ctx, &msg),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })),
        };
        if let Some(r) = reply {
            if writeln!(stdout, "{r}").and_then(|_| stdout.flush()).is_err() {
                break;
            }
        }
    }
}

/// Handles one message; notifications get no reply.
pub fn handle(ctx: &Ctx, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let params = &msg["params"];
    let result = match msg["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = if PROTOCOLS.contains(&asked) { asked } else { "2025-06-18" };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "y7ya-settings", "title": "y7ya-os Settings", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Settings tools for y7ya-os. Read state before changing it; every change is verified and reported.",
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": list(ctx) })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            // Calls from the model are never pre-confirmed.
            let r = tools::call(ctx, name, &params["arguments"], false);
            let body = r.to_json();
            Ok(json!({
                "content": [{ "type": "text", "text": body.to_string() }],
                "structuredContent": body,
                "isError": matches!(r, CallResult::Failed(_)),
            }))
        }
        m => Err(format!("Method not found: {m}")),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": e } }),
    })
}

fn list(ctx: &Ctx) -> Vec<Value> {
    tools::all()
        .filter(|t| !(ctx.read_only && t.effect == Effect::Change))
        .map(|t| {
            let read = t.effect == Effect::Read;
            let asks = [json!({}), json!({ "enabled": false })].iter().any(|a| (t.confirm)(a).is_some());
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.schema)(),
                "annotations": {
                    "title": t.doing,
                    "readOnlyHint": read,
                    "destructiveHint": asks,
                    "idempotentHint": true,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(read_only: bool) -> Ctx {
        let dir = std::env::temp_dir().join(format!("assistant-mcp-{read_only}-{}", std::process::id()));
        Ctx { settings: dir.join("settings.toml"), read_only, ..Ctx::system() }
    }

    #[test]
    fn handshake_and_list() {
        let ctx = ctx(false);
        let r = handle(&ctx, &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } })).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert!(handle(&ctx, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let r = handle(&ctx, &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        let tools = r["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "set_appearance" && t["annotations"]["readOnlyHint"] == false));
        let wifi = tools.iter().find(|t| t["name"] == "set_wifi_enabled").unwrap();
        assert_eq!(wifi["annotations"]["destructiveHint"], true);
        let r = handle(&ctx, &json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" })).unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn read_only_lists_only_readers_and_confirmation_is_not_bypassable() {
        let ctx = ctx(true);
        let r = handle(&ctx, &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })).unwrap();
        assert!(r["result"]["tools"].as_array().unwrap().iter().all(|t| t["annotations"]["readOnlyHint"] == true));

        // Even with extra arguments, a risky call from the model only asks.
        let ctx = self::ctx(false);
        let r = handle(
            &ctx,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "set_wifi_enabled", "arguments": { "enabled": false, "confirmed": true } } }),
        )
        .unwrap();
        assert_eq!(r["result"]["structuredContent"]["needs_confirmation"], true);
        assert_eq!(r["result"]["isError"], false);
    }
}
