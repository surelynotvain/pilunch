//! Commands for usage, traces, skills, custom tools, MCP, plugins and the browser panel.

use crate::error::{Error, Result};
use crate::ext::{self, custom_tools, mcp, plugins, skills};
use crate::records::{TraceStats, UsageSummary};
use crate::state::AppState;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

fn project_root(state: &AppState) -> Option<PathBuf> {
    state.workspace.read().unwrap().as_ref().map(|w| w.root().to_path_buf())
}

fn loaded(state: &AppState) -> ext::Loaded {
    let s = state.settings.get();
    ext::load(state.settings.config_dir(), project_root(state).as_deref(), &s.disabled_plugins)
}

// ---- usage & traces -----------------------------------------------------------------

#[tauri::command]
pub fn usage_summary(state: State<'_, AppState>, days: Option<u32>) -> UsageSummary {
    state.records.usage_summary(days.unwrap_or(30).clamp(1, 365))
}

#[tauri::command]
pub fn clear_usage(state: State<'_, AppState>) -> Result<()> {
    state.records.clear_usage()
}

#[tauri::command]
pub fn trace_stats(state: State<'_, AppState>) -> TraceStats {
    state.records.trace_stats()
}

#[tauri::command]
pub fn export_traces(state: State<'_, AppState>, path: String, local_only: bool) -> Result<usize> {
    state.records.export_traces(Path::new(&path), local_only)
}

#[tauri::command]
pub fn clear_traces(state: State<'_, AppState>) -> Result<()> {
    let dir = state.records.traces_dir();
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if e.path().extension().is_some_and(|x| x == "json") {
            std::fs::remove_file(e.path())?;
        }
    }
    Ok(())
}

// ---- skills -------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillText {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Paths a skill command may touch: SKILL.md files inside the known skill folders.
fn skill_path(state: &AppState, path: &str) -> Result<PathBuf> {
    let p = dunce::canonicalize(path).map_err(|_| Error::msg("skill not found"))?;
    let dirs = loaded(state).skills.dirs;
    let inside = dirs.iter().any(|(_, d)| dunce::canonicalize(d).is_ok_and(|d| p.starts_with(d)));
    if !inside || p.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
        return Err(Error::msg("not a skill file"));
    }
    Ok(p)
}

#[tauri::command]
pub fn list_skills(state: State<'_, AppState>) -> Vec<skills::Skill> {
    loaded(&state).skills.list()
}

#[tauri::command]
pub fn read_skill(state: State<'_, AppState>, path: String) -> Result<SkillText> {
    let p = skill_path(&state, &path)?;
    let text = std::fs::read_to_string(&p)?;
    let (meta, body) = skills::parse(&text);
    let folder = p.parent().and_then(|d| d.file_name()).and_then(|n| n.to_str()).unwrap_or_default().to_string();
    Ok(SkillText { name: meta.name.unwrap_or(folder), description: meta.description.unwrap_or_default(), body: body.trim().to_string() })
}

#[tauri::command]
pub fn save_skill(state: State<'_, AppState>, scope: String, name: String, description: String, body: String) -> Result<String> {
    let l = loaded(&state);
    let dir = l.skills.dir_for(&scope).ok_or_else(|| Error::msg(if scope == "project" { "Open a folder to save project skills" } else { "unknown scope" }))?;
    let path = skills::write(dir, name.trim(), &skills::render(name.trim(), &description, &body))?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn delete_skill(state: State<'_, AppState>, path: String) -> Result<()> {
    let p = skill_path(&state, &path)?;
    if let Some(dir) = p.parent() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

// ---- custom tools -------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolList {
    pub tools: Vec<custom_tools::CustomTool>,
    pub errors: Vec<String>,
    pub user_dir: String,
}

#[tauri::command]
pub fn list_custom_tools(state: State<'_, AppState>) -> ToolList {
    let l = loaded(&state);
    ToolList { tools: l.tools, errors: l.tool_errors, user_dir: state.settings.config_dir().join("tools").display().to_string() }
}

#[tauri::command]
pub fn save_custom_tool(state: State<'_, AppState>, scope: String, spec: custom_tools::ToolSpec) -> Result<String> {
    let dir = match scope.as_str() {
        "project" => project_root(&state).ok_or_else(|| Error::msg("Open a folder to save project tools"))?.join(".pilunch").join("tools"),
        _ => state.settings.config_dir().join("tools"),
    };
    custom_tools::save(&dir, &spec).map(|p| p.display().to_string()).map_err(Error::msg)
}

#[tauri::command]
pub fn delete_custom_tool(state: State<'_, AppState>, name: String) -> Result<()> {
    let l = loaded(&state);
    let t = l.tools.into_iter().find(|t| t.spec.name == name && !t.scope.starts_with("plugin:")).ok_or_else(|| Error::msg("tool not found (plugin tools are removed with their plugin)"))?;
    std::fs::remove_file(&t.path)?;
    Ok(())
}

// ---- MCP ----------------------------------------------------------------------------

#[tauri::command]
pub fn mcp_config(state: State<'_, AppState>) -> String {
    let file = mcp::McpFile::load(state.settings.config_dir());
    serde_json::to_string_pretty(&file).unwrap_or_else(|_| "{\n  \"mcpServers\": {}\n}".into())
}

#[tauri::command]
pub fn save_mcp_config(state: State<'_, AppState>, raw: String) -> Result<()> {
    mcp::McpFile::save_raw(state.settings.config_dir(), &raw).map(|_| ()).map_err(Error::msg)
}

/// Connect every configured server and report its state and tools.
#[tauri::command]
pub async fn mcp_status(state: State<'_, AppState>, reconnect: Option<String>) -> Result<Vec<mcp::ServerStatus>> {
    if let Some(name) = reconnect {
        state.mcp.disconnect(&name).await;
    }
    let l = loaded(&state);
    state.mcp.ensure(&l.servers, &state.http, state.settings.config_dir()).await;
    Ok(state.mcp.status())
}

// ---- plugins ------------------------------------------------------------------------

#[tauri::command]
pub fn list_plugins(state: State<'_, AppState>) -> Vec<plugins::Plugin> {
    plugins::list(state.settings.config_dir(), &state.settings.get().disabled_plugins)
}

#[tauri::command]
pub fn install_plugin_folder(state: State<'_, AppState>, path: String) -> Result<String> {
    plugins::install_folder(state.settings.config_dir(), Path::new(&path)).map_err(Error::msg)
}

#[tauri::command]
pub async fn install_plugin_git(state: State<'_, AppState>, url: String) -> Result<String> {
    plugins::install_git(state.settings.config_dir(), &url).await.map_err(Error::msg)
}

#[tauri::command]
pub fn remove_plugin(state: State<'_, AppState>, id: String) -> Result<()> {
    plugins::remove(state.settings.config_dir(), &id).map_err(Error::msg)
}

#[tauri::command]
pub fn set_plugin_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> Result<crate::settings::SettingsView> {
    let mut disabled = state.settings.get().disabled_plugins;
    disabled.retain(|d| d != &id);
    if !enabled {
        disabled.push(id);
    }
    state.settings.update(serde_json::json!({ "disabledPlugins": disabled }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResult {
    pub ok: bool,
    pub id: Option<String>,
    pub log: String,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn build_rust_extension(state: State<'_, AppState>, path: String) -> Result<BuildResult> {
    let mut log = String::new();
    let res = plugins::build_rust_extension(state.settings.config_dir(), Path::new(&path), |s| log.push_str(s)).await;
    Ok(match res {
        Ok(id) => BuildResult { ok: true, id: Some(id), log, error: None },
        Err(e) => BuildResult { ok: false, id: None, log, error: Some(e) },
    })
}

// ---- misc ---------------------------------------------------------------------------

/// Show a PiLunch folder (config, data, plugin…) in the file manager.
#[tauri::command]
pub fn open_folder(app: tauri::AppHandle, path: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let p = Path::new(&path);
    if !p.is_dir() {
        std::fs::create_dir_all(p).map_err(|e| Error::msg(format!("{path}: {e}")))?;
    }
    app.opener().open_path(path, None::<&str>).map_err(|e| Error::msg(e.to_string()))
}

// ---- browser panel ------------------------------------------------------------------

/// Run a browser action from the panel and return a fresh screenshot.
#[tauri::command]
pub async fn browser_action(state: State<'_, AppState>, action: serde_json::Value) -> Result<crate::browser::Outcome> {
    let a: crate::browser::Action = serde_json::from_value(action).map_err(|e| Error::msg(e.to_string()))?;
    let opts = crate::agent::browser_options(&state.settings.get());
    let mut out = state.browser.run(&state.http, &opts, &a).await.map_err(Error::msg)?;
    if a.action != "close" && a.action != "screenshot" {
        let shot = state.browser.run(&state.http, &opts, &crate::browser::Action { action: "screenshot".into(), ..Default::default() }).await.map_err(Error::msg)?;
        out.screenshot = shot.screenshot;
        out.url = shot.url;
        out.title = shot.title;
    }
    Ok(out)
}

#[tauri::command]
pub async fn browser_running(state: State<'_, AppState>) -> Result<bool> {
    Ok(state.browser.is_running().await)
}
