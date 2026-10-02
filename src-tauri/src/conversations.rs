//! Chat history persistence (`~/.local/share/pilunch/conversations/`).
//!
//! Each conversation is one JSON file holding the exact Messages-API history (so it can be
//! replayed to the API unchanged — thinking blocks included) plus small UI-only metadata.
//! `index.json` caches the metadata of every conversation so listing is a single small read.

use crate::error::{Error, Result};
use crate::util::{atomic_write, now_ms};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMeta {
    pub id: String,
    pub title: String,
    pub workspace: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub message_count: usize,
}

/// One Messages-API message plus UI extras. `role` and `content` are sent to the API verbatim.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StoredMessage {
    pub role: String,
    pub content: Vec<Value>,
    /// For user messages: the text as typed, without inlined attachment contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<UserDisplay>,
    /// For assistant messages: the model that produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub ts: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserDisplay {
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Running,
    Done,
    Error,
    Denied,
    Cancelled,
}

/// How a tool call is rendered in the chat (keyed by tool_use id).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolUi {
    pub status: ToolStatus,
    pub summary: String,
    /// Diff, command output or other detail text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// "diff" | "output" | "text"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    #[serde(flatten)]
    pub meta: ConversationMeta,
    #[serde(default)]
    pub messages: Vec<StoredMessage>,
    #[serde(default)]
    pub tool_ui: HashMap<String, ToolUi>,
    #[serde(default)]
    pub usage: UsageTotals,
}

impl Conversation {
    /// Messages in the shape the Messages API expects.
    pub fn api_messages(&self) -> Vec<Value> {
        self.messages
            .iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
            .collect()
    }
}

pub struct ConversationStore {
    dir: PathBuf,
    index: Mutex<Vec<ConversationMeta>>,
}

impl ConversationStore {
    pub fn load(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let index = std::fs::read(dir.join("index.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<ConversationMeta>>(&b).ok())
            .unwrap_or_else(|| Self::rebuild_index(&dir));
        Self { dir, index: Mutex::new(index) }
    }

    fn rebuild_index(dir: &PathBuf) -> Vec<ConversationMeta> {
        let mut metas: Vec<ConversationMeta> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json") && e.file_name() != "index.json")
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice::<Conversation>(&b).ok())
            .map(|c| c.meta)
            .collect();
        metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at));
        metas
    }

    fn path_for(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(Error::msg("invalid conversation id"));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    pub fn list(&self) -> Vec<ConversationMeta> {
        let mut v = self.index.lock().unwrap().clone();
        v.sort_by_key(|m| std::cmp::Reverse(m.updated_at));
        v
    }

    pub fn create(&self, workspace: Option<String>) -> Result<Conversation> {
        let now = now_ms();
        let conv = Conversation {
            meta: ConversationMeta {
                id: uuid::Uuid::new_v4().simple().to_string(),
                title: "New chat".into(),
                workspace,
                created_at: now,
                updated_at: now,
                message_count: 0,
            },
            messages: Vec::new(),
            tool_ui: HashMap::new(),
            usage: UsageTotals::default(),
        };
        self.save(&conv)?;
        Ok(conv)
    }

    pub fn get(&self, id: &str) -> Result<Conversation> {
        let bytes = std::fs::read(self.path_for(id)?).map_err(|_| Error::msg("conversation not found"))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save(&self, conv: &Conversation) -> Result<()> {
        let mut meta = conv.meta.clone();
        meta.message_count = conv.messages.len();
        let mut full = conv.clone();
        full.meta = meta.clone();
        atomic_write(&self.path_for(&conv.meta.id)?, &serde_json::to_vec(&full)?)?;
        let mut idx = self.index.lock().unwrap();
        match idx.iter_mut().find(|m| m.id == meta.id) {
            Some(m) => *m = meta,
            None => idx.push(meta),
        }
        self.write_index(&idx)
    }

    pub fn rename(&self, id: &str, title: &str) -> Result<ConversationMeta> {
        let mut conv = self.get(id)?;
        conv.meta.title = title.trim().chars().take(120).collect();
        if conv.meta.title.is_empty() {
            conv.meta.title = "Untitled".into();
        }
        self.save(&conv)?;
        Ok(conv.meta)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.path_for(id)?;
        let _ = std::fs::remove_file(path);
        let mut idx = self.index.lock().unwrap();
        idx.retain(|m| m.id != id);
        self.write_index(&idx)
    }

    fn write_index(&self, idx: &[ConversationMeta]) -> Result<()> {
        atomic_write(&self.dir.join("index.json"), &serde_json::to_vec(idx)?)?;
        Ok(())
    }
}

/// A short title from the first user message.
pub fn title_from(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New chat");
    let mut t: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        t.push('…');
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_save_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConversationStore::load(dir.path().to_path_buf());
        let mut c = store.create(Some("/tmp/proj".into())).unwrap();
        c.messages.push(StoredMessage {
            role: "user".into(),
            content: vec![serde_json::json!({"type": "text", "text": "hi"})],
            display: None,
            model: None,
            ts: 1,
        });
        c.meta.updated_at += 1;
        store.save(&c).unwrap();
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].message_count, 1);
        assert_eq!(store.get(&c.meta.id).unwrap().api_messages()[0]["role"], "user");
        // index is rebuilt from files if it goes missing
        std::fs::remove_file(dir.path().join("index.json")).unwrap();
        let store2 = ConversationStore::load(dir.path().to_path_buf());
        assert_eq!(store2.list().len(), 1);
        store2.delete(&c.meta.id).unwrap();
        assert!(store2.list().is_empty());
        assert!(store2.get("../../etc/passwd").is_err());
    }

    #[test]
    fn titles() {
        assert_eq!(title_from("\n  Fix the bug  \nmore"), "Fix the bug");
        assert!(title_from(&"a".repeat(100)).ends_with('…'));
    }
}
