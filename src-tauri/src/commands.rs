//! Tauri commands: the IPC surface used by the TypeScript UI. They are thin wrappers;
//! heavy work runs on the blocking pool so the async runtime never stalls.

use crate::agent::{self, events::AgentEvent, Decision, SendRequest};
use crate::conversations::{Conversation, ConversationMeta};
use crate::error::{Error, Result};
use crate::fs_ops::{self, DirEntryInfo, FileContent};
use crate::git::{self, GitStatus};
use crate::search::{self, FileMatch, GrepOptions, GrepResult};
use crate::settings::SettingsView;
use crate::state::AppState;
use crate::workspace::Workspace;
use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager, State};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| Error::msg(e.to_string()))?
}

// ---- settings -----------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsView {
    state.settings.view()
}

#[tauri::command]
pub fn update_settings(state: State<'_, AppState>, patch: serde_json::Value) -> Result<SettingsView> {
    state.settings.update(patch)
}

#[tauri::command]
pub fn set_api_key(state: State<'_, AppState>, key: String) -> Result<SettingsView> {
    state.settings.set_api_key(key)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    id: String,
    display_name: String,
}

/// Validates the active provider's credentials and returns the models it can use.
#[tauri::command]
pub async fn list_models(state: State<'_, AppState>, provider: Option<String>) -> Result<Vec<ModelInfo>> {
    let mut settings = state.settings.get();
    if let Some(p) = provider {
        settings.provider = p;
    }
    let to_info = |v: Vec<(String, String)>| v.into_iter().map(|(id, display_name)| ModelInfo { id, display_name }).collect();
    match settings.provider.as_str() {
        "openrouter" | "local" => {
            let ep = agent::openai::Endpoint::for_settings(&settings, state.settings.openrouter_key(), state.settings.local_key()).expect("openai provider");
            let resp = ep
                .request(&state.http, reqwest::Method::GET, "/models")
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await
                .map_err(|e| Error::msg(format!("Can't reach {}: {}", ep.base, agent::api::describe_reqwest(&e))))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                return Err(Error::msg(format!("{status}: {}", agent::openai::error_message(&text))));
            }
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| Error::msg(format!("unexpected response: {e}")))?;
            Ok(to_info(agent::openai::parse_models(&v)))
        }
        _ => {
            let key = state.settings.api_key().ok_or_else(|| Error::msg("No API key configured"))?;
            agent::api::list_models(&state.http, &settings.api_base(), &key).await.map(to_info).map_err(|e| Error::msg(e.user_message()))
        }
    }
}

#[tauri::command]
pub fn set_provider_key(state: State<'_, AppState>, provider: String, key: String) -> Result<SettingsView> {
    state.settings.set_secret(&provider, key)
}

// ---- workspace ----------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    root: String,
    name: String,
}

fn info(ws: &Workspace) -> WorkspaceInfo {
    WorkspaceInfo { root: ws.root().display().to_string(), name: ws.name() }
}

#[tauri::command]
pub async fn open_workspace(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<WorkspaceInfo> {
    let ws = Workspace::open(&path)?;
    let root = ws.root().to_path_buf();
    *state.workspace.write().unwrap() = Some(ws.clone());
    state.settings.add_recent_workspace(&root.display().to_string());
    state.index.mark_dirty();
    // Replace the watcher; build the quick-open index in the background.
    *state.watcher.lock().unwrap() = None;
    match crate::watcher::watch(app.clone(), root.clone()) {
        Ok(w) => *state.watcher.lock().unwrap() = Some(w),
        Err(e) => eprintln!("file watcher unavailable: {e}"),
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let files = app2.state::<AppState>().index.files(&root);
        let _ = app2.emit("index-ready", files.len());
    });
    Ok(info(&ws))
}

#[tauri::command]
pub fn close_workspace(state: State<'_, AppState>) {
    *state.workspace.write().unwrap() = None;
    *state.watcher.lock().unwrap() = None;
}

#[tauri::command]
pub fn current_workspace(state: State<'_, AppState>) -> Option<WorkspaceInfo> {
    state.workspace.read().unwrap().as_ref().map(info)
}

// ---- files --------------------------------------------------------------------------

#[tauri::command]
pub async fn list_dir(state: State<'_, AppState>, path: String) -> Result<Vec<DirEntryInfo>> {
    let ws = state.workspace()?;
    blocking(move || fs_ops::list_dir(&ws, &path)).await
}

#[tauri::command]
pub async fn read_file(state: State<'_, AppState>, path: String) -> Result<FileContent> {
    let ws = state.workspace()?;
    blocking(move || fs_ops::read_file(&ws, &path)).await
}

#[tauri::command]
pub async fn write_file(state: State<'_, AppState>, path: String, content: String) -> Result<()> {
    let ws = state.workspace()?;
    blocking(move || fs_ops::write_file(&ws, &path, &content)).await
}

#[tauri::command]
pub fn create_file(state: State<'_, AppState>, path: String) -> Result<String> {
    fs_ops::create_file(&state.workspace()?, &path)
}

#[tauri::command]
pub fn create_dir(state: State<'_, AppState>, path: String) -> Result<String> {
    fs_ops::create_dir(&state.workspace()?, &path)
}

#[tauri::command]
pub fn rename_path(state: State<'_, AppState>, from: String, to: String) -> Result<String> {
    fs_ops::rename(&state.workspace()?, &from, &to)
}

#[tauri::command]
pub async fn delete_path(state: State<'_, AppState>, path: String, permanent: bool) -> Result<()> {
    let ws = state.workspace()?;
    blocking(move || fs_ops::delete(&ws, &path, permanent)).await
}

// ---- search -------------------------------------------------------------------------

#[tauri::command]
pub async fn quick_open(app: AppHandle, query: String, limit: Option<usize>) -> Result<Vec<FileMatch>> {
    let ws = app.state::<AppState>().workspace()?;
    blocking(move || {
        let files = app.state::<AppState>().index.files(ws.root());
        Ok(search::fuzzy(&files, &query, limit.unwrap_or(50).min(500)))
    })
    .await
}

#[tauri::command]
pub async fn search_text(state: State<'_, AppState>, query: String, options: GrepOptions) -> Result<GrepResult> {
    let ws = state.workspace()?;
    blocking(move || search::grep(ws.root(), ws.root(), &query, &options)).await
}

#[tauri::command]
pub async fn git_status(state: State<'_, AppState>) -> Result<Option<GitStatus>> {
    let ws = state.workspace()?;
    Ok(git::status(ws.root()).await)
}

// ---- terminal -----------------------------------------------------------------------

#[tauri::command]
pub fn terminal_spawn(app: AppHandle, state: State<'_, AppState>, cols: u16, rows: u16, on_data: Channel<InvokeResponseBody>) -> Result<u32> {
    let cwd = match state.workspace() {
        Ok(ws) => ws.root().to_path_buf(),
        Err(_) => dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")),
    };
    let shell = crate::process::terminal_shell(&state.settings.get().terminal_shell);
    state.terminals.spawn(&shell, &cwd, cols, rows, on_data, move |id| {
        let _ = app.emit("terminal-exit", id);
    })
}

#[tauri::command]
pub fn terminal_write(state: State<'_, AppState>, id: u32, data: String) -> Result<()> {
    state.terminals.write(id, data.as_bytes())
}

#[tauri::command]
pub fn terminal_resize(state: State<'_, AppState>, id: u32, cols: u16, rows: u16) -> Result<()> {
    state.terminals.resize(id, cols, rows)
}

#[tauri::command]
pub fn terminal_kill(state: State<'_, AppState>, id: u32) {
    state.terminals.kill(id);
}

// ---- conversations ------------------------------------------------------------------

#[tauri::command]
pub fn list_conversations(state: State<'_, AppState>) -> Vec<ConversationMeta> {
    state.conversations.list()
}

#[tauri::command]
pub fn create_conversation(state: State<'_, AppState>) -> Result<ConversationMeta> {
    let ws = state.workspace.read().unwrap().as_ref().map(|w| w.root().display().to_string());
    Ok(state.conversations.create(ws)?.meta)
}

#[tauri::command]
pub async fn get_conversation(state: State<'_, AppState>, id: String) -> Result<Conversation> {
    state.conversations.get(&id)
}

#[tauri::command]
pub fn rename_conversation(state: State<'_, AppState>, id: String, title: String) -> Result<ConversationMeta> {
    state.conversations.rename(&id, &title)
}

#[tauri::command]
pub fn delete_conversation(state: State<'_, AppState>, id: String) -> Result<()> {
    state.agent.cancel(&id);
    state.conversations.delete(&id)
}

// ---- agent --------------------------------------------------------------------------

#[tauri::command]
pub fn agent_send(app: AppHandle, request: SendRequest, on_event: Channel<AgentEvent>) -> Result<()> {
    agent::start(app, request, on_event)
}

#[tauri::command]
pub fn agent_cancel(state: State<'_, AppState>, conversation_id: String) {
    state.agent.cancel(&conversation_id);
}

#[tauri::command]
pub fn agent_cancel_all(state: State<'_, AppState>) {
    state.agent.cancel_all();
}

#[tauri::command]
pub fn agent_respond(state: State<'_, AppState>, approval_id: String, decision: Decision, feedback: Option<String>) -> Result<()> {
    state.agent.respond(&approval_id, decision, feedback)
}

#[tauri::command]
pub fn agent_running(state: State<'_, AppState>) -> Vec<String> {
    state.agent.running()
}
