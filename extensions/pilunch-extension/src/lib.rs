//! Write PiLunch extensions in Rust.
//!
//! An extension is a small program that gives the PiLunch agent new tools. It speaks the
//! Model Context Protocol over stdin/stdout, so it also works in any other MCP host.
//! This crate handles the protocol; you describe tools and write their handlers:
//!
//! ```no_run
//! use pilunch_extension::{json, Extension, Output};
//!
//! fn main() {
//!     Extension::new("hello-ext", env!("CARGO_PKG_VERSION"))
//!         .tool(
//!             "greet",
//!             "Greet someone by name.",
//!             json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }),
//!             |args| Ok(Output::text(format!("Hello, {}!", args["name"].as_str().unwrap_or("world")))),
//!         )
//!         .run();
//! }
//! ```
//!
//! Build and install it from PiLunch: *Customize → Extensions → Build Rust extension*,
//! then pick the crate folder. See docs/extensions.md in the PiLunch repository.

pub use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// Protocol versions this crate speaks (newest first).
const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// What a tool returns: text and/or images.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Output {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl Output {
    pub fn text(s: impl Into<String>) -> Self {
        Output { content: vec![json!({ "type": "text", "text": s.into() })], is_error: false }
    }

    /// A base64-encoded image (e.g. "image/png").
    pub fn image(base64: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Output { content: vec![json!({ "type": "image", "data": base64.into(), "mimeType": mime_type.into() })], is_error: false }
    }

    /// Append more content (text after an image, several images…).
    pub fn and(mut self, other: Output) -> Self {
        self.content.extend(other.content);
        self
    }

    /// A failed call the model should see (and can recover from).
    pub fn error(s: impl Into<String>) -> Self {
        Output { is_error: true, ..Output::text(s) }
    }
}

type Handler = Box<dyn Fn(&Value) -> Result<Output, String> + Send + Sync>;

struct Tool {
    name: String,
    description: String,
    schema: Value,
    read_only: bool,
    handler: Handler,
}

pub struct Extension {
    name: String,
    version: String,
    instructions: Option<String>,
    tools: Vec<Tool>,
}

impl Extension {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Extension { name: name.into(), version: version.into(), instructions: None, tools: Vec::new() }
    }

    /// Optional guidance for the model on using this extension.
    pub fn instructions(mut self, text: impl Into<String>) -> Self {
        self.instructions = Some(text.into());
        self
    }

    /// Add a tool. `schema` is the JSON Schema of its input object. PiLunch asks the user
    /// before each call (unless they allowed tools for the chat).
    pub fn tool(self, name: &str, description: &str, schema: Value, handler: impl Fn(&Value) -> Result<Output, String> + Send + Sync + 'static) -> Self {
        self.add(name, description, schema, false, handler)
    }

    /// Add a tool that only reads (never changes anything): it runs without asking and is
    /// available in Plan mode.
    pub fn read_only_tool(self, name: &str, description: &str, schema: Value, handler: impl Fn(&Value) -> Result<Output, String> + Send + Sync + 'static) -> Self {
        self.add(name, description, schema, true, handler)
    }

    fn add(mut self, name: &str, description: &str, schema: Value, read_only: bool, handler: impl Fn(&Value) -> Result<Output, String> + Send + Sync + 'static) -> Self {
        self.tools.push(Tool { name: name.into(), description: description.into(), schema, read_only, handler: Box::new(handler) });
        self
    }

    /// Serve requests on stdin/stdout until stdin closes. Log with `eprintln!` — stdout
    /// is the protocol channel.
    pub fn run(self) {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout().lock();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } })),
            };
            if let Some(r) = reply {
                if writeln!(stdout, "{r}").and_then(|_| stdout.flush()).is_err() {
                    break;
                }
            }
        }
    }

    /// Handle one JSON-RPC message; `None` for notifications.
    pub fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id")?.clone();
        let method = msg["method"].as_str().unwrap_or_default();
        let params = &msg["params"];
        let result = match method {
            "initialize" => {
                let asked = params["protocolVersion"].as_str().unwrap_or(VERSIONS[0]);
                let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
                let mut r = json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": self.name, "version": self.version }
                });
                if let Some(i) = &self.instructions {
                    r["instructions"] = json!(i);
                }
                Ok(r)
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self.tools.iter().map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.schema,
                    "annotations": { "readOnlyHint": t.read_only }
                })).collect::<Vec<_>>()
            })),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                match self.tools.iter().find(|t| t.name == name) {
                    None => Err((-32602, format!("unknown tool {name}"))),
                    Some(t) => {
                        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (t.handler)(&args)))
                            .unwrap_or_else(|_| Err(format!("{name} panicked")));
                        let out = out.unwrap_or_else(Output::error);
                        Ok(json!({ "content": out.content, "isError": out.is_error }))
                    }
                }
            }
            _ => Err((-32601, format!("method {method} not found"))),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext() -> Extension {
        Extension::new("t", "1.0")
            .read_only_tool("add", "Add", json!({"type":"object"}), |a| Ok(Output::text((a["a"].as_i64().unwrap_or(0) + a["b"].as_i64().unwrap_or(0)).to_string())))
            .tool("fail", "Fails", json!({"type":"object"}), |_| Err("nope".into()))
    }

    #[test]
    fn speaks_mcp() {
        let e = ext();
        let init = e.handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(e.handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        let list = e.handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        assert_eq!(list["result"]["tools"][0]["annotations"]["readOnlyHint"], true);
        let r = e.handle(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"add","arguments":{"a":2,"b":3}}})).unwrap();
        assert_eq!(r["result"]["content"][0]["text"], "5");
        let r = e.handle(&json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"fail"}})).unwrap();
        assert_eq!(r["result"]["isError"], true);
        let r = e.handle(&json!({"jsonrpc":"2.0","id":5,"method":"nope"})).unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }
}
