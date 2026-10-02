//! User settings (`~/.config/pilunch/settings.json`) and the API key
//! (`~/.config/pilunch/secrets.json`, mode 0600). The API key never travels back to the
//! webview: the UI only learns whether one is configured and its last four characters.

use crate::error::{Error, Result};
use crate::util::{atomic_write, write_private};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Ask before every file edit and command.
    #[default]
    Ask,
    /// Apply file edits automatically, ask before commands.
    AcceptEdits,
    /// Read-only: the agent can explore and plan but not change anything.
    Plan,
    /// Never ask. Edits and commands run immediately.
    Bypass,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub model: String,
    /// low | medium | high | xhigh | max
    pub effort: String,
    pub max_tokens: u32,
    pub base_url: String,
    pub permission_mode: PermissionMode,
    pub show_thinking: bool,
    pub custom_instructions: String,
    /// dark | light | system
    pub theme: String,
    pub editor_font_size: u32,
    pub editor_word_wrap: bool,
    pub editor_minimap: bool,
    /// Empty = $SHELL (Linux/macOS) or PowerShell (Windows)
    pub terminal_shell: String,
    pub recent_workspaces: Vec<String>,
    /// Let Claude search the web (Anthropic server-side tool; billed per search).
    pub web_search: bool,
    /// The first-run setup has been completed.
    pub onboarded: bool,
    /// anthropic | openai | xai | google | openrouter | local
    pub provider: String,
    pub openrouter_model: String,
    pub openai_model: String,
    pub xai_model: String,
    pub google_model: String,
    /// OpenAI-compatible server for local models (Ollama, LM Studio, vLLM, llama.cpp…).
    pub local_base_url: String,
    pub local_model: String,
    /// Local models: off | low | normal | medium | high | xhigh | ultra | max.
    /// xhigh and above enforce a minimum thinking budget (see agent::thinking).
    pub thinking_level: String,
    /// Save every run (prompts, thinking, tool calls, results) as a training trace.
    pub save_traces: bool,
    /// "all" or "local": which providers' runs are traced.
    pub traces_scope: String,
    /// Let the agent see the screen and control mouse and keyboard (always asks first).
    pub computer_use: bool,
    /// Let the agent drive the built-in Firefox browser.
    pub browser_use: bool,
    /// Run Firefox without a window (the Browser panel shows its screen either way).
    pub browser_headless: bool,
    /// geckodriver executable; empty = find it on PATH.
    pub geckodriver_path: String,
    /// Firefox executable; empty = let geckodriver find it.
    pub firefox_path: String,
    /// Plugins (by folder name) that are installed but turned off.
    pub disabled_plugins: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.into(),
            effort: "high".into(),
            max_tokens: 32000,
            base_url: DEFAULT_BASE_URL.into(),
            permission_mode: PermissionMode::Ask,
            show_thinking: true,
            custom_instructions: String::new(),
            theme: "dark".into(),
            editor_font_size: 14,
            editor_word_wrap: false,
            editor_minimap: true,
            terminal_shell: String::new(),
            recent_workspaces: Vec::new(),
            web_search: false,
            onboarded: false,
            provider: "anthropic".into(),
            openrouter_model: "openrouter/auto".into(),
            local_base_url: "http://localhost:11434/v1".into(),
            local_model: String::new(),
            openai_model: "gpt-5".into(),
            xai_model: "grok-4".into(),
            google_model: "gemini-2.5-pro".into(),
            thinking_level: "normal".into(),
            save_traces: false,
            traces_scope: "all".into(),
            computer_use: false,
            browser_use: true,
            browser_headless: true,
            geckodriver_path: String::new(),
            firefox_path: String::new(),
            disabled_plugins: Vec::new(),
        }
    }
}

impl Settings {
    pub fn is_default_endpoint(&self) -> bool {
        let b = self.base_url.trim().trim_end_matches('/');
        b.is_empty() || b == DEFAULT_BASE_URL
    }

    pub fn api_base(&self) -> String {
        let b = self.base_url.trim().trim_end_matches('/');
        if b.is_empty() { DEFAULT_BASE_URL.to_string() } else { b.to_string() }
    }
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Secrets {
    anthropic_api_key: String,
    openrouter_api_key: String,
    local_api_key: String,
    openai_api_key: String,
    xai_api_key: String,
    google_api_key: String,
}

impl Secrets {
    fn slot(&mut self, provider: &str) -> Option<&mut String> {
        Some(match provider {
            "anthropic" => &mut self.anthropic_api_key,
            "openrouter" => &mut self.openrouter_api_key,
            "local" => &mut self.local_api_key,
            "openai" => &mut self.openai_api_key,
            "xai" => &mut self.xai_api_key,
            "google" => &mut self.google_api_key,
            _ => return None,
        })
    }
}

/// Providers that take a key, with the environment variables checked when none is saved.
pub const KEY_PROVIDERS: &[(&str, &[&str])] = &[
    ("anthropic", &["ANTHROPIC_API_KEY"]),
    ("openai", &["OPENAI_API_KEY"]),
    ("xai", &["XAI_API_KEY"]),
    ("google", &["GEMINI_API_KEY", "GOOGLE_API_KEY"]),
    ("openrouter", &["OPENROUTER_API_KEY"]),
    ("local", &[]),
];

/// What the UI sees: settings plus API-key status (never the key itself).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    #[serde(flatten)]
    pub settings: Settings,
    pub has_api_key: bool,
    /// "settings" | "env"
    pub api_key_source: Option<&'static str>,
    pub api_key_hint: Option<String>,
    pub has_openrouter_key: bool,
    pub has_local_key: bool,
    /// Provider id → a key is available (saved or from the environment).
    pub provider_keys: std::collections::BTreeMap<&'static str, bool>,
    pub config_dir: String,
}

pub struct SettingsStore {
    dir: PathBuf,
    settings: RwLock<Settings>,
    api_key: RwLock<String>,
    secrets: RwLock<Secrets>,
}

impl SettingsStore {
    pub fn load(dir: PathBuf) -> Self {
        let settings = std::fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_default();
        let secrets = std::fs::read(dir.join("secrets.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Secrets>(&b).ok())
            .unwrap_or_default();
        let api_key = secrets.anthropic_api_key.clone();
        Self { dir, settings: RwLock::new(settings), api_key: RwLock::new(api_key), secrets: RwLock::new(secrets) }
    }

    pub fn get(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    pub fn permission_mode(&self) -> PermissionMode {
        self.settings.read().unwrap().permission_mode
    }

    /// The configured key, or `ANTHROPIC_API_KEY` from the environment.
    pub fn api_key(&self) -> Option<String> {
        let k = self.api_key.read().unwrap().trim().to_string();
        if !k.is_empty() {
            return Some(k);
        }
        std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.trim().is_empty())
    }

    pub fn view(&self) -> SettingsView {
        let stored = self.api_key.read().unwrap().trim().to_string();
        let (source, key) = if !stored.is_empty() {
            (Some("settings"), Some(stored))
        } else {
            match std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.trim().is_empty()) {
                Some(k) => (Some("env"), Some(k)),
                None => (None, None),
            }
        };
        SettingsView {
            settings: self.get(),
            has_api_key: key.is_some(),
            api_key_source: source,
            api_key_hint: key.map(|k| {
                let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
                format!("…{tail}")
            }),
            has_openrouter_key: self.provider_key("openrouter").is_some(),
            has_local_key: self.provider_key("local").is_some(),
            provider_keys: KEY_PROVIDERS.iter().map(|(p, _)| (*p, self.provider_key(p).is_some())).collect(),
            config_dir: self.dir.display().to_string(),
        }
    }

    /// Merge a partial JSON object into the settings (type-checked by deserializing).
    pub fn update(&self, patch: serde_json::Value) -> Result<SettingsView> {
        let serde_json::Value::Object(patch) = patch else {
            return Err(Error::msg("settings patch must be an object"));
        };
        {
            let mut guard = self.settings.write().unwrap();
            let mut current = serde_json::to_value(&*guard)?;
            let obj = current.as_object_mut().expect("settings serialize to an object");
            for (k, v) in patch {
                obj.insert(k, v);
            }
            let mut next: Settings = serde_json::from_value(current)
                .map_err(|e| Error::msg(format!("invalid setting: {e}")))?;
            next.editor_font_size = next.editor_font_size.clamp(8, 40);
            next.max_tokens = next.max_tokens.clamp(1024, 128_000);
            *guard = next;
        }
        self.save()?;
        Ok(self.view())
    }

    pub fn add_recent_workspace(&self, path: &str) {
        {
            let mut s = self.settings.write().unwrap();
            s.recent_workspaces.retain(|p| p != path);
            s.recent_workspaces.insert(0, path.to_string());
            s.recent_workspaces.truncate(12);
        }
        let _ = self.save();
    }

    pub fn set_api_key(&self, key: String) -> Result<SettingsView> {
        self.set_secret("anthropic", key)
    }

    /// Store a provider secret (see `KEY_PROVIDERS`).
    pub fn set_secret(&self, provider: &str, key: String) -> Result<SettingsView> {
        let key = key.trim().to_string();
        {
            let mut s = self.secrets.write().unwrap();
            let slot = s.slot(provider).ok_or_else(|| Error::msg(format!("unknown provider {provider}")))?;
            *slot = key.clone();
            if provider == "anthropic" {
                *self.api_key.write().unwrap() = key;
            }
            write_private(&self.dir.join("secrets.json"), &serde_json::to_vec_pretty(&*s)?)?;
        }
        Ok(self.view())
    }

    /// A provider's saved key, else its environment variable.
    pub fn provider_key(&self, provider: &str) -> Option<String> {
        if provider == "anthropic" {
            return self.api_key();
        }
        let saved = self.secrets.write().unwrap().slot(provider).map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        saved.or_else(|| {
            let vars = KEY_PROVIDERS.iter().find(|(p, _)| *p == provider).map(|(_, v)| *v).unwrap_or(&[]);
            vars.iter().find_map(|v| std::env::var(v).ok().filter(|k| !k.trim().is_empty()))
        })
    }

    pub fn config_dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn save(&self) -> Result<()> {
        let data = serde_json::to_vec_pretty(&*self.settings.read().unwrap())?;
        atomic_write(&self.dir.join("settings.json"), &data)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_merges_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::load(dir.path().to_path_buf());
        assert_eq!(store.get().model, DEFAULT_MODEL);
        let v = store
            .update(serde_json::json!({"model": "claude-sonnet-5-5", "permissionMode": "acceptEdits", "editorFontSize": 99}))
            .unwrap();
        assert_eq!(v.settings.model, "claude-sonnet-5-5");
        assert_eq!(v.settings.permission_mode, PermissionMode::AcceptEdits);
        assert_eq!(v.settings.editor_font_size, 40);
        let reloaded = SettingsStore::load(dir.path().to_path_buf());
        assert_eq!(reloaded.get().model, "claude-sonnet-5-5");
        // bad types are rejected and leave settings unchanged
        assert!(store.update(serde_json::json!({"maxTokens": "lots"})).is_err());
        assert_eq!(store.get().model, "claude-sonnet-5-5");
    }

    #[test]
    fn api_key_is_private_and_hinted() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::load(dir.path().to_path_buf());
        let v = store.set_api_key("sk-ant-test-abcd1234".into()).unwrap();
        assert!(v.has_api_key);
        assert_eq!(v.api_key_hint.as_deref(), Some("…1234"));
        let json = serde_json::to_string(&v).unwrap();
        assert!(!json.contains("sk-ant-test"), "key must never be serialized to the UI");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("secrets.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(SettingsStore::load(dir.path().to_path_buf()).api_key().as_deref(), Some("sk-ant-test-abcd1234"));
    }
}
