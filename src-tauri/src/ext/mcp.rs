//! Model Context Protocol client: connects to MCP servers and offers their tools to the
//! agent as `mcp__<server>__<tool>`.
//!
//! Servers are configured in `~/.config/pilunch/mcp.json` (the same shape Claude Desktop
//! and other MCP hosts use, so configs can be pasted in) or by plugins:
//!
//! ```json
//! { "mcpServers": {
//!     "github": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": { "GITHUB_TOKEN": "…" } },
//!     "docs":   { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer …" } }
//! } }
//! ```
//!
//! Both transports are supported: stdio (newline-delimited JSON-RPC to a child process)
//! and Streamable HTTP (JSON-RPC over POST, answered with JSON or an SSE stream).

use crate::agent::sse::SseParser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConfig {
    /// stdio: the program to start
    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    /// Streamable HTTP: the endpoint URL
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct McpFile {
    #[serde(rename = "mcpServers", default)]
    pub servers: BTreeMap<String, ServerConfig>,
}

impl McpFile {
    pub fn load(config_dir: &Path) -> Self {
        std::fs::read(config_dir.join("mcp.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Validate and save raw JSON (the editor in the UI); secrets inside → owner-only file.
    pub fn save_raw(config_dir: &Path, raw: &str) -> Result<Self, String> {
        let file: McpFile = serde_json::from_str(raw).map_err(|e| format!("Invalid mcp.json: {e}"))?;
        for (name, s) in &file.servers {
            if s.command.trim().is_empty() && s.url.trim().is_empty() {
                return Err(format!("Server \"{name}\" needs a \"command\" (stdio) or a \"url\" (HTTP)"));
            }
        }
        let data = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
        crate::util::write_private(&config_dir.join("mcp.json"), &data).map_err(|e| e.to_string())?;
        Ok(file)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// The server says the tool doesn't modify anything (`annotations.readOnlyHint`).
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub name: String,
    /// "connected" | "error" | "disabled"
    pub state: String,
    pub transport: String,
    pub source: String,
    pub tools: Vec<String>,
    pub error: Option<String>,
}

/// Output of a tool call.
pub struct CallOutput {
    pub text: String,
    pub images: Vec<(String, String)>,
    pub is_error: bool,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

enum Transport {
    Stdio {
        stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
        pending: Pending,
        _child: tokio::process::Child,
    },
    Http {
        http: reqwest::Client,
        url: String,
        headers: BTreeMap<String, String>,
        session: Mutex<Option<String>>,
    },
}

pub struct Client {
    pub name: String,
    config: ServerConfig,
    transport: Transport,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    stderr: Arc<Mutex<String>>,
    pub tools: Vec<ToolInfo>,
}

/// Replace `${PLUGIN_DIR}` (and `${CONFIG_DIR}`) in config strings.
fn substitute(s: &str, vars: &[(&str, &Path)]) -> String {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), &v.display().to_string());
    }
    out
}

impl Client {
    pub async fn connect(name: &str, cfg: &ServerConfig, http: &reqwest::Client, vars: &[(&str, &Path)]) -> Result<Client, String> {
        let transport = if !cfg.url.trim().is_empty() {
            Transport::Http {
                http: http.clone(),
                url: substitute(cfg.url.trim(), vars),
                headers: cfg.headers.iter().map(|(k, v)| (k.clone(), substitute(v, vars))).collect(),
                session: Mutex::new(None),
            }
        } else {
            let program = substitute(cfg.command.trim(), vars);
            let mut cmd = crate::process::command(&program);
            cmd.args(cfg.args.iter().map(|a| substitute(a, vars)))
                .envs(cfg.env.iter().map(|(k, v)| (k.clone(), substitute(v, vars))))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            if !cfg.cwd.trim().is_empty() {
                cmd.current_dir(substitute(cfg.cwd.trim(), vars));
            }
            let mut child = cmd.spawn().map_err(|e| format!("Couldn't start `{program}`: {e}"))?;
            let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take().expect("piped stdin")));
            Transport::Stdio { stdin, pending: Arc::default(), _child: child }
        };
        let mut client = Client { name: name.to_string(), config: cfg.clone(), transport, next_id: AtomicU64::new(1), alive: Arc::new(AtomicBool::new(true)), stderr: Arc::default(), tools: Vec::new() };
        if let Transport::Stdio { .. } = &client.transport {
            client.spawn_readers_from_child_handles();
        }
        let init = tokio::time::timeout(
            CONNECT_TIMEOUT,
            client.request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "PiLunch", "version": env!("CARGO_PKG_VERSION") }
                }),
            ),
        )
        .await
        .map_err(|_| client.with_stderr("The server didn't answer `initialize` within 30s".into()))?
        .map_err(|e| client.with_stderr(e))?;
        if init.get("protocolVersion").is_none() {
            return Err(client.with_stderr("The server's `initialize` reply has no protocolVersion".into()));
        }
        client.notify("notifications/initialized", json!({})).await;
        client.tools = client.list_tools().await.map_err(|e| client.with_stderr(e))?;
        Ok(client)
    }

    fn with_stderr(&self, msg: String) -> String {
        let tail = self.stderr.lock().unwrap().trim().to_string();
        if tail.is_empty() { msg } else { format!("{msg}\n{}", crate::util::truncate_end(&tail, 2000)) }
    }

    /// Start the stdout (responses) and stderr (log tail) readers for a stdio server.
    fn spawn_readers_from_child_handles(&mut self) {
        let Transport::Stdio { stdin, pending, _child } = &mut self.transport else { return };
        let stdout = _child.stdout.take().expect("piped stdout");
        let stderr = _child.stderr.take().expect("piped stderr");
        let (pending, stdin, alive) = (pending.clone(), stdin.clone(), self.alive.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
                if let Some(reply) = server_request_reply(&msg) {
                    let mut w = stdin.lock().await;
                    let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
                    let _ = w.flush().await;
                    continue;
                }
                if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(msg);
                    }
                }
            }
            alive.store(false, Ordering::SeqCst);
            pending.lock().unwrap().clear();
        });
        let tail = self.stderr.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(stderr);
            let mut buf = vec![0u8; 4096];
            while let Ok(n) = r.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let mut t = tail.lock().unwrap();
                t.push_str(&String::from_utf8_lossy(&buf[..n]));
                if t.len() > 8000 {
                    let cut = t.len() - 4000;
                    let cut = (cut..t.len()).find(|i| t.is_char_boundary(*i)).unwrap_or(t.len());
                    t.drain(..cut);
                }
            }
        });
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn transport_name(&self) -> &'static str {
        match self.transport {
            Transport::Stdio { .. } => "stdio",
            Transport::Http { .. } => "http",
        }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let reply = match &self.transport {
            Transport::Stdio { stdin, pending, .. } => {
                if !self.is_alive() {
                    return Err("the server process has exited".into());
                }
                let (tx, rx) = oneshot::channel();
                pending.lock().unwrap().insert(id, tx);
                {
                    let mut w = stdin.lock().await;
                    w.write_all(format!("{msg}\n").as_bytes()).await.map_err(|e| format!("write failed: {e}"))?;
                    w.flush().await.map_err(|e| format!("write failed: {e}"))?;
                }
                rx.await.map_err(|_| "the server closed the connection".to_string())?
            }
            Transport::Http { .. } => self.http_post(&msg, Some(id)).await?.ok_or("empty response")?,
        };
        if let Some(err) = reply.get("error") {
            return Err(err.get("message").and_then(Value::as_str).unwrap_or("MCP error").to_string());
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let mut w = stdin.lock().await;
                let _ = w.write_all(format!("{msg}\n").as_bytes()).await;
                let _ = w.flush().await;
            }
            Transport::Http { .. } => {
                let _ = self.http_post(&msg, None).await;
            }
        }
    }

    /// POST one JSON-RPC message; returns the response with id `want` (JSON or SSE).
    async fn http_post(&self, msg: &Value, want: Option<u64>) -> Result<Option<Value>, String> {
        let Transport::Http { http, url, headers, session } = &self.transport else { unreachable!() };
        let mut req = http
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", PROTOCOL_VERSION)
            .timeout(CALL_TIMEOUT)
            .json(msg);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(s) = session.lock().unwrap().clone() {
            req = req.header("mcp-session-id", s);
        }
        let mut resp = req.send().await.map_err(|e| format!("can't reach {url}: {}", crate::agent::api::describe_reqwest(&e)))?;
        if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *session.lock().unwrap() = Some(s.to_string());
        }
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("{status}: {}", crate::util::truncate_end(body.trim(), 400)));
        }
        let Some(want) = want else { return Ok(None) };
        let is_sse = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.contains("event-stream"));
        if !is_sse {
            let text = resp.text().await.map_err(|e| e.to_string())?;
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("bad JSON from server: {e}"))?;
            // A batch response is an array.
            let found = match v {
                Value::Array(items) => items.into_iter().find(|m| m["id"].as_u64() == Some(want)),
                m => Some(m),
            };
            return Ok(found);
        }
        let mut parser = SseParser::default();
        let mut events = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            parser.feed(&chunk, &mut events);
            for ev in events.drain(..) {
                let Ok(m) = serde_json::from_str::<Value>(&ev.data) else { continue };
                if m["id"].as_u64() == Some(want) && (m.get("result").is_some() || m.get("error").is_some()) {
                    return Ok(Some(m));
                }
            }
        }
        Err("the server ended the stream without a response".into())
    }

    async fn list_tools(&self) -> Result<Vec<ToolInfo>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let r = self.request("tools/list", params).await?;
            for t in r["tools"].as_array().into_iter().flatten() {
                let Some(name) = t["name"].as_str() else { continue };
                tools.push(ToolInfo {
                    name: name.to_string(),
                    description: t["description"].as_str().unwrap_or_default().to_string(),
                    input_schema: t.get("inputSchema").cloned().filter(Value::is_object).unwrap_or_else(|| json!({ "type": "object" })),
                    read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
                });
            }
            cursor = r["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    pub async fn call(&self, tool: &str, args: Value) -> Result<CallOutput, String> {
        let r = tokio::time::timeout(CALL_TIMEOUT, self.request("tools/call", json!({ "name": tool, "arguments": args })))
            .await
            .map_err(|_| format!("{tool} timed out after {}s", CALL_TIMEOUT.as_secs()))??;
        Ok(parse_call_result(&r))
    }
}

/// Answer requests the server sends to us (ping, roots/list); `None` for anything else.
fn server_request_reply(msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id")?.clone();
    Some(match method {
        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        "roots/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "roots": [] } }),
        _ => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("{method} is not supported by PiLunch") } }),
    })
}

pub fn parse_call_result(r: &Value) -> CallOutput {
    let mut text = Vec::new();
    let mut images = Vec::new();
    for c in r["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => text.push(c["text"].as_str().unwrap_or_default().to_string()),
            Some("image") => images.push((c["mimeType"].as_str().unwrap_or("image/png").to_string(), c["data"].as_str().unwrap_or_default().to_string())),
            Some("resource") => {
                let res = &c["resource"];
                text.push(res["text"].as_str().map(String::from).unwrap_or_else(|| format!("[resource {}]", res["uri"].as_str().unwrap_or("?"))));
            }
            Some("resource_link") => text.push(format!("[resource {}]", c["uri"].as_str().unwrap_or("?"))),
            _ => {}
        }
    }
    if text.is_empty() {
        if let Some(sc) = r.get("structuredContent") {
            text.push(sc.to_string());
        }
    }
    CallOutput { text: text.join("\n"), images, is_error: r["isError"].as_bool().unwrap_or(false) }
}

/// The agent-facing name of a server's tool (API names: `[a-zA-Z0-9_-]{1,64}`).
pub fn tool_name(server: &str, tool: &str) -> String {
    let clean = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect::<String>();
    let mut n = format!("mcp__{}__{}", clean(server), clean(tool));
    n.truncate(64);
    n
}

/// A configured server plus where it came from (mcp.json or a plugin folder).
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub config: ServerConfig,
    /// "mcp.json" or "plugin:<name>"
    pub origin: String,
    pub plugin_dir: Option<PathBuf>,
}

/// Live connections, kept across runs and reconnected when the config changes or a
/// server dies.
#[derive(Default)]
pub struct McpManager {
    clients: tokio::sync::Mutex<HashMap<String, Arc<Client>>>,
    status: Mutex<BTreeMap<String, ServerStatus>>,
}

impl McpManager {
    /// Connect every enabled server (in parallel) and return the live clients.
    pub async fn ensure(&self, sources: &BTreeMap<String, Source>, http: &reqwest::Client, config_dir: &Path) -> Vec<Arc<Client>> {
        let mut clients = self.clients.lock().await;
        clients.retain(|name, c| sources.get(name).is_some_and(|s| !s.config.disabled && s.config == c.config && c.is_alive()));
        let todo: Vec<(&String, &Source)> = sources.iter().filter(|(n, s)| !s.config.disabled && !clients.contains_key(*n)).collect();
        let results = futures_util::future::join_all(todo.iter().map(|(name, src)| async move {
            let plugin_dir = src.plugin_dir.clone().unwrap_or_else(|| config_dir.to_path_buf());
            let vars: [(&str, &Path); 2] = [("PLUGIN_DIR", &plugin_dir), ("CONFIG_DIR", config_dir)];
            (name.to_string(), Client::connect(name, &src.config, http, &vars).await)
        }))
        .await;
        let mut status = self.status.lock().unwrap();
        status.clear();
        for (name, res) in results {
            match res {
                Ok(c) => {
                    clients.insert(name, Arc::new(c));
                }
                Err(e) => {
                    let src = &sources[&name];
                    status.insert(name.clone(), ServerStatus { name, state: "error".into(), transport: transport_of(&src.config).into(), source: src.origin.clone(), tools: vec![], error: Some(e) });
                }
            }
        }
        for (name, src) in sources {
            if src.config.disabled {
                status.insert(name.clone(), ServerStatus { name: name.clone(), state: "disabled".into(), transport: transport_of(&src.config).into(), source: src.origin.clone(), ..Default::default() });
            } else if let Some(c) = clients.get(name) {
                status.insert(
                    name.clone(),
                    ServerStatus { name: name.clone(), state: "connected".into(), transport: c.transport_name().into(), source: src.origin.clone(), tools: c.tools.iter().map(|t| t.name.clone()).collect(), error: None },
                );
            }
        }
        clients.values().cloned().collect()
    }

    pub fn status(&self) -> Vec<ServerStatus> {
        self.status.lock().unwrap().values().cloned().collect()
    }

    /// Drop a server's connection so the next `ensure` reconnects it.
    pub async fn disconnect(&self, name: &str) {
        self.clients.lock().await.remove(name);
    }

    pub async fn shutdown(&self) {
        self.clients.lock().await.clear();
    }
}

fn transport_of(c: &ServerConfig) -> &'static str {
    if c.url.trim().is_empty() { "stdio" } else { "http" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_claude_desktop_format() {
        let d = tempfile::tempdir().unwrap();
        let raw = r#"{"mcpServers":{"fs":{"command":"npx","args":["-y","x"],"env":{"K":"v"}},"web":{"url":"https://e.com/mcp","headers":{"Authorization":"Bearer t"}}}}"#;
        let f = McpFile::save_raw(d.path(), raw).unwrap();
        assert_eq!(f.servers["fs"].args, vec!["-y", "x"]);
        assert_eq!(McpFile::load(d.path()), f);
        assert!(McpFile::save_raw(d.path(), r#"{"mcpServers":{"bad":{}}}"#).is_err());
        assert!(McpFile::save_raw(d.path(), "nope").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(d.path().join("mcp.json")).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn names_results_and_server_requests() {
        assert_eq!(tool_name("my server", "get.issue"), "mcp__my_server__get_issue");
        assert!(tool_name(&"s".repeat(50), &"t".repeat(50)).len() <= 64);
        let out = parse_call_result(&json!({"content":[{"type":"text","text":"a"},{"type":"image","data":"QQ==","mimeType":"image/jpeg"},{"type":"resource","resource":{"uri":"file:///x","text":"body"}}],"isError":true}));
        assert_eq!(out.text, "a\nbody");
        assert_eq!(out.images, vec![("image/jpeg".to_string(), "QQ==".to_string())]);
        assert!(out.is_error);
        assert_eq!(server_request_reply(&json!({"jsonrpc":"2.0","id":7,"method":"ping"})).unwrap()["result"], json!({}));
        assert_eq!(server_request_reply(&json!({"jsonrpc":"2.0","id":8,"method":"sampling/createMessage"})).unwrap()["error"]["code"], -32601);
        assert!(server_request_reply(&json!({"jsonrpc":"2.0","id":9,"result":{}})).is_none());
        assert_eq!(substitute("${PLUGIN_DIR}/bin/x", &[("PLUGIN_DIR", Path::new("/p"))]), "/p/bin/x");
    }

    /// A tiny stdio MCP server written in shell + the client talking to it for real.
    #[cfg(unix)]
    #[test]
    fn stdio_server_end_to_end() {
        let d = tempfile::tempdir().unwrap();
        let script = d.path().join("server.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"t","version":"1"}}}\n' "$id" ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}},"annotations":{"readOnlyHint":true}}]}}\n' "$id" ;;
    *'"tools/call"'*) echo "log line" >&2; printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"pong"}]}}\n' "$id" ;;
  esac
done
"#,
        )
        .unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let cfg = ServerConfig { command: "sh".into(), args: vec![script.display().to_string()], ..Default::default() };
            let http = reqwest::Client::new();
            let mgr = McpManager::default();
            let sources: BTreeMap<String, Source> = [("t".to_string(), Source { config: cfg.clone(), origin: "mcp.json".into(), plugin_dir: None })].into();
            let clients = mgr.ensure(&sources, &http, d.path()).await;
            assert_eq!(clients.len(), 1, "{:?}", mgr.status());
            let c = &clients[0];
            assert_eq!(c.tools[0].name, "echo");
            assert!(c.tools[0].read_only);
            let out = c.call("echo", json!({"text":"ping"})).await.unwrap();
            assert_eq!(out.text, "pong");
            assert_eq!(mgr.status()[0].state, "connected");
            // reusing the same connection on the next ensure
            let again = mgr.ensure(&sources, &http, d.path()).await;
            assert!(Arc::ptr_eq(&again[0], &clients[0]));
            // a broken server reports an error status
            let bad: BTreeMap<String, Source> = [("b".to_string(), Source { config: ServerConfig { command: "/nonexistent/x".into(), ..Default::default() }, origin: "mcp.json".into(), plugin_dir: None })].into();
            assert!(mgr.ensure(&bad, &http, d.path()).await.is_empty());
            assert_eq!(mgr.status()[0].state, "error");
        });
    }
}
