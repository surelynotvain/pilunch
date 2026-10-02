//! The coding agent: a streaming tool-use loop over the Claude Messages API.
//!
//! One run = one user message. The loop streams a turn, appends the assistant message to
//! the (append-only) history, executes any tool calls — asking the user for approval
//! according to the permission mode — appends the tool results, and repeats until Claude
//! ends its turn. The conversation is saved after every step.

pub mod api;
pub mod events;
pub mod prompt;
pub mod sse;
pub mod toolbox;
pub mod tools;

#[cfg(test)]
mod loop_tests;

use crate::conversations::{title_from, Conversation, StoredMessage, ToolStatus, ToolUi, UserDisplay};
use crate::error::{Error, Result};
use crate::settings::{PermissionMode, Settings};
use crate::state::AppState;
use crate::util::{looks_binary, now_ms, truncate_end};
use crate::workspace::Workspace;
use api::{ApiError, RequestParts};
use events::{AgentEvent, Sink};
use serde::Deserialize;
use serde_json::{json, Value};
use sse::{AssistantTurn, MessageBuilder, SseParser, StreamEvent};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const MAX_RETRIES: u32 = 4;
const MAX_ATTACHMENT: usize = 256 * 1024;
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_CONSECUTIVE_TRUNCATIONS: u32 = 2;

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    /// Allow this and every later call of the same kind in this conversation.
    AllowSession,
    Deny,
}

#[derive(Default, Clone, Copy)]
struct Grants {
    edits: bool,
    commands: bool,
    network: bool,
}

type ApprovalReply = oneshot::Sender<(Decision, Option<String>)>;

#[derive(Default)]
pub struct AgentManager {
    runs: Mutex<HashMap<String, CancellationToken>>,
    approvals: Mutex<HashMap<String, ApprovalReply>>,
    grants: Mutex<HashMap<String, Grants>>,
}

impl AgentManager {
    pub fn running(&self) -> Vec<String> {
        self.runs.lock().unwrap().keys().cloned().collect()
    }

    pub fn cancel(&self, conversation_id: &str) {
        if let Some(t) = self.runs.lock().unwrap().get(conversation_id) {
            t.cancel();
        }
    }

    pub fn cancel_all(&self) {
        for t in self.runs.lock().unwrap().values() {
            t.cancel();
        }
    }

    pub fn respond(&self, approval_id: &str, decision: Decision, feedback: Option<String>) -> Result<()> {
        let tx = self.approvals.lock().unwrap().remove(approval_id).ok_or_else(|| Error::msg("approval is no longer pending"))?;
        let _ = tx.send((decision, feedback));
        Ok(())
    }

    fn grants(&self, conversation_id: &str) -> Grants {
        self.grants.lock().unwrap().get(conversation_id).copied().unwrap_or_default()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    pub conversation_id: String,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<String>,
}

/// Start a run in the background. Events are delivered on `channel`.
pub fn start<R: Runtime>(app: AppHandle<R>, req: SendRequest, channel: Channel<AgentEvent>) -> Result<()> {
    let state = app.state::<AppState>();
    let token = CancellationToken::new();
    {
        let mut runs = state.agent.runs.lock().unwrap();
        if runs.contains_key(&req.conversation_id) {
            return Err(Error::msg("This conversation is already running"));
        }
        runs.insert(req.conversation_id.clone(), token.clone());
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let conv_id = req.conversation_id.clone();
        let mut sink = Sink::new(channel);
        let stop = Run::execute(&app2, req, &mut sink, token).await;
        sink.send(AgentEvent::Done { stop_reason: stop });
        app2.state::<AppState>().agent.runs.lock().unwrap().remove(&conv_id);
    });
    Ok(())
}

enum StreamFail {
    Cancelled(Vec<Value>),
    Api(ApiError),
}

struct Run<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    state: &'a AppState,
    settings: Settings,
    api_key: String,
    ws: Option<Workspace>,
    conv: Conversation,
    cancel: CancellationToken,
}

impl<'a, R: Runtime> Run<'a, R> {
    async fn execute(app: &'a AppHandle<R>, req: SendRequest, sink: &mut Sink, cancel: CancellationToken) -> String {
        let state: &AppState = app.state::<AppState>().inner();
        let fail = |sink: &mut Sink, message: String| {
            sink.send(AgentEvent::Error { message });
            "error".to_string()
        };
        let conv = match state.conversations.get(&req.conversation_id) {
            Ok(c) => c,
            Err(e) => return fail(sink, e.to_string()),
        };
        let Some(api_key) = state.settings.api_key() else {
            return fail(sink, "No API key configured. Add your Anthropic API key in Settings (Ctrl+,) or set ANTHROPIC_API_KEY.".into());
        };
        let ws = match conv.meta.workspace.as_deref().map(Workspace::open).transpose() {
            Ok(ws) => ws,
            Err(e) => return fail(sink, format!("The folder for this chat is unavailable: {e}")),
        };
        let mut run = Run { app, state, settings: state.settings.get(), api_key, ws, conv, cancel };
        run.loop_(req, sink).await
    }

    fn save(&mut self) {
        self.conv.meta.updated_at = now_ms();
        if let Err(e) = self.state.conversations.save(&self.conv) {
            eprintln!("failed to save conversation: {e}");
        }
    }

    fn append(&mut self, sink: &mut Sink, message: StoredMessage) {
        self.conv.messages.push(message.clone());
        self.save();
        sink.send(AgentEvent::MessageAppended { message });
    }

    async fn loop_(&mut self, req: SendRequest, sink: &mut Sink) -> String {
        self.repair_dangling_tool_uses(sink);

        let (text, attached) = build_user_text(self.ws.as_ref(), &req.text, &req.attachments);
        if self.conv.messages.is_empty() || self.conv.meta.title == "New chat" {
            self.conv.meta.title = title_from(&req.text);
            sink.send(AgentEvent::Title { title: self.conv.meta.title.clone() });
        }
        self.append(
            sink,
            StoredMessage {
                role: "user".into(),
                content: vec![json!({ "type": "text", "text": text })],
                display: Some(UserDisplay { text: req.text.clone(), attachments: attached }),
                model: None,
                ts: now_ms(),
            },
        );

        let system = prompt::system_prompt(self.ws.as_ref(), self.state.settings.permission_mode(), &self.settings.custom_instructions);
        let eager = self.settings.is_default_endpoint();
        let mut truncations = 0;

        loop {
            if self.cancel.is_cancelled() || sink.closed {
                return "cancelled".into();
            }
            let mode = self.state.settings.permission_mode();
            let web_search = self.settings.web_search && self.settings.is_default_endpoint() && api::ModelCaps::of(&self.settings.model).adaptive;
            let tools = if self.ws.is_some() {
                tools::definitions(&tools::ToolOptions { mode, eager, web_search })
            } else if web_search {
                vec![toolbox::web_search_def()]
            } else {
                Vec::new()
            };
            let (body, betas) = api::build_request(RequestParts {
                settings: &self.settings,
                system: &system,
                tools,
                messages: self.conv.api_messages(),
            });

            let turn = match self.stream_with_retries(&body, &betas, sink).await {
                Ok(t) => t,
                Err(StreamFail::Cancelled(partial)) => {
                    sink.send(AgentEvent::Reset);
                    if !partial.is_empty() {
                        self.append(sink, assistant_message(partial, None));
                    }
                    return "cancelled".into();
                }
                Err(StreamFail::Api(e)) => {
                    sink.send(AgentEvent::Reset);
                    sink.send(AgentEvent::Error { message: e.user_message() });
                    return "error".into();
                }
            };
            self.record_usage(&turn, sink);

            if turn.stop_reason.as_deref() == Some("refusal") {
                // The partial output of a declined request is discarded, not kept as an answer.
                sink.send(AgentEvent::Reset);
                let explanation = turn
                    .stop_details
                    .as_ref()
                    .and_then(|d| d.get("explanation"))
                    .and_then(Value::as_str)
                    .unwrap_or("Claude declined to continue with this request.");
                sink.send(AgentEvent::Refusal { message: explanation.to_string() });
                return "refusal".into();
            }

            let tool_uses: Vec<(String, String, Value)> = turn
                .content
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .filter_map(|b| {
                    Some((b.get("id")?.as_str()?.to_string(), b.get("name")?.as_str()?.to_string(), b.get("input").cloned().unwrap_or(json!({}))))
                })
                .collect();
            let stop = turn.stop_reason.clone().unwrap_or_else(|| "end_turn".into());
            if turn.content.is_empty() {
                sink.send(AgentEvent::Reset);
                return stop;
            }
            self.append(sink, assistant_message(turn.content.clone(), Some(turn.model.clone())));

            match stop.as_str() {
                "tool_use" if !tool_uses.is_empty() => truncations = 0,
                "pause_turn" => continue,
                "max_tokens" if !tool_uses.is_empty() => {
                    // A tool input cut off by max_tokens can still parse as valid JSON: never run it.
                    truncations += 1;
                    let results = tool_uses
                        .iter()
                        .map(|(id, _, _)| {
                            tool_result(id, "Your response hit the output token limit while writing this tool call, so it was not run. Retry with smaller steps (e.g. several edit_file calls instead of one large write_file).", true)
                        })
                        .collect();
                    for (id, name, input) in &tool_uses {
                        self.set_tool_ui(sink, id, ToolUi { status: ToolStatus::Error, summary: tools::describe(name, input), detail: Some("Not run: output was truncated (max_tokens).".into()), detail_kind: Some("text".into()), path: None });
                    }
                    self.append(sink, user_tool_results(results));
                    if truncations > MAX_CONSECUTIVE_TRUNCATIONS {
                        sink.send(AgentEvent::Notice { message: "Stopped: responses keep hitting the output token limit. Increase “Max output tokens” in Settings.".into() });
                        return "max_tokens".into();
                    }
                    continue;
                }
                "max_tokens" => {
                    sink.send(AgentEvent::Notice { message: "The response hit the output token limit (Settings → Max output tokens).".into() });
                    return stop;
                }
                "model_context_window_exceeded" => {
                    sink.send(AgentEvent::Notice { message: "The conversation filled the model's context window. Start a new chat to continue.".into() });
                    return stop;
                }
                _ => return stop,
            }

            let invalid: HashMap<String, String> = turn.invalid_inputs.into_iter().map(|i| (i.id, i.raw)).collect();
            let results = self.execute_tools(tool_uses, &invalid, sink).await;
            self.append(sink, user_tool_results(results));
        }
    }

    /// If a previous run died between a tool call and its result (e.g. the app was closed),
    /// close the dangling calls so the history stays valid for the API.
    fn repair_dangling_tool_uses(&mut self, sink: &mut Sink) {
        let Some(last) = self.conv.messages.last() else { return };
        if last.role != "assistant" {
            return;
        }
        let ids: Vec<String> = last
            .content
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
            .filter_map(|b| b.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        if ids.is_empty() {
            return;
        }
        let results = ids.iter().map(|id| tool_result(id, "Interrupted: this tool call never ran.", true)).collect();
        self.append(sink, user_tool_results(results));
    }

    fn record_usage(&mut self, turn: &AssistantTurn, sink: &mut Sink) {
        let u = &turn.usage;
        let (input, output) = (u.input_tokens.unwrap_or(0), u.output_tokens.unwrap_or(0));
        let (cr, cw) = (u.cache_read_input_tokens.unwrap_or(0), u.cache_creation_input_tokens.unwrap_or(0));
        let t = &mut self.conv.usage;
        t.input_tokens += input;
        t.output_tokens += output;
        t.cache_read_tokens += cr;
        t.cache_write_tokens += cw;
        sink.send(AgentEvent::Usage { totals: t.clone(), context_tokens: input + cr + cw + output });
    }

    fn set_tool_ui(&mut self, sink: &mut Sink, id: &str, ui: ToolUi) {
        self.conv.tool_ui.insert(id.to_string(), ui.clone());
        sink.send(AgentEvent::ToolStatus { tool_id: id.to_string(), ui });
    }

    // -----------------------------------------------------------------------------------
    // Streaming
    // -----------------------------------------------------------------------------------

    async fn stream_with_retries(&self, body: &Value, betas: &[&str], sink: &mut Sink) -> std::result::Result<AssistantTurn, StreamFail> {
        let mut attempt = 0;
        loop {
            match self.stream_once(body, betas, sink).await {
                Err(StreamFail::Api(e)) if e.retryable() && attempt < MAX_RETRIES => {
                    attempt += 1;
                    let backoff = Duration::from_millis(1000 * 2u64.pow(attempt - 1) + now_ms() % 400);
                    let delay = e.retry_after().unwrap_or(backoff).min(Duration::from_secs(60));
                    sink.send(AgentEvent::Reset);
                    sink.send(AgentEvent::Retrying { attempt, delay_ms: delay.as_millis() as u64, message: e.user_message() });
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = self.cancel.cancelled() => return Err(StreamFail::Cancelled(Vec::new())),
                    }
                }
                other => return other,
            }
        }
    }

    async fn stream_once(&self, body: &Value, betas: &[&str], sink: &mut Sink) -> std::result::Result<AssistantTurn, StreamFail> {
        let base = self.settings.api_base();
        let mut resp = tokio::select! {
            r = api::open_stream(&self.state.http, &base, &self.api_key, body, betas) => r.map_err(StreamFail::Api)?,
            _ = self.cancel.cancelled() => return Err(StreamFail::Cancelled(Vec::new())),
        };
        let mut parser = SseParser::default();
        let mut builder = MessageBuilder::default();
        let mut events = Vec::new();
        let mut tick = tokio::time::interval(Duration::from_millis(33));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            let chunk = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => {
                    sink.flush();
                    return Err(StreamFail::Cancelled(builder.partial_text_blocks()));
                }
                _ = tick.tick() => { sink.flush(); continue; }
                c = tokio::time::timeout(STREAM_IDLE_TIMEOUT, resp.chunk()) => c,
            };
            let bytes = match chunk {
                Err(_) => return Err(StreamFail::Api(ApiError::Network("the response stream stalled".into()))),
                Ok(Err(e)) => return Err(StreamFail::Api(ApiError::Network(api::describe_reqwest(&e)))),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(b))) => b,
            };
            parser.feed(&bytes, &mut events);
            for ev in events.drain(..) {
                if ev.data.is_empty() {
                    continue;
                }
                let Ok(parsed) = serde_json::from_str::<StreamEvent>(&ev.data) else { continue };
                match parsed {
                    StreamEvent::MessageStart { message } => {
                        sink.send(AgentEvent::RequestStarted { model: message.model.clone() });
                        builder.on_message_start(message);
                    }
                    StreamEvent::ContentBlockStart { index, content_block } => {
                        let tool_id = content_block.get("id").and_then(Value::as_str).map(String::from);
                        let tool_name = content_block.get("name").and_then(Value::as_str).map(String::from);
                        let kind = builder.on_block_start(index, content_block);
                        sink.send(AgentEvent::BlockStart { index, kind: kind.as_str(), tool_id, tool_name });
                    }
                    StreamEvent::ContentBlockDelta { index, delta } => {
                        builder.on_delta(index, &delta);
                        match &delta {
                            sse::Delta::TextDelta { text } => sink.delta(index, text),
                            sse::Delta::ThinkingDelta { thinking } => sink.delta(index, thinking),
                            sse::Delta::InputJsonDelta { .. } => sink.progress(index, builder.json_len(index)),
                            _ => {}
                        }
                    }
                    StreamEvent::ContentBlockStop { index } => {
                        builder.on_block_stop(index);
                        if let Some((tool_id, name, input)) = builder.tool_input(index) {
                            let summary = tools::describe(&name, &input);
                            sink.send(AgentEvent::ToolInput { index, tool_id, name, input, summary });
                        }
                    }
                    StreamEvent::MessageDelta { delta, usage } => builder.on_message_delta(delta, usage),
                    StreamEvent::MessageStop => builder.finished = true,
                    StreamEvent::Error { error } => {
                        return Err(StreamFail::Api(ApiError::Stream { kind: error.kind, message: error.message }));
                    }
                    StreamEvent::Ping | StreamEvent::Unknown => {}
                }
            }
            if builder.finished {
                break;
            }
        }
        sink.flush();
        if !builder.finished && builder.stop_reason.is_none() {
            return Err(StreamFail::Api(ApiError::Network("the connection closed before the response finished".into())));
        }
        Ok(builder.finish())
    }

    // -----------------------------------------------------------------------------------
    // Tools
    // -----------------------------------------------------------------------------------

    async fn execute_tools(&mut self, calls: Vec<(String, String, Value)>, invalid: &HashMap<String, String>, sink: &mut Sink) -> Vec<Value> {
        let mut results = Vec::with_capacity(calls.len());
        for (id, name, input) in calls {
            let summary = tools::describe(&name, &input);
            if self.cancel.is_cancelled() {
                results.push(tool_result(&id, "Cancelled by the user before this ran.", true));
                self.set_tool_ui(sink, &id, ToolUi { status: ToolStatus::Cancelled, summary, detail: None, detail_kind: None, path: None });
                continue;
            }
            if let Some(raw) = invalid.get(&id) {
                // Streamed tool input that wasn't valid JSON: report it back so Claude can retry.
                let content = json!({ "INVALID_JSON": raw }).to_string();
                results.push(tool_result(&id, &content, true));
                self.set_tool_ui(sink, &id, ToolUi { status: ToolStatus::Error, summary, detail: Some("The tool input was not valid JSON; asked Claude to retry.".into()), detail_kind: Some("text".into()), path: None });
                continue;
            }
            self.set_tool_ui(sink, &id, ToolUi { status: ToolStatus::Running, summary: summary.clone(), detail: None, detail_kind: None, path: None });
            let res = self.run_tool(&id, &name, &input, sink).await;
            results.push(tool_result(&id, &res.content, res.is_error));
            self.set_tool_ui(sink, &id, res.ui);
        }
        results
    }

    async fn run_tool(&mut self, id: &str, name: &str, input: &Value, sink: &mut Sink) -> tools::ToolResult {
        let summary = tools::describe(name, input);
        let mode = self.state.settings.permission_mode();
        let grants = self.state.agent.grants(&self.conv.meta.id);
        // Tools that don't touch the workspace.
        match tools::class_of(name) {
            Some(tools::ToolClass::Meta) => {
                return match toolbox::parse_todos(input) {
                    Ok(todos) => toolbox::todo_result(&todos),
                    Err(e) => tools::ToolResult::error(e, summary),
                };
            }
            Some(tools::ToolClass::Network) => {
                let (url, max) = match toolbox::parse_fetch(input) {
                    Ok(v) => v,
                    Err(e) => return tools::ToolResult::error(e, summary),
                };
                if mode != PermissionMode::Bypass && !grants.network {
                    match self.ask(sink, id, "network", &summary, url.as_str()).await {
                        None => return cancelled_result(summary),
                        Some((Decision::Deny, feedback)) => return denied_result("request", summary, feedback, None),
                        Some((Decision::AllowSession, _)) => self.grant(|g| g.network = true),
                        Some((Decision::Allow, _)) => {}
                    }
                }
                self.set_tool_ui(sink, id, ToolUi { status: ToolStatus::Running, summary: summary.clone(), detail: None, detail_kind: None, path: None });
                return tokio::select! {
                    r = toolbox::web_fetch(&self.state.http, url, max) => r,
                    _ = self.cancel.cancelled() => cancelled_result(summary),
                };
            }
            _ => {}
        }
        let Some(ws) = self.ws.clone() else {
            return tools::ToolResult::error("No folder is open, so file and command tools are unavailable.", summary);
        };
        match tools::class_of(name) {
            None => tools::ToolResult::error(format!("Unknown tool: {name}"), summary),
            Some(tools::ToolClass::Read) => {
                let app = self.app.clone();
                let (name, input) = (name.to_string(), input.clone());
                tauri::async_runtime::spawn_blocking(move || tools::run_read(&name, &input, &ws, &app.state::<AppState>().index))
                    .await
                    .unwrap_or_else(|e| tools::ToolResult::error(format!("Tool crashed: {e}"), summary))
            }
            Some(tools::ToolClass::Meta | tools::ToolClass::Network) => unreachable!("handled above"),
            Some(_) if mode == PermissionMode::Plan => {
                tools::ToolResult::error("Plan mode is on: file edits and commands are disabled. Present your plan instead.", summary)
            }
            Some(tools::ToolClass::Edit) => {
                let (name_c, input_c, ws_c) = (name.to_string(), input.clone(), ws.clone());
                let prepared = match tauri::async_runtime::spawn_blocking(move || tools::prepare_edit(&name_c, &input_c, &ws_c)).await {
                    Ok(Ok(p)) => p,
                    Ok(Err(e)) => return tools::ToolResult::error(e, summary),
                    Err(e) => return tools::ToolResult::error(format!("Tool crashed: {e}"), summary),
                };
                let needs_approval = mode == PermissionMode::Ask && !grants.edits;
                if needs_approval {
                    match self.ask(sink, id, "edit", &prepared.summary, &prepared.diff).await {
                        None => return cancelled_result(prepared.summary),
                        Some((Decision::Deny, feedback)) => return denied_result("edit", prepared.summary, feedback, Some(prepared.diff)),
                        Some((Decision::AllowSession, _)) => self.grant(|g| g.edits = true),
                        Some((Decision::Allow, _)) => {}
                    }
                }
                tauri::async_runtime::spawn_blocking(move || tools::apply_edit(&prepared))
                    .await
                    .unwrap_or_else(|e| tools::ToolResult::error(format!("Tool crashed: {e}"), summary))
            }
            Some(tools::ToolClass::Command) => {
                let cmd = match tools::parse_command(input) {
                    Ok(c) => c,
                    Err(e) => return tools::ToolResult::error(e, summary),
                };
                let needs_approval = matches!(mode, PermissionMode::Ask | PermissionMode::AcceptEdits) && !grants.commands;
                if needs_approval {
                    match self.ask(sink, id, "command", &summary, &cmd.command).await {
                        None => return cancelled_result(summary),
                        Some((Decision::Deny, feedback)) => return denied_result("command", summary, feedback, None),
                        Some((Decision::AllowSession, _)) => self.grant(|g| g.commands = true),
                        Some((Decision::Allow, _)) => {}
                    }
                }
                self.set_tool_ui(sink, id, ToolUi { status: ToolStatus::Running, summary: summary.clone(), detail: None, detail_kind: None, path: None });
                // Live output: the command writes into a buffer that is flushed to the UI ~20×/s.
                let buf = Mutex::new(String::new());
                let fut = tools::run_command(&cmd.command, cmd.timeout_secs, &ws, &self.cancel, |s| buf.lock().unwrap().push_str(s));
                tokio::pin!(fut);
                let mut tick = tokio::time::interval(Duration::from_millis(50));
                let res = loop {
                    tokio::select! {
                        r = &mut fut => break r,
                        _ = tick.tick() => {
                            let text = std::mem::take(&mut *buf.lock().unwrap());
                            if !text.is_empty() {
                                sink.send(AgentEvent::ToolOutput { tool_id: id.to_string(), text });
                            }
                        }
                    }
                };
                let rest = std::mem::take(&mut *buf.lock().unwrap());
                if !rest.is_empty() {
                    sink.send(AgentEvent::ToolOutput { tool_id: id.to_string(), text: rest });
                }
                res
            }
        }
    }

    fn grant(&self, f: impl FnOnce(&mut Grants)) {
        let mut g = self.state.agent.grants.lock().unwrap();
        f(g.entry(self.conv.meta.id.clone()).or_default());
    }

    /// Ask the user to approve a tool call. `None` = the run was cancelled meanwhile.
    async fn ask(&self, sink: &mut Sink, tool_id: &str, kind: &'static str, title: &str, detail: &str) -> Option<(Decision, Option<String>)> {
        let approval_id = uuid::Uuid::new_v4().simple().to_string();
        let (tx, rx) = oneshot::channel();
        self.state.agent.approvals.lock().unwrap().insert(approval_id.clone(), tx);
        sink.send(AgentEvent::ApprovalRequest {
            approval_id: approval_id.clone(),
            tool_id: tool_id.to_string(),
            kind,
            title: title.to_string(),
            detail: truncate_end(detail, 200_000).to_string(),
        });
        let decision = tokio::select! {
            r = rx => r.ok(),
            _ = self.cancel.cancelled() => None,
        };
        self.state.agent.approvals.lock().unwrap().remove(&approval_id);
        sink.send(AgentEvent::ApprovalResolved { approval_id, tool_id: tool_id.to_string() });
        decision
    }
}

fn assistant_message(content: Vec<Value>, model: Option<String>) -> StoredMessage {
    StoredMessage { role: "assistant".into(), content, display: None, model, ts: now_ms() }
}

fn user_tool_results(results: Vec<Value>) -> StoredMessage {
    StoredMessage { role: "user".into(), content: results, display: None, model: None, ts: now_ms() }
}

fn tool_result(id: &str, content: &str, is_error: bool) -> Value {
    let mut v = json!({ "type": "tool_result", "tool_use_id": id, "content": content });
    if is_error {
        v["is_error"] = json!(true);
    }
    v
}

fn cancelled_result(summary: String) -> tools::ToolResult {
    let mut r = tools::ToolResult::error("Cancelled by the user.", summary);
    r.ui.status = ToolStatus::Cancelled;
    r
}

fn denied_result(kind: &str, summary: String, feedback: Option<String>, detail: Option<String>) -> tools::ToolResult {
    let feedback = feedback.map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
    let msg = match &feedback {
        Some(f) => format!("The user rejected this {kind}. Their feedback: {f}"),
        None => format!("The user rejected this {kind}. Don't retry it; ask what they want instead if unclear."),
    };
    let mut r = tools::ToolResult::error(msg, summary);
    r.ui.status = ToolStatus::Denied;
    if let Some(d) = detail {
        r.ui.detail = Some(d);
        r.ui.detail_kind = Some("diff".into());
    } else if let Some(f) = feedback {
        r.ui.detail = Some(format!("Feedback: {f}"));
    }
    r
}

/// The text sent to the API for a user message: attached files inlined before the request.
fn build_user_text(ws: Option<&Workspace>, text: &str, attachments: &[String]) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut attached = Vec::new();
    if let Some(ws) = ws {
        for a in attachments {
            let Ok(path) = ws.resolve(a) else { continue };
            let rel = ws.relative(&path);
            match std::fs::read(&path) {
                Ok(bytes) if looks_binary(&bytes) => {
                    out.push_str(&format!("<file path=\"{rel}\">(binary file, {} bytes, not shown)</file>\n\n", bytes.len()));
                }
                Ok(bytes) => {
                    let s = String::from_utf8_lossy(&bytes);
                    let body = truncate_end(&s, MAX_ATTACHMENT);
                    let note = if body.len() < s.len() { "\n[truncated]" } else { "" };
                    out.push_str(&format!("<file path=\"{rel}\">\n{body}{note}\n</file>\n\n"));
                }
                Err(e) => out.push_str(&format!("<file path=\"{rel}\">(could not read: {e})</file>\n\n")),
            }
            attached.push(rel);
        }
    }
    out.push_str(text);
    (out, attached)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_inlines_attachments() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(d.path().join("b.bin"), [0u8, 1]).unwrap();
        let ws = Workspace::open(d.path()).unwrap();
        let (t, att) = build_user_text(Some(&ws), "explain", &["a.rs".into(), "b.bin".into(), "../nope".into()]);
        assert!(t.starts_with("<file path=\"a.rs\">\nfn a() {}\n</file>"));
        assert!(t.contains("binary file"));
        assert!(t.ends_with("explain"));
        assert_eq!(att, vec!["a.rs", "b.bin"]);
        let (t, att) = build_user_text(None, "hi", &["a.rs".into()]);
        assert_eq!(t, "hi");
        assert!(att.is_empty());
    }

    #[test]
    fn tool_result_shape() {
        assert_eq!(tool_result("t1", "ok", false), json!({"type":"tool_result","tool_use_id":"t1","content":"ok"}));
        assert_eq!(tool_result("t1", "bad", true)["is_error"], true);
        let d = denied_result("edit", "Edit x".into(), Some("  use tabs ".into()), None);
        assert!(d.content.contains("Their feedback: use tabs"));
        assert_eq!(d.ui.status, ToolStatus::Denied);
    }
}
