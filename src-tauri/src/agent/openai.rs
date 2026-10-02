//! OpenAI-compatible providers: OpenRouter and local servers (Ollama, LM Studio, vLLM,
//! llama.cpp…). Requests are built from the same Messages-API history the rest of the app
//! uses, and streamed chunks are translated into Messages-API stream events, so the agent
//! loop, tools, approvals and UI work unchanged.

use super::sse::{Delta, MessageDeltaInfo, MessageStartInfo, StreamEvent, Usage};
use crate::settings::Settings;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub base: String,
    pub key: Option<String>,
    pub model: String,
    pub openrouter: bool,
}

impl Endpoint {
    pub fn for_settings(s: &Settings, openrouter_key: Option<String>, local_key: Option<String>) -> Option<Self> {
        match s.provider.as_str() {
            "openrouter" => Some(Endpoint { base: OPENROUTER_BASE.into(), key: openrouter_key, model: s.openrouter_model.clone(), openrouter: true }),
            "local" => Some(Endpoint {
                base: s.local_base_url.trim().trim_end_matches('/').to_string(),
                key: local_key,
                model: s.local_model.clone(),
                openrouter: false,
            }),
            _ => None,
        }
    }

    pub fn request(&self, http: &reqwest::Client, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = http.request(method, format!("{}{path}", self.base));
        if let Some(k) = &self.key {
            r = r.bearer_auth(k);
        }
        if self.openrouter {
            r = r.header("HTTP-Referer", "https://github.com/surelynotvain/pilunch").header("X-Title", "PiLunch");
        }
        r
    }
}

/// Messages-API history → chat-completions messages.
pub fn convert_messages(system: &str, messages: &[Value]) -> Vec<Value> {
    let mut out = vec![json!({ "role": "system", "content": system })];
    for m in messages {
        let role = m["role"].as_str().unwrap_or("user");
        let blocks: Vec<Value> = match &m["content"] {
            Value::String(s) => vec![json!({ "type": "text", "text": s })],
            Value::Array(a) => a.clone(),
            _ => vec![],
        };
        if role == "assistant" {
            let text: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
            let calls: Vec<Value> = blocks
                .iter()
                .filter(|b| b["type"] == "tool_use")
                .map(|b| {
                    json!({
                        "id": b["id"],
                        "type": "function",
                        "function": { "name": b["name"], "arguments": b["input"].to_string() }
                    })
                })
                .collect();
            if text.is_empty() && calls.is_empty() {
                continue;
            }
            let mut msg = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text.join("\n\n")) } });
            if !calls.is_empty() {
                msg["tool_calls"] = Value::Array(calls);
            }
            out.push(msg);
        } else {
            for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                let content = match &b["content"] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.push(json!({ "role": "tool", "tool_call_id": b["tool_use_id"], "content": content }));
            }
            let text: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
            if !text.is_empty() {
                out.push(json!({ "role": "user", "content": text.join("\n\n") }));
            }
        }
    }
    out
}

/// Client tool definitions → function tools (server tools are dropped).
pub fn convert_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter(|t| t.get("input_schema").is_some())
        .map(|t| json!({ "type": "function", "function": { "name": t["name"], "description": t["description"], "parameters": t["input_schema"] } }))
        .collect()
}

pub fn build_request(s: &Settings, ep: &Endpoint, system: &str, tools: &[Value], messages: &[Value]) -> Value {
    let mut body = json!({
        "model": ep.model,
        "stream": true,
        "stream_options": { "include_usage": true },
        "max_tokens": s.max_tokens,
        "messages": convert_messages(system, messages),
    });
    let fns = convert_tools(tools);
    if !fns.is_empty() {
        body["tools"] = Value::Array(fns);
        body["tool_choice"] = json!("auto");
    }
    let effort = match s.effort.as_str() {
        "low" => "low",
        "medium" => "medium",
        _ => "high",
    };
    if ep.openrouter {
        body["reasoning"] = json!({ "effort": effort });
    } else {
        body["reasoning_effort"] = json!(effort);
    }
    body
}

/// Translates chat-completion chunks into Messages-API stream events.
#[derive(Default)]
pub struct Translator {
    started: bool,
    next: usize,
    text: Option<usize>,
    thinking: Option<usize>,
    /// OpenAI tool-call index → our block index.
    tools: BTreeMap<u64, usize>,
    finish: Option<String>,
    usage: Usage,
    done: bool,
}

impl Translator {
    fn alloc(&mut self) -> usize {
        self.next += 1;
        self.next - 1
    }

    pub fn feed(&mut self, data: &str, out: &mut Vec<StreamEvent>) -> Result<(), String> {
        if data.trim() == "[DONE]" {
            self.finish_into(out);
            return Ok(());
        }
        let v: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        if let Some(err) = v.get("error") {
            return Err(err.get("message").and_then(Value::as_str).unwrap_or("provider error").to_string());
        }
        if !self.started {
            self.started = true;
            out.push(StreamEvent::MessageStart {
                message: MessageStartInfo { model: v["model"].as_str().unwrap_or_default().to_string(), usage: Usage::default() },
            });
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage.input_tokens = u["prompt_tokens"].as_u64();
            self.usage.output_tokens = u["completion_tokens"].as_u64();
            self.usage.cache_read_input_tokens = u["prompt_tokens_details"]["cached_tokens"].as_u64();
        }
        let Some(choice) = v["choices"].get(0) else { return Ok(()) };
        let d = &choice["delta"];
        let reasoning = d["reasoning"].as_str().or_else(|| d["reasoning_content"].as_str()).unwrap_or("");
        if !reasoning.is_empty() {
            let idx = match self.thinking {
                Some(i) => i,
                None => {
                    let i = self.alloc();
                    self.thinking = Some(i);
                    out.push(StreamEvent::ContentBlockStart { index: i, content_block: json!({ "type": "thinking", "thinking": "" }) });
                    i
                }
            };
            out.push(StreamEvent::ContentBlockDelta { index: idx, delta: Delta::ThinkingDelta { thinking: reasoning.to_string() } });
        }
        if let Some(text) = d["content"].as_str().filter(|t| !t.is_empty()) {
            let idx = match self.text {
                Some(i) => i,
                None => {
                    let i = self.alloc();
                    self.text = Some(i);
                    out.push(StreamEvent::ContentBlockStart { index: i, content_block: json!({ "type": "text", "text": "" }) });
                    i
                }
            };
            out.push(StreamEvent::ContentBlockDelta { index: idx, delta: Delta::TextDelta { text: text.to_string() } });
        }
        for tc in d["tool_calls"].as_array().into_iter().flatten() {
            let oa = tc["index"].as_u64().unwrap_or(self.tools.len() as u64);
            let idx = match self.tools.get(&oa) {
                Some(i) => *i,
                None => {
                    let i = self.alloc();
                    self.tools.insert(oa, i);
                    let id = tc["id"]
                        .as_str()
                        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                        .map(String::from)
                        .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()));
                    let name = tc["function"]["name"].as_str().unwrap_or_default();
                    out.push(StreamEvent::ContentBlockStart { index: i, content_block: json!({ "type": "tool_use", "id": id, "name": name, "input": {} }) });
                    i
                }
            };
            if let Some(args) = tc["function"]["arguments"].as_str().filter(|a| !a.is_empty()) {
                out.push(StreamEvent::ContentBlockDelta { index: idx, delta: Delta::InputJsonDelta { partial_json: args.to_string() } });
            }
        }
        if let Some(f) = choice["finish_reason"].as_str() {
            self.finish = Some(f.to_string());
        }
        Ok(())
    }

    /// Close all blocks and the message (on `[DONE]` or end of stream).
    pub fn finish_into(&mut self, out: &mut Vec<StreamEvent>) {
        if self.done {
            return;
        }
        self.done = true;
        for i in 0..self.next {
            out.push(StreamEvent::ContentBlockStop { index: i });
        }
        let stop = match self.finish.as_deref() {
            Some("tool_calls") | Some("function_call") => "tool_use",
            Some("length") => "max_tokens",
            _ if !self.tools.is_empty() => "tool_use",
            _ => "end_turn",
        };
        out.push(StreamEvent::MessageDelta { delta: MessageDeltaInfo { stop_reason: Some(stop.into()), stop_details: None }, usage: Some(self.usage) });
        out.push(StreamEvent::MessageStop);
    }

    pub fn finished(&self) -> bool {
        self.done
    }
}

pub fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from).or_else(|| v["message"].as_str().map(String::from)))
        .unwrap_or_else(|| crate::util::truncate_end(body.trim(), 300).to_string())
}

/// Model ids from GET /models.
pub fn parse_models(v: &Value) -> Vec<(String, String)> {
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_string();
            let name = m["name"].as_str().unwrap_or(&id).to_string();
            Some((id, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::sse::MessageBuilder;

    fn run(chunks: &[Value]) -> crate::agent::sse::AssistantTurn {
        let mut t = Translator::default();
        let mut b = MessageBuilder::default();
        let mut evs = Vec::new();
        for c in chunks {
            t.feed(&c.to_string(), &mut evs).unwrap();
        }
        t.feed("[DONE]", &mut evs).unwrap();
        for ev in evs {
            match ev {
                StreamEvent::MessageStart { message } => b.on_message_start(message),
                StreamEvent::ContentBlockStart { index, content_block } => {
                    b.on_block_start(index, content_block);
                }
                StreamEvent::ContentBlockDelta { index, delta } => b.on_delta(index, &delta),
                StreamEvent::ContentBlockStop { index } => b.on_block_stop(index),
                StreamEvent::MessageDelta { delta, usage } => b.on_message_delta(delta, usage),
                StreamEvent::MessageStop => b.finished = true,
                _ => {}
            }
        }
        b.finish()
    }

    #[test]
    fn translates_text_reasoning_and_tool_calls() {
        let turn = run(&[
            json!({"model":"qwen3","choices":[{"delta":{"reasoning_content":"think "}}]}),
            json!({"choices":[{"delta":{"content":"Reading"}}]}),
            json!({"choices":[{"delta":{"content":" it."}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"pa"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.rs\"}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}),
        ]);
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(turn.model, "qwen3");
        assert_eq!(turn.usage.input_tokens, Some(10));
        assert_eq!(turn.content[0], json!({"type":"thinking","thinking":"think "}));
        assert_eq!(turn.content[1], json!({"type":"text","text":"Reading it."}));
        assert_eq!(turn.content[2], json!({"type":"tool_use","id":"call_1","name":"read_file","input":{"path":"a.rs"}}));
    }

    #[test]
    fn missing_tool_ids_are_generated_and_length_maps_to_max_tokens() {
        let turn = run(&[json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"glob","arguments":"{}"}}]}}]})]);
        assert!(turn.content[0]["id"].as_str().unwrap().starts_with("call_"));
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_use"));
        let turn = run(&[json!({"choices":[{"delta":{"content":"cut"},"finish_reason":"length"}]})]);
        assert_eq!(turn.stop_reason.as_deref(), Some("max_tokens"));
        let mut t = Translator::default();
        assert!(t.feed(r#"{"error":{"message":"No such model"}}"#, &mut Vec::new()).unwrap_err().contains("No such model"));
    }

    #[test]
    fn converts_history_and_tools() {
        let msgs = vec![
            json!({"role":"user","content":[{"type":"text","text":"hi"}]}),
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"x","signature":"s"},{"type":"text","text":"ok"},{"type":"tool_use","id":"t1","name":"glob","input":{"pattern":"*.rs"}}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"a.rs"}]}),
        ];
        let out = convert_messages("SYS", &msgs);
        assert_eq!(out[0], json!({"role":"system","content":"SYS"}));
        assert_eq!(out[1], json!({"role":"user","content":"hi"}));
        assert_eq!(out[2]["content"], "ok");
        assert_eq!(out[2]["tool_calls"][0]["function"]["arguments"], "{\"pattern\":\"*.rs\"}");
        assert_eq!(out[3], json!({"role":"tool","tool_call_id":"t1","content":"a.rs"}));
        let tools = convert_tools(&[json!({"name":"glob","description":"d","input_schema":{"type":"object"}}), json!({"type":"web_search_20260209","name":"web_search"})]);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"], "glob");
    }
}
