use crate::agent::AgentManager;
use crate::conversations::ConversationStore;
use crate::error::{Error, Result};
use crate::records::Records;
use crate::search::FileIndex;
use crate::settings::SettingsStore;
use crate::terminal::Terminals;
use crate::watcher::WatcherHandle;
use crate::workspace::Workspace;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

pub struct AppState {
    pub settings: SettingsStore,
    pub conversations: ConversationStore,
    pub workspace: RwLock<Option<Workspace>>,
    pub index: FileIndex,
    pub watcher: Mutex<Option<WatcherHandle>>,
    pub terminals: Terminals,
    pub agent: AgentManager,
    pub http: reqwest::Client,
    pub records: Records,
    pub mcp: crate::ext::mcp::McpManager,
    pub browser: crate::browser::Browser,
}

impl AppState {
    pub fn new(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            settings: SettingsStore::load(config_dir),
            conversations: ConversationStore::load(data_dir.join("conversations")),
            records: Records::new(data_dir.clone()),
            mcp: Default::default(),
            browser: Default::default(),
            workspace: RwLock::new(None),
            index: FileIndex::default(),
            watcher: Mutex::new(None),
            terminals: Terminals::default(),
            agent: AgentManager::default(),
            http: crate::agent::api::http_client(),
        }
    }

    pub fn workspace(&self) -> Result<Workspace> {
        self.workspace.read().unwrap().clone().ok_or(Error::NoWorkspace)
    }
}

/// `~/.config/pilunch` and `~/.local/share/pilunch` (XDG on Linux, AppData on Windows),
/// overridable with PILUNCH_CONFIG_DIR / PILUNCH_DATA_DIR (portable installs, tests).
pub fn app_dirs() -> (PathBuf, PathBuf) {
    let config = std::env::var_os("PILUNCH_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("pilunch"));
    let data = std::env::var_os("PILUNCH_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("pilunch"));
    let _ = std::fs::create_dir_all(&config);
    let _ = std::fs::create_dir_all(&data);
    (config, data)
}
