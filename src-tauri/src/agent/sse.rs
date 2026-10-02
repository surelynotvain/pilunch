//! Server-sent-events parsing and Messages-API stream accumulation.
//!
//! `SseParser` turns arbitrary byte chunks (split anywhere, even inside a UTF-8 sequence)
//! into SSE events. `MessageBuilder` folds the Messages-API stream events into the final
//! content blocks — preserving thinking blocks and their signatures byte-for-byte so they
//! can be replayed to the API unchanged.

use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: String,
    has_data: bool,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8], out: &mut Vec<SseEvent>) {
        self.buf.extend_from_slice(chunk);
        let mut start = 0;
        while let Some(pos) = memchr::memchr(b'\n', &self.buf[start..]) {
            let end = start + pos;
            let mut line = &self.buf[start..end];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            let line = String::from_utf8_lossy(line).into_owned();
            self.line(&line, out);
            start = end + 1;
        }
        if start > 0 {
            self.buf.drain(..start);
        }
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if self.has_data || self.event.is_some() {
                out.push(SseEvent { event: self.event.take(), data: std::mem::take(&mut self.data) });
                self.has_data = false;
            }
            return;
        }
        if line.starts_with(':') {
            return; // comment / keep-alive
        }
        let (field, value) = match line.find(':') {
            Some(i) => (&line[..i], line[i + 1..].strip_prefix(' ').unwrap_or(&line[i + 1..])),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------
// Messages API stream events
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, Debug, Default, Clone, Copy)]
#[serde(default)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

impl Usage {
    fn merge(&mut self, other: &Usage) {
        if other.input_tokens.is_some() {
            self.input_tokens = other.input_tokens;
        }
        if other.output_tokens.is_some() {
            self.output_tokens = other.output_tokens;
        }
        if other.cache_creation_input_tokens.is_some() {
            self.cache_creation_input_tokens = other.cache_creation_input_tokens;
        }
        if other.cache_read_input_tokens.is_some() {
            self.cache_read_input_tokens = other.cache_read_input_tokens;
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct MessageStartInfo {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct MessageDeltaInfo {
    pub stop_reason: Option<String>,
    pub stop_details: Option<Value>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ErrorBody {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub message: String,
}

/// Variant names mirror the API's delta types (`text_delta`, ...).
#[allow(clippy::enum_variant_names)]
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Delta {
    TextDelta { text: String },
    ThinkingDelta { thinking: String },
    SignatureDelta { signature: String },
    InputJsonDelta { partial_json: String },
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    MessageStart { message: MessageStartInfo },
    ContentBlockStart { index: usize, content_block: Value },
    ContentBlockDelta { index: usize, delta: Delta },
    ContentBlockStop { index: usize },
    MessageDelta {
        #[serde(default)]
        delta: MessageDeltaInfo,
        #[serde(default)]
        usage: Option<Usage>,
    },
    MessageStop,
    Ping,
    Error { error: ErrorBody },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Text,
    Thinking,
    ToolUse,
    /// Server-side tool call (e.g. web_search): input streams like tool_use, but the API runs it.
    ServerToolUse,
    Other,
}

impl BlockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BlockKind::Text => "text",
            BlockKind::Thinking => "thinking",
            BlockKind::ToolUse => "tool_use",
            BlockKind::ServerToolUse => "server_tool_use",
            BlockKind::Other => "other",
        }
    }
}

#[derive(Debug)]
struct BlockAcc {
    kind: BlockKind,
    base: Map<String, Value>,
    text: String,
    signature: String,
    json: String,
    done: bool,
}

/// A tool_use block whose streamed input was not valid JSON.
#[derive(Debug, Clone)]
pub struct InvalidToolInput {
    pub id: String,
    pub raw: String,
}

/// The completed assistant message.
#[derive(Debug, Clone)]
pub struct AssistantTurn {
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub stop_details: Option<Value>,
    pub model: String,
    pub usage: Usage,
    pub invalid_inputs: Vec<InvalidToolInput>,
}

#[derive(Default, Debug)]
pub struct MessageBuilder {
    blocks: Vec<Option<BlockAcc>>,
    pub model: String,
    pub usage: Usage,
    pub stop_reason: Option<String>,
    pub stop_details: Option<Value>,
    pub finished: bool,
}

impl MessageBuilder {
    pub fn on_message_start(&mut self, info: MessageStartInfo) {
        self.model = info.model;
        self.usage.merge(&info.usage);
    }

    /// Returns the kind of the new block.
    pub fn on_block_start(&mut self, index: usize, block: Value) -> BlockKind {
        let base = match block {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        let kind = match base.get("type").and_then(Value::as_str) {
            Some("text") => BlockKind::Text,
            Some("thinking") => BlockKind::Thinking,
            Some("tool_use") => BlockKind::ToolUse,
            Some("server_tool_use") => BlockKind::ServerToolUse,
            _ => BlockKind::Other,
        };
        let text = match kind {
            BlockKind::Text => base.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
            BlockKind::Thinking => base.get("thinking").and_then(Value::as_str).unwrap_or("").to_string(),
            _ => String::new(),
        };
        if self.blocks.len() <= index {
            self.blocks.resize_with(index + 1, || None);
        }
        self.blocks[index] = Some(BlockAcc { kind, base, text, signature: String::new(), json: String::new(), done: false });
        kind
    }

    pub fn on_delta(&mut self, index: usize, delta: &Delta) {
        let Some(Some(b)) = self.blocks.get_mut(index) else { return };
        match delta {
            Delta::TextDelta { text } => b.text.push_str(text),
            Delta::ThinkingDelta { thinking } => b.text.push_str(thinking),
            Delta::SignatureDelta { signature } => b.signature.push_str(signature),
            Delta::InputJsonDelta { partial_json } => b.json.push_str(partial_json),
            Delta::Other => {}
        }
    }

    pub fn on_block_stop(&mut self, index: usize) {
        if let Some(Some(b)) = self.blocks.get_mut(index) {
            b.done = true;
        }
    }

    pub fn on_message_delta(&mut self, delta: MessageDeltaInfo, usage: Option<Usage>) {
        if delta.stop_reason.is_some() {
            self.stop_reason = delta.stop_reason;
        }
        if delta.stop_details.is_some() {
            self.stop_details = delta.stop_details;
        }
        if let Some(u) = usage {
            self.usage.merge(&u);
        }
    }

    /// Size of the tool input streamed so far for a block (for progress display).
    pub fn json_len(&self, index: usize) -> usize {
        self.blocks.get(index).and_then(|b| b.as_ref()).map_or(0, |b| b.json.len())
    }

    /// The tool id/name/parsed input of a finished tool_use block.
    pub fn tool_input(&self, index: usize) -> Option<(String, String, Value)> {
        let b = self.blocks.get(index)?.as_ref()?;
        if b.kind != BlockKind::ToolUse {
            return None;
        }
        let id = b.base.get("id")?.as_str()?.to_string();
        let name = b.base.get("name")?.as_str()?.to_string();
        let input = parse_tool_json(&b.json).unwrap_or_else(|_| Value::Object(Map::new()));
        Some((id, name, input))
    }

    /// Text streamed so far (used to keep a partial answer when the user stops a response).
    pub fn partial_text_blocks(&self) -> Vec<Value> {
        self.blocks
            .iter()
            .flatten()
            .filter(|b| b.kind == BlockKind::Text && !b.text.trim().is_empty())
            .map(|b| serde_json::json!({ "type": "text", "text": b.text }))
            .collect()
    }

    pub fn finish(self) -> AssistantTurn {
        let mut invalid_inputs = Vec::new();
        let content = self
            .blocks
            .into_iter()
            .flatten()
            .map(|b| {
                let mut m = b.base;
                match b.kind {
                    BlockKind::Text => {
                        m.insert("text".into(), Value::String(b.text));
                    }
                    BlockKind::Thinking => {
                        m.insert("thinking".into(), Value::String(b.text));
                        if !b.signature.is_empty() {
                            m.insert("signature".into(), Value::String(b.signature));
                        }
                    }
                    BlockKind::ToolUse => {
                        let input = match parse_tool_json(&b.json) {
                            Ok(v) => v,
                            Err(()) => {
                                let id = m.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                                invalid_inputs.push(InvalidToolInput { id, raw: b.json.clone() });
                                Value::Object(Map::new())
                            }
                        };
                        m.insert("input".into(), input);
                    }
                    BlockKind::ServerToolUse => {
                        if let Ok(v) = parse_tool_json(&b.json) {
                            m.insert("input".into(), v);
                        }
                    }
                    BlockKind::Other => {}
                }
                Value::Object(m)
            })
            .collect();
        AssistantTurn {
            content: normalize_fallback(content),
            stop_reason: self.stop_reason,
            stop_details: self.stop_details,
            model: self.model,
            usage: self.usage,
            invalid_inputs,
        }
    }
}

/// Strict parse of a streamed tool input. Empty input means `{}`; it must be an object.
fn parse_tool_json(raw: &str) -> Result<Value, ()> {
    if raw.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(v @ Value::Object(_)) => Ok(v),
        _ => Err(()),
    }
}

/// After a mid-output server-side fallback, the declined model's thinking / tool_use (and
/// other model-internal) blocks that precede the last `fallback` block must not be echoed
/// back; text blocks and everything after the boundary are kept.
pub fn normalize_fallback(content: Vec<Value>) -> Vec<Value> {
    let Some(boundary) = content.iter().rposition(|b| b.get("type").and_then(Value::as_str) == Some("fallback")) else {
        return content;
    };
    content
        .into_iter()
        .enumerate()
        .filter(|(i, b)| {
            *i >= boundary || matches!(b.get("type").and_then(Value::as_str), Some("text") | Some("fallback"))
        })
        .map(|(_, b)| b)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut p = SseParser::default();
        let mut out = Vec::new();
        for c in chunks {
            p.feed(c, &mut out);
        }
        out
    }

    #[test]
    fn sse_handles_arbitrary_chunk_boundaries() {
        let stream = "event: message_start\ndata: {\"a\":1}\n\n: keepalive\n\nevent: ping\r\ndata: {}\r\n\r\ndata: line1\ndata: line2\n\n".as_bytes();
        let whole = parse_all(&[stream]);
        assert_eq!(
            whole,
            vec![
                SseEvent { event: Some("message_start".into()), data: "{\"a\":1}".into() },
                SseEvent { event: Some("ping".into()), data: "{}".into() },
                SseEvent { event: None, data: "line1\nline2".into() },
            ]
        );
        // byte-by-byte feeding yields the same events
        let bytes: Vec<&[u8]> = stream.chunks(1).collect();
        assert_eq!(parse_all(&bytes), whole);
    }

    #[test]
    fn sse_splits_inside_utf8() {
        let s = "data: {\"t\":\"héllo 😀\"}\n\n".as_bytes();
        for split in 1..s.len() {
            let ev = parse_all(&[&s[..split], &s[split..]]);
            assert_eq!(ev.len(), 1);
            assert_eq!(ev[0].data, "{\"t\":\"héllo 😀\"}");
        }
    }

    fn feed(b: &mut MessageBuilder, ev: Value) {
        match serde_json::from_value::<StreamEvent>(ev).unwrap() {
            StreamEvent::MessageStart { message } => b.on_message_start(message),
            StreamEvent::ContentBlockStart { index, content_block } => {
                b.on_block_start(index, content_block);
            }
            StreamEvent::ContentBlockDelta { index, delta } => b.on_delta(index, &delta),
            StreamEvent::ContentBlockStop { index } => b.on_block_stop(index),
            StreamEvent::MessageDelta { delta, usage } => b.on_message_delta(delta, usage),
            _ => {}
        }
    }

    #[test]
    fn builds_thinking_text_and_tool_use() {
        let mut b = MessageBuilder::default();
        for ev in [
            json!({"type":"message_start","message":{"id":"m","model":"claude-opus-5-5","usage":{"input_tokens":10,"cache_read_input_tokens":5}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me "}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"look."}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig123"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Reading it."}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_file","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"src/"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"main.rs\"}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}),
            json!({"type":"message_stop"}),
        ] {
            feed(&mut b, ev);
        }
        assert_eq!(b.tool_input(2).unwrap().2, json!({"path":"src/main.rs"}));
        let turn = b.finish();
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(turn.model, "claude-opus-5-5");
        assert_eq!(turn.usage.input_tokens, Some(10));
        assert_eq!(turn.usage.output_tokens, Some(42));
        assert_eq!(turn.usage.cache_read_input_tokens, Some(5));
        assert_eq!(
            turn.content,
            vec![
                json!({"type":"thinking","thinking":"Let me look.","signature":"sig123"}),
                json!({"type":"text","text":"Reading it."}),
                json!({"type":"tool_use","id":"toolu_1","name":"read_file","input":{"path":"src/main.rs"}}),
            ]
        );
        assert!(turn.invalid_inputs.is_empty());
    }

    #[test]
    fn server_tool_use_input_is_accumulated() {
        let mut b = MessageBuilder::default();
        feed(&mut b, json!({"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srv_1","name":"web_search","input":{}}}));
        feed(&mut b, json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\": \"tauri 2\"}"}}));
        feed(&mut b, json!({"type":"content_block_start","index":1,"content_block":{"type":"web_search_tool_result","tool_use_id":"srv_1","content":[{"type":"web_search_result","url":"https://v2.tauri.app","title":"Tauri"}]}}));
        assert!(b.tool_input(0).is_none(), "server tools are not executed locally");
        let turn = b.finish();
        assert_eq!(turn.content[0]["input"], json!({"query":"tauri 2"}));
        assert_eq!(turn.content[1]["content"][0]["url"], "https://v2.tauri.app");
    }

    #[test]
    fn invalid_tool_json_is_reported_not_guessed() {
        let mut b = MessageBuilder::default();
        feed(&mut b, json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"write_file","input":{}}}));
        feed(&mut b, json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"a\", \"content\": \"unterminated"}}));
        let turn = b.finish();
        assert_eq!(turn.content[0]["input"], json!({}));
        assert_eq!(turn.invalid_inputs.len(), 1);
        assert_eq!(turn.invalid_inputs[0].id, "t1");
        // empty input is a valid empty object
        let mut b = MessageBuilder::default();
        feed(&mut b, json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t2","name":"list_dir","input":{}}}));
        let turn = b.finish();
        assert!(turn.invalid_inputs.is_empty());
    }

    #[test]
    fn unknown_events_and_blocks_pass_through() {
        assert!(matches!(serde_json::from_value::<StreamEvent>(json!({"type":"brand_new_event","x":1})).unwrap(), StreamEvent::Unknown));
        let mut b = MessageBuilder::default();
        feed(&mut b, json!({"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"opaque"}}));
        feed(&mut b, json!({"type":"content_block_delta","index":0,"delta":{"type":"citations_delta","citation":{}}}));
        let turn = b.finish();
        assert_eq!(turn.content, vec![json!({"type":"redacted_thinking","data":"opaque"})]);
        let err: StreamEvent = serde_json::from_value(json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}})).unwrap();
        assert!(matches!(err, StreamEvent::Error { error } if error.kind == "overloaded_error"));
    }

    #[test]
    fn fallback_boundary_strips_declined_internal_blocks() {
        let content = vec![
            json!({"type":"thinking","thinking":"","signature":"s"}),
            json!({"type":"text","text":"partial"}),
            json!({"type":"tool_use","id":"t0","name":"read_file","input":{}}),
            json!({"type":"fallback","from":{"model":"a"},"to":{"model":"b"}}),
            json!({"type":"thinking","thinking":"","signature":"s2"}),
            json!({"type":"tool_use","id":"t1","name":"read_file","input":{}}),
        ];
        let out = normalize_fallback(content);
        let types: Vec<_> = out.iter().map(|b| b["type"].as_str().unwrap()).collect();
        assert_eq!(types, vec!["text", "fallback", "thinking", "tool_use"]);
        assert_eq!(out[3]["id"], "t1");
        // no fallback block: unchanged
        let plain = vec![json!({"type":"thinking","thinking":"x","signature":"s"})];
        assert_eq!(normalize_fallback(plain.clone()), plain);
    }
}
