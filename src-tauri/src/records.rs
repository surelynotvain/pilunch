//! Local records of what the agent did: token usage (`usage.jsonl`) and training traces
//! (`traces/<conversation>.json`, exportable as one JSONL dataset).
//!
//! A trace is the whole conversation in chat-completions format — system prompt, tool
//! definitions, user turns, assistant turns with their `reasoning_content` and
//! `tool_calls`, and tool results — plus metadata (provider, model, thinking level). It is
//! rewritten after every run, so each file always holds the latest full trajectory.

use crate::conversations::Conversation;
use crate::error::Result;
use crate::util::{atomic_write, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Records {
    dir: PathBuf,
    usage_lock: Mutex<()>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageEntry {
    pub ts: u64,
    pub provider: String,
    pub model: String,
    pub conversation: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub key: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl UsageRow {
    fn add(&mut self, e: &UsageEntry) {
        self.requests += 1;
        self.input_tokens += e.input_tokens;
        self.output_tokens += e.output_tokens;
        self.cache_read_tokens += e.cache_read_tokens;
        self.cache_write_tokens += e.cache_write_tokens;
    }
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub total: UsageRow,
    pub today: UsageRow,
    /// The last `days` days (UTC), oldest first, including empty days.
    pub by_day: Vec<UsageRow>,
    pub by_model: Vec<UsageRow>,
    pub by_provider: Vec<UsageRow>,
    pub conversations: usize,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TraceStats {
    pub dir: String,
    pub count: usize,
    pub bytes: u64,
}

/// What a trace records about the run besides the conversation itself.
pub struct TraceMeta<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub thinking_level: &'a str,
    pub effort: &'a str,
    pub system: &'a str,
    pub tools: &'a [Value],
    pub stop_reason: &'a str,
}

impl Records {
    pub fn new(data_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(data_dir.join("traces"));
        Self { dir: data_dir, usage_lock: Mutex::new(()) }
    }

    pub fn traces_dir(&self) -> PathBuf {
        self.dir.join("traces")
    }

    // ---------------------------------------------------------------------------------
    // Usage
    // ---------------------------------------------------------------------------------

    pub fn record_usage(&self, e: &UsageEntry) {
        let _g = self.usage_lock.lock().unwrap();
        let line = match serde_json::to_string(e) {
            Ok(l) => l,
            Err(_) => return,
        };
        let res = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("usage.jsonl")).and_then(|mut f| writeln!(f, "{line}"));
        if let Err(err) = res {
            eprintln!("failed to record usage: {err}");
        }
    }

    pub fn usage_entries(&self) -> Vec<UsageEntry> {
        let _g = self.usage_lock.lock().unwrap();
        std::fs::read_to_string(self.dir.join("usage.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    pub fn usage_summary(&self, days: u32) -> UsageSummary {
        summarize(&self.usage_entries(), now_ms(), days)
    }

    pub fn clear_usage(&self) -> Result<()> {
        let _g = self.usage_lock.lock().unwrap();
        match std::fs::remove_file(self.dir.join("usage.jsonl")) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    // ---------------------------------------------------------------------------------
    // Traces
    // ---------------------------------------------------------------------------------

    pub fn save_trace(&self, conv: &Conversation, meta: &TraceMeta) -> Result<()> {
        let trace = build_trace(conv, meta);
        let path = self.traces_dir().join(format!("{}.json", conv.meta.id));
        atomic_write(&path, &serde_json::to_vec(&trace)?)?;
        Ok(())
    }

    pub fn trace_stats(&self) -> TraceStats {
        let mut stats = TraceStats { dir: self.traces_dir().display().to_string(), ..Default::default() };
        for e in std::fs::read_dir(self.traces_dir()).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "json") {
                stats.count += 1;
                stats.bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        stats
    }

    /// Write every trace as one JSON object per line. `local_only` keeps local-model runs.
    pub fn export_traces(&self, out: &Path, local_only: bool) -> Result<usize> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(self.traces_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        let mut buf = Vec::new();
        let mut n = 0;
        for f in files {
            let Ok(v) = std::fs::read(&f).map_err(|_| ()).and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|_| ())) else { continue };
            if local_only && v["provider"] != "local" {
                continue;
            }
            serde_json::to_writer(&mut buf, &v)?;
            buf.push(b'\n');
            n += 1;
        }
        atomic_write(out, &buf)?;
        Ok(n)
    }
}

fn build_trace(conv: &Conversation, meta: &TraceMeta) -> Value {
    let mut raw: Vec<Value> = conv.messages.iter().map(|m| json!({ "role": m.role, "content": m.content })).collect();
    strip_images(&mut raw);
    let tools = crate::agent::openai::convert_tools(meta.tools);
    json!({
        "id": conv.meta.id,
        "title": conv.meta.title,
        "created_at": conv.meta.created_at,
        "updated_at": conv.meta.updated_at,
        "provider": meta.provider,
        "model": meta.model,
        "thinking_level": meta.thinking_level,
        "effort": meta.effort,
        "stop_reason": meta.stop_reason,
        "usage": conv.usage,
        "tools": tools,
        "messages": crate::agent::openai::convert_messages_with_reasoning(meta.system, &raw),
    })
}

/// Images make traces huge; keep a marker instead.
fn strip_images(messages: &mut [Value]) {
    fn walk(v: &mut Value) {
        if let Some(arr) = v.as_array_mut() {
            for b in arr {
                if b["type"] == "image" {
                    *b = json!({ "type": "text", "text": "[image]" });
                } else if b["type"] == "tool_result" {
                    walk(&mut b["content"]);
                }
            }
        }
    }
    for m in messages {
        walk(&mut m["content"]);
    }
}

const DAY_MS: u64 = 86_400_000;

/// `YYYY-MM-DD` (UTC) for a day number since the Unix epoch.
pub fn day_label(day: u64) -> String {
    // Howard Hinnant's civil-from-days algorithm.
    let z = day as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn summarize(entries: &[UsageEntry], now: u64, days: u32) -> UsageSummary {
    let today = now / DAY_MS;
    let first = today.saturating_sub(u64::from(days.max(1)) - 1);
    let mut by_day: Vec<UsageRow> = (first..=today).map(|d| UsageRow { key: day_label(d), ..Default::default() }).collect();
    let mut by_model: BTreeMap<String, UsageRow> = BTreeMap::new();
    let mut by_provider: BTreeMap<String, UsageRow> = BTreeMap::new();
    let mut s = UsageSummary { total: UsageRow { key: "total".into(), ..Default::default() }, today: UsageRow { key: day_label(today), ..Default::default() }, ..Default::default() };
    let mut convs = std::collections::HashSet::new();
    for e in entries {
        let day = e.ts / DAY_MS;
        s.total.add(e);
        if day == today {
            s.today.add(e);
        }
        if day >= first && day <= today {
            by_day[(day - first) as usize].add(e);
        }
        by_model.entry(e.model.clone()).or_insert_with(|| UsageRow { key: e.model.clone(), ..Default::default() }).add(e);
        by_provider.entry(e.provider.clone()).or_insert_with(|| UsageRow { key: e.provider.clone(), ..Default::default() }).add(e);
        convs.insert(e.conversation.as_str());
    }
    let sorted = |m: BTreeMap<String, UsageRow>| {
        let mut v: Vec<UsageRow> = m.into_values().collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.input_tokens + r.output_tokens));
        v
    };
    s.by_day = std::mem::take(&mut by_day);
    s.by_model = sorted(by_model);
    s.by_provider = sorted(by_provider);
    s.conversations = convs.len();
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::StoredMessage;

    #[test]
    fn day_labels() {
        assert_eq!(day_label(0), "1970-01-01");
        assert_eq!(day_label(19_723), "2024-01-01");
        assert_eq!(day_label(20_728), "2026-10-02");
    }

    #[test]
    fn usage_is_summarized_by_day_model_and_provider() {
        let d = tempfile::tempdir().unwrap();
        let r = Records::new(d.path().to_path_buf());
        let now = 20_728 * DAY_MS + 5000;
        let e = |ts: u64, model: &str, provider: &str, i: u64| UsageEntry { ts, provider: provider.into(), model: model.into(), conversation: "c1".into(), input_tokens: i, output_tokens: 10, ..Default::default() };
        r.record_usage(&e(now, "opus", "anthropic", 100));
        r.record_usage(&e(now - DAY_MS, "qwen", "local", 5));
        r.record_usage(&e(now - 40 * DAY_MS, "opus", "anthropic", 1));
        let s = summarize(&r.usage_entries(), now, 7);
        assert_eq!(s.total.requests, 3);
        assert_eq!(s.today.input_tokens, 100);
        assert_eq!(s.by_day.len(), 7);
        assert_eq!(s.by_day[6].key, "2026-10-02");
        assert_eq!(s.by_day[5].input_tokens, 5);
        assert_eq!(s.by_model[0].key, "opus");
        assert_eq!(s.by_model[0].input_tokens, 101);
        assert_eq!(s.by_provider.len(), 2);
        assert_eq!(s.conversations, 1);
        r.clear_usage().unwrap();
        assert!(r.usage_entries().is_empty());
    }

    #[test]
    fn traces_keep_reasoning_and_export_as_jsonl() {
        let d = tempfile::tempdir().unwrap();
        let r = Records::new(d.path().to_path_buf());
        let mut conv = Conversation::new(None);
        let msg = |role: &str, content: Value| StoredMessage { role: role.into(), content: content.as_array().unwrap().clone(), display: None, model: None, ts: 0 };
        conv.messages.push(msg("user", json!([{"type":"text","text":"fix it"}])));
        conv.messages.push(msg("assistant", json!([{"type":"thinking","thinking":"look first"},{"type":"tool_use","id":"t1","name":"read_file","input":{"path":"a"}}])));
        conv.messages.push(msg("user", json!([{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"shot"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]}])));
        conv.messages.push(msg("assistant", json!([{"type":"text","text":"done"}])));
        let tools = vec![json!({"name":"read_file","description":"d","input_schema":{"type":"object"}})];
        let meta = TraceMeta { provider: "local", model: "qwen3", thinking_level: "max", effort: "high", system: "SYS", tools: &tools, stop_reason: "end_turn" };
        r.save_trace(&conv, &meta).unwrap();
        assert_eq!(r.trace_stats().count, 1);
        let out = d.path().join("dataset.jsonl");
        assert_eq!(r.export_traces(&out, true).unwrap(), 1);
        let line: Value = serde_json::from_str(std::fs::read_to_string(&out).unwrap().lines().next().unwrap()).unwrap();
        assert_eq!(line["model"], "qwen3");
        assert_eq!(line["tools"][0]["function"]["name"], "read_file");
        let m = line["messages"].as_array().unwrap();
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[2]["reasoning_content"], "look first");
        assert_eq!(m[2]["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(m[3]["role"], "tool");
        assert!(!std::fs::read_to_string(&out).unwrap().contains("QUJD"), "images are stripped");
        assert_eq!(m.last().unwrap()["content"], "done");
        // the local-only export skips other providers
        let meta2 = TraceMeta { provider: "anthropic", ..meta };
        let mut c2 = Conversation::new(None);
        c2.messages.push(msg("user", json!([{"type":"text","text":"hi"}])));
        r.save_trace(&c2, &meta2).unwrap();
        assert_eq!(r.export_traces(&out, true).unwrap(), 1);
        assert_eq!(r.export_traces(&out, false).unwrap(), 2);
    }
}
