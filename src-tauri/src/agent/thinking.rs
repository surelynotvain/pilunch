//! Thinking levels for local models: Off · Low · Normal · Medium · High · XHigh · Ultra · Max.
//!
//! The lower levels map to the request knobs local servers understand (`reasoning_effort`,
//! `chat_template_kwargs.enable_thinking`, Ollama's `think`, Qwen's `/no_think`). XHigh and
//! above also enforce a *minimum thinking budget*: if the model stops reasoning before the
//! budget is spent, its answer is discarded and it is asked to keep thinking from where it
//! left off (budget forcing, as in "s1: simple test-time scaling"). Max forces tens of
//! thousands of thinking tokens.

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    /// `reasoning_effort` to send; `None` = leave it to the server.
    pub effort: Option<&'static str>,
    /// `Some(false)` disables thinking, `Some(true)` asks for it.
    pub enable: Option<bool>,
    /// Minimum thinking tokens (estimated) before an answer is accepted.
    pub min_tokens: u32,
    /// Most extra rounds spent forcing more thinking.
    pub max_rounds: u32,
}


pub fn level(name: &str) -> Level {
    let l = |effort, enable, min_tokens, max_rounds| Level { effort, enable, min_tokens, max_rounds };
    match name {
        "off" => l(None, Some(false), 0, 0),
        "low" => l(Some("low"), Some(true), 0, 0),
        "medium" => l(Some("medium"), Some(true), 0, 0),
        "high" => l(Some("high"), Some(true), 0, 0),
        "xhigh" => l(Some("high"), Some(true), 2_000, 3),
        "ultra" => l(Some("high"), Some(true), 6_000, 6),
        "max" => l(Some("high"), Some(true), 16_000, 12),
        _ => l(None, None, 0, 0), // normal: the model's default
    }
}

/// Apply a level to a chat-completions request body.
pub fn apply(body: &mut Value, lv: Level) {
    if let Some(e) = lv.effort {
        body["reasoning_effort"] = json!(e);
    }
    if let Some(on) = lv.enable {
        body["chat_template_kwargs"] = json!({ "enable_thinking": on });
        body["think"] = json!(on);
        if !on {
            // Qwen-style soft switch, for servers that ignore the fields above.
            if let Some(sys) = body["messages"].get_mut(0).filter(|m| m["role"] == "system") {
                let text = format!("{}\n\n/no_think", sys["content"].as_str().unwrap_or_default());
                sys["content"] = json!(text);
            }
        }
    }
}

/// Rough token count of reasoning text (≈4 characters per token).
pub fn estimate_tokens(text: &str) -> u32 {
    (text.chars().count() as u32).div_ceil(4)
}

/// The user turn that asks the model to continue reasoning.
pub fn nudge(reasoning_so_far: &str, spent: u32, budget: u32) -> String {
    format!(
        "<reasoning_so_far>\n{reasoning_so_far}\n</reasoning_so_far>\n\n\
You stopped thinking too early ({spent} of at least {budget} thinking tokens). Wait — don't answer yet. \
Continue your reasoning from where it left off: re-check your assumptions, look for mistakes and edge cases, \
consider alternative approaches, and verify each step. Then give your final answer or tool calls."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_map_to_request_fields() {
        let mut b = json!({"messages":[{"role":"system","content":"S"}]});
        apply(&mut b, level("off"));
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(b["think"], false);
        assert!(b["messages"][0]["content"].as_str().unwrap().ends_with("/no_think"));
        assert!(b.get("reasoning_effort").is_none());

        let mut b = json!({"messages":[]});
        apply(&mut b, level("normal"));
        assert_eq!(b, json!({"messages":[]}));

        let mut b = json!({"messages":[]});
        apply(&mut b, level("max"));
        assert_eq!(b["reasoning_effort"], "high");
        assert!(level("max").min_tokens >= 10_000);
        assert!(level("xhigh").min_tokens < level("ultra").min_tokens);
        assert_eq!(level("high").min_tokens, 0);
    }

    #[test]
    fn token_estimate_and_nudge() {
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        assert_eq!(estimate_tokens("abc"), 1);
        let n = nudge("step 1", 10, 2000);
        assert!(n.contains("step 1") && n.contains("10 of at least 2000"));
    }
}
