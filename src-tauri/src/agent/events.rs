//! Events streamed from an agent run to the UI, and a sink that batches high-frequency
//! events (token deltas, tool-input progress, command output) so the webview receives at
//! most ~30 updates per second instead of one IPC message per token.

use crate::conversations::{StoredMessage, ToolUi, UsageTotals};
use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AgentEvent {
    /// A message was appended to the conversation history (and saved).
    MessageAppended { message: StoredMessage },
    Title { title: String },
    /// An API request began streaming; `model` is the model actually serving it.
    RequestStarted { model: String },
    BlockStart { index: usize, kind: &'static str, tool_id: Option<String>, tool_name: Option<String> },
    /// Text or thinking appended to block `index`.
    Delta { index: usize, text: String },
    /// Bytes of tool input streamed so far (e.g. a large file being written).
    ToolInputProgress { index: usize, bytes: usize },
    ToolInput { index: usize, tool_id: String, name: String, input: Value, summary: String },
    ToolStatus { tool_id: String, ui: ToolUi },
    ToolOutput { tool_id: String, text: String },
    ApprovalRequest { approval_id: String, tool_id: String, kind: &'static str, title: String, detail: String },
    ApprovalResolved { approval_id: String, tool_id: String },
    Usage { totals: UsageTotals, context_tokens: u64 },
    Retrying { attempt: u32, delay_ms: u64, message: String },
    /// Discard the in-progress (streaming) assistant message.
    Reset,
    Notice { message: String },
    Refusal { message: String },
    Error { message: String },
    Done { stop_reason: String },
}

const FLUSH_EVERY: Duration = Duration::from_millis(33);

pub struct Sink {
    ch: Channel<AgentEvent>,
    delta: Option<(usize, String)>,
    progress: Option<(usize, usize)>,
    last_flush: Instant,
    pub closed: bool,
}

impl Sink {
    pub fn new(ch: Channel<AgentEvent>) -> Self {
        Self { ch, delta: None, progress: None, last_flush: Instant::now(), closed: false }
    }

    /// Send an event immediately (after flushing anything batched, to keep ordering).
    pub fn send(&mut self, ev: AgentEvent) {
        self.flush();
        self.raw(ev);
    }

    pub fn delta(&mut self, index: usize, text: &str) {
        match &mut self.delta {
            Some((i, buf)) if *i == index => buf.push_str(text),
            _ => {
                self.flush();
                self.delta = Some((index, text.to_string()));
            }
        }
        self.maybe_flush();
    }

    pub fn progress(&mut self, index: usize, bytes: usize) {
        if self.progress.is_some_and(|(i, _)| i != index) {
            self.flush();
        }
        self.progress = Some((index, bytes));
        self.maybe_flush();
    }

    fn maybe_flush(&mut self) {
        if self.last_flush.elapsed() >= FLUSH_EVERY {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        if let Some((index, text)) = self.delta.take() {
            self.raw(AgentEvent::Delta { index, text });
        }
        if let Some((index, bytes)) = self.progress.take() {
            self.raw(AgentEvent::ToolInputProgress { index, bytes });
        }
        self.last_flush = Instant::now();
    }

    fn raw(&mut self, ev: AgentEvent) {
        if self.ch.send(ev).is_err() {
            self.closed = true;
        }
    }
}
