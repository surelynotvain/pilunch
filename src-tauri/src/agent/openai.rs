//! OpenAI-compatible providers: OpenAI, xAI (Grok), Google (Gemini), OpenRouter and local
//! servers (Ollama, LM Studio, vLLM, llama.cpp…). Requests are built from the same Messages-API history the rest of the app
//! uses, and streamed chunks are translated into Messages-API stream events, so the agent
//! loop, tools, approvals and UI work unchanged.

use super::sse::{Delta, MessageDeltaInfo, MessageStartInfo, StreamEvent, Usage};
use crate::settings::Settings;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";

/// Which OpenAI-compatible service an endpoint talks to (they differ in small ways).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    OpenAi,
    Xai,
    Google,
    OpenRouter,
    Local,
}

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub base: String,
    pub key: Option<String>,
    pub model: String,
    pub flavor: Flavor,
}

impl Endpoint {
    /// The endpoint for the selected provider; `None` for Anthropic (native API).
    pub fn for_settings(s: &Settings, store: &crate::settings::SettingsStore) -> Option<Self> {
        let (flavor, base, model) = match s.provider.as_str() {
            "openai" => (Flavor::OpenAi, "https://api.openai.com/v1".to_string(), &s.openai_model),
            "xai" => (Flavor::Xai, "https://api.x.ai/v1".to_string(), &s.xai_model),
            "google" => (Flavor::Google, "https://generativelanguage.googleapis.com/v1beta/openai".to_string(), &s.google_model),
            "openrouter" => (Flavor::OpenRouter, OPENROUTER_BASE.to_string(), &s.openrouter_model),
            "local" => (Flavor::Local, s.local_base_url.trim().trim_end_matches('/').to_string(), &s.local_model),
            _ => return None,
        };
        Some(Endpoint { base, key: store.provider_key(&s.provider), model: model.clone(), flavor })
    }

    pub fn request(&self, http: &reqwest::Client, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = http.request(method, format!("{}{path}", self.base));
        if let Some(k) = &self.key {
            r = r.bearer_auth(k);
        }
        if self.flavor == Flavor::OpenRouter {
            r = r.header("HTTP-Referer", "https://github.com/surelynotvain/pilunch").header("X-Title", "PiLunch");
        }
        r
    }
}

/// Messages-API history → chat-completions messages.
pub fn convert_messages(system: &str, messages: &[Value]) -> Vec<Value> {
    convert(system, messages, false)
}

/// Like `convert_messages`, but keeps each assistant turn's thinking as `reasoning_content`
/// (the format fine-tuning tools for reasoning models expect).
pub fn convert_messages_with_reasoning(system: &str, messages: &[Value]) -> Vec<Value> {
    convert(system, messages, true)
}

fn convert(system: &str, messages: &[Value], reasoning: bool) -> Vec<Value> {
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
            let thought: Vec<&str> = if reasoning { blocks.iter().filter(|b| b["type"] == "thinking").filter_map(|b| b["thinking"].as_str()).collect() } else { Vec::new() };
            if text.is_empty() && calls.is_empty() && thought.is_empty() {
                continue;
            }
            let mut msg = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text.join("\n\n")) } });
            if !thought.is_empty() {
                msg["reasoning_content"] = json!(thought.join("\n\n"));
            }
            if !calls.is_empty() {
                msg["tool_calls"] = Value::Array(calls);
            }
            out.push(msg);
        } else {
            // Tool messages can't hold images: they follow in a user message.
            let mut images = Vec::new();
            for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                let content = match &b["content"] {
                    Value::String(s) => s.clone(),
                    Value::Array(parts) => {
                        images.extend(parts.iter().filter(|p| p["type"] == "image").cloned());
                        parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n")
                    }
                    other => other.to_string(),
                };
                out.push(json!({ "role": "tool", "tool_call_id": b["tool_use_id"], "content": content }));
            }
            images.extend(blocks.iter().filter(|b| b["type"] == "image").cloned());
            let text: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
            if !images.is_empty() {
                let mut parts = vec![json!({ "type": "text", "text": if text.is_empty() { "Screenshot from the tool call above.".to_string() } else { text.join("\n\n") } })];
                for img in images {
                    let url = format!("data:{};base64,{}", img["source"]["media_type"].as_str().unwrap_or("image/png"), img["source"]["data"].as_str().unwrap_or_default());
                    parts.push(json!({ "type": "image_url", "image_url": { "url": url } }));
                }
                out.push(json!({ "role": "user", "content": parts }));
            } else if !text.is_empty() {
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
        "messages": convert_messages(system, messages),
    });
    // OpenAI's reasoning models only accept max_completion_tokens.
    let max_key = if ep.flavor == Flavor::OpenAi { "max_completion_tokens" } else { "max_tokens" };
    body[max_key] = json!(s.max_tokens);
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
    match ep.flavor {
        Flavor::OpenRouter => body["reasoning"] = json!({ "effort": effort }),
        Flavor::OpenAi | Flavor::Google => body["reasoning_effort"] = json!(effort),
        // Only Grok's mini models take reasoning_effort; the others reject it.
        Flavor::Xai if ep.model.contains("mini") => body["reasoning_effort"] = json!(effort),
        Flavor::Xai => {}
        Flavor::Local => super::thinking::apply(&mut body, super::thinking::level(&s.thinking_level)),
    }
    body
}

/// Drop the reasoning knobs from a request (for servers or models that reject them).
/// Returns false if there was nothing to drop.
pub fn strip_reasoning(body: &mut Value) -> bool {
    let Some(obj) = body.as_object_mut() else { return false };
    let mut any = false;
    for k in ["reasoning_effort", "reasoning", "think", "chat_template_kwargs"] {
        any |= obj.remove(k).is_some();
    }
    any
}

/// Does an error say a reasoning parameter is unsupported?
pub fn rejects_reasoning(status: u16, message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    status == 400 && (m.contains("reasoning") || m.contains("think") || m.contains("chat_template_kwargs")) && (m.contains("support") || m.contains("unrecognized") || m.contains("unknown") || m.contains("invalid") || m.contains("not allowed") || m.contains("extra"))
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
pub fn parse_models(v: &Value, flavor: Flavor) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            let id = id.strip_prefix("models/").unwrap_or(id).to_string();
            if !is_chat_model(&id, flavor) {
                return None;
            }
            let name = m["name"].as_str().or_else(|| m["display_name"].as_str()).unwrap_or(&id).to_string();
            Some((id, name))
        })
        .collect();
    if matches!(flavor, Flavor::OpenAi | Flavor::Xai | Flavor::Google) {
        out.sort_by(|a, b| b.0.cmp(&a.0));
    }
    out
}

/// Hide embedding, audio, image and moderation models from the big providers' lists.
fn is_chat_model(id: &str, flavor: Flavor) -> bool {
    if matches!(flavor, Flavor::OpenRouter | Flavor::Local) {
        return true;
    }
    let skip = ["embed", "tts", "whisper", "dall-e", "image", "moderation", "audio", "realtime", "transcribe", "search", "aqa", "imagen", "veo", "babbage", "davinci"];
    !skip.iter().any(|s| id.contains(s))
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
        let with_img = vec![json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"shot"},{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"AAA"}}]}]})];
        let out = convert_messages("S", &with_img);
        assert_eq!(out[1], json!({"role":"tool","tool_call_id":"t2","content":"shot"}));
        assert_eq!(out[2]["content"][1]["image_url"]["url"], "data:image/jpeg;base64,AAA");
        let tools = convert_tools(&[json!({"name":"glob","description":"d","input_schema":{"type":"object"}}), json!({"type":"web_search_20260209","name":"web_search"})]);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"], "glob");
    }
}
