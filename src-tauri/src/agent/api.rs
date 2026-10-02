//! Claude Messages API over raw HTTP (there is no official Rust SDK).
//!
//! Builds request bodies with the right per-model options (adaptive thinking, effort,
//! server-side refusal fallbacks, prompt caching) and opens the SSE stream.

use crate::settings::Settings;
use serde_json::{json, Value};
use std::time::Duration;

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// What a model accepts. Unknown `claude-*` models are assumed to be current-generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCaps {
    /// `thinking: {type: "adaptive"}` + `output_config.effort`
    pub adaptive: bool,
    /// Accepts `thinking.display` (default is "omitted" on 4.7+, so we ask for summaries).
    pub thinking_display: bool,
    /// Accepts effort "xhigh" (4.7+).
    pub xhigh: bool,
    /// Server-side refusal fallbacks (`fallbacks: "default"`), Claude API only.
    pub server_fallbacks: bool,
}

impl ModelCaps {
    pub fn of(model: &str) -> Self {
        let m = model.trim().to_ascii_lowercase();
        let legacy = !m.starts_with("claude-")
            || m.contains("haiku")
            || m.starts_with("claude-3")
            || ["-4-0", "-4-1", "-4-5", "-4-2025"].iter().any(|v| m.contains(v));
        if legacy {
            return Self { adaptive: false, thinking_display: false, xhigh: false, server_fallbacks: false };
        }
        let is_46 = m.contains("-4-6");
        let server_fallbacks = ["claude-fable-5-1", "claude-fable-5", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"]
            .iter()
            .any(|p| m == *p || m.starts_with(&format!("{p}-")));
        Self { adaptive: true, thinking_display: !is_46, xhigh: !is_46, server_fallbacks }
    }
}

pub struct RequestParts<'a> {
    pub settings: &'a Settings,
    pub system: &'a str,
    pub tools: Vec<Value>,
    pub messages: Vec<Value>,
}

/// Returns (body, beta headers).
pub fn build_request(p: RequestParts<'_>) -> (Value, Vec<&'static str>) {
    let s = p.settings;
    let caps = ModelCaps::of(&s.model);
    let mut betas = Vec::new();
    let mut body = json!({
        "model": s.model,
        "max_tokens": s.max_tokens,
        "stream": true,
        // Automatic prompt caching: caches the longest stable prefix (tools + system +
        // history), which is what makes long agent loops cheap and fast.
        "cache_control": { "type": "ephemeral" },
        "system": [{ "type": "text", "text": p.system }],
        "messages": p.messages,
    });
    if !p.tools.is_empty() {
        body["tools"] = Value::Array(p.tools);
    }
    if caps.adaptive {
        let mut thinking = json!({ "type": "adaptive" });
        if caps.thinking_display {
            thinking["display"] = json!(if s.show_thinking { "summarized" } else { "omitted" });
        }
        body["thinking"] = thinking;
        let mut effort = s.effort.as_str();
        if !matches!(effort, "low" | "medium" | "high" | "xhigh" | "max") {
            effort = "high";
        }
        if effort == "xhigh" && !caps.xhigh {
            effort = "high";
        }
        body["output_config"] = json!({ "effort": effort });
    }
    if caps.server_fallbacks && s.is_default_endpoint() {
        // On a policy decline the API transparently retries on Anthropic's recommended
        // fallback model instead of returning the refusal.
        body["fallbacks"] = json!("default");
        betas.push(FALLBACK_BETA);
    }
    (body, betas)
}

#[derive(Debug, Clone)]
pub enum ApiError {
    Http { status: u16, kind: String, message: String, retry_after: Option<Duration> },
    Network(String),
    /// An `error` event inside the stream (e.g. overloaded_error).
    Stream { kind: String, message: String },
}

impl ApiError {
    pub fn retryable(&self) -> bool {
        match self {
            ApiError::Http { status, .. } => matches!(status, 408 | 409 | 429 | 500..=599),
            ApiError::Network(_) => true,
            ApiError::Stream { kind, .. } => matches!(kind.as_str(), "overloaded_error" | "api_error" | "rate_limit_error"),
        }
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            ApiError::Http { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            ApiError::Http { status: 401, .. } => "Invalid API key. Check it in Settings (Ctrl+,).".into(),
            ApiError::Http { status: 403, message, .. } => format!("Permission denied by the API: {message}"),
            ApiError::Http { status: 404, message, .. } => format!("Not found: {message} (check the model name in Settings)"),
            ApiError::Http { status: 413, .. } => "The request is too large. Start a new chat or attach fewer files.".into(),
            ApiError::Http { status: 429, message, .. } => format!("Rate limited: {message}"),
            ApiError::Http { status: 529, .. } => "The API is overloaded. Try again in a moment.".into(),
            ApiError::Http { status, message, kind, .. } => {
                if message.is_empty() { format!("API error {status} ({kind})") } else { format!("API error {status}: {message}") }
            }
            ApiError::Network(m) => format!("Network error: {m}"),
            ApiError::Stream { message, kind } => {
                if message.is_empty() { format!("Stream error: {kind}") } else { format!("Stream error: {message}") }
            }
        }
    }
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("PiLunch/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .expect("HTTP client")
}

/// POST /v1/messages with `stream: true`. Returns the response once headers arrive.
pub async fn open_stream(
    http: &reqwest::Client,
    base: &str,
    api_key: &str,
    body: &Value,
    betas: &[&str],
) -> Result<reqwest::Response, ApiError> {
    let mut req = http
        .post(format!("{base}/v1/messages"))
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .body(serde_json::to_vec(body).unwrap_or_default());
    if !betas.is_empty() {
        req = req.header("anthropic-beta", betas.join(","));
    }
    let resp = req.send().await.map_err(|e| ApiError::Network(describe_reqwest(&e)))?;
    if resp.status().is_success() {
        return Ok(resp);
    }
    Err(error_from_response(resp).await)
}

async fn error_from_response(resp: reqwest::Response) -> ApiError {
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|s| Duration::from_secs_f64(s.clamp(0.0, 120.0)));
    let text = resp.text().await.unwrap_or_default();
    let (kind, message) = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            let e = v.get("error")?;
            Some((
                e.get("type").and_then(Value::as_str).unwrap_or("").to_string(),
                e.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
            ))
        })
        .unwrap_or_else(|| (String::new(), crate::util::truncate_end(text.trim(), 300).to_string()));
    ApiError::Http { status, kind, message, retry_after }
}

pub fn describe_reqwest(e: &reqwest::Error) -> String {
    use std::error::Error as _;
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    msg
}

/// GET /v1/models — validates the key and lists available models.
pub async fn list_models(http: &reqwest::Client, base: &str, api_key: &str) -> Result<Vec<(String, String)>, ApiError> {
    let resp = http
        .get(format!("{base}/v1/models?limit=100"))
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| ApiError::Network(describe_reqwest(&e)))?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp).await);
    }
    let v: Value = resp.json().await.map_err(|e| ApiError::Network(describe_reqwest(&e)))?;
    Ok(v.get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?.to_string();
                    let name = m.get("display_name").and_then(Value::as_str).unwrap_or(&id).to_string();
                    Some((id, name))
                })
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_by_model() {
        let opus = ModelCaps::of("claude-opus-5-5");
        assert!(opus.adaptive && opus.thinking_display && opus.xhigh && opus.server_fallbacks);
        assert!(ModelCaps::of("claude-sonnet-5-5").server_fallbacks);
        assert!(ModelCaps::of("claude-fable-5-1").server_fallbacks);
        assert!(!ModelCaps::of("claude-opus-4-8").server_fallbacks);
        assert!(ModelCaps::of("claude-opus-4-8").adaptive);
        let s46 = ModelCaps::of("claude-sonnet-4-6");
        assert!(s46.adaptive && !s46.thinking_display && !s46.xhigh);
        for legacy in ["claude-haiku-4-5", "claude-3-7-sonnet-latest", "claude-sonnet-4-5", "claude-opus-4-1", "claude-sonnet-4-20250514", "gpt-4o"] {
            assert!(!ModelCaps::of(legacy).adaptive, "{legacy}");
        }
        // a hypothetical future model is treated as current-generation
        assert!(ModelCaps::of("claude-opus-6").adaptive);
        assert!(!ModelCaps::of("claude-opus-6").server_fallbacks);
    }

    #[test]
    fn request_body_shapes() {
        let mut s = Settings::default();
        let (body, betas) = build_request(RequestParts { settings: &s, system: "sys", tools: vec![], messages: vec![json!({"role":"user","content":"hi"})] });
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["stream"], true);
        assert_eq!(body["thinking"], json!({"type":"adaptive","display":"summarized"}));
        assert_eq!(body["output_config"], json!({"effort":"high"}));
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["cache_control"], json!({"type":"ephemeral"}));
        assert!(body.get("tools").is_none());
        assert_eq!(betas, vec![FALLBACK_BETA]);

        // custom endpoint: no fallbacks beta
        s.base_url = "http://localhost:9999".into();
        let (body, betas) = build_request(RequestParts { settings: &s, system: "sys", tools: vec![], messages: vec![] });
        assert!(body.get("fallbacks").is_none());
        assert!(betas.is_empty());

        // Haiku: no thinking/effort params
        s.model = "claude-haiku-4-5".into();
        let (body, _) = build_request(RequestParts { settings: &s, system: "sys", tools: vec![json!({"name":"x"})], messages: vec![] });
        assert!(body.get("thinking").is_none());
        assert!(body.get("output_config").is_none());
        assert_eq!(body["tools"][0]["name"], "x");

        // xhigh downgraded where unsupported
        s.model = "claude-opus-4-6".into();
        s.effort = "xhigh".into();
        let (body, _) = build_request(RequestParts { settings: &s, system: "sys", tools: vec![], messages: vec![] });
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["thinking"], json!({"type":"adaptive"}));
    }

    #[test]
    fn retry_classification() {
        let h = |status| ApiError::Http { status, kind: String::new(), message: String::new(), retry_after: None };
        assert!(h(429).retryable());
        assert!(h(529).retryable());
        assert!(h(500).retryable());
        assert!(!h(400).retryable());
        assert!(!h(401).retryable());
        assert!(ApiError::Stream { kind: "overloaded_error".into(), message: String::new() }.retryable());
        assert!(!ApiError::Stream { kind: "invalid_request_error".into(), message: String::new() }.retryable());
        assert!(h(401).user_message().contains("API key"));
    }
}
