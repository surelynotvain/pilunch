//! Tools beyond the built-in set: skills, custom tools, MCP servers, the browser and
//! computer use. Loaded at the start of each run; executed with the same approval rules as
//! the built-in tools.

use super::events::{AgentEvent, Sink};
use super::tools::{Image, ToolResult};
use super::{cancelled_result, denied_result, Decision, Run};
use crate::conversations::{ToolStatus, ToolUi};
use crate::ext::{self, custom_tools::CustomTool, mcp, skills};
use crate::settings::PermissionMode;
use crate::state::AppState;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Runtime;

#[derive(Clone)]
pub(super) enum ExtTool {
    SkillLoad,
    SkillSave,
    Custom(CustomTool),
    Mcp { client: Arc<mcp::Client>, tool: String, read_only: bool },
    Browser,
    Computer,
}

pub(super) struct Extras {
    pub tools: BTreeMap<String, (ExtTool, Value)>,
    pub skills: skills::SkillDirs,
    pub prompt: String,
}

impl Extras {
    pub fn empty(config_dir: &std::path::Path) -> Self {
        Extras { tools: BTreeMap::new(), skills: skills::SkillDirs::new(config_dir, None, &[]), prompt: String::new() }
    }

    /// Definitions for the current permission mode (Plan mode: read-only ones).
    pub fn definitions(&self, mode: PermissionMode) -> Vec<Value> {
        self.tools
            .values()
            .filter(|(t, _)| {
                mode != PermissionMode::Plan || matches!(t, ExtTool::SkillLoad | ExtTool::Browser | ExtTool::Computer | ExtTool::Mcp { read_only: true, .. })
            })
            .map(|(_, d)| d.clone())
            .collect()
    }
}

fn skill_defs() -> [Value; 2] {
    [
        json!({
            "name": "skill_load",
            "description": "Read a saved skill (instructions for a recurring task) by name. Skills are listed in the system prompt.",
            "input_schema": { "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }
        }),
        json!({
            "name": "skill_save",
            "description": "Create or update a skill: reusable Markdown instructions you (or the user) can load in later conversations. \
Save what you learn when it will be useful again — a project's release procedure, how an API works after researching it, a \
fix that took several attempts. Write clear, step-by-step instructions with the commands, file paths and pitfalls. To improve \
an existing skill, load it first and save the full new text.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short id: letters, digits, '-' and '_'." },
                    "description": { "type": "string", "description": "One line: when to use this skill." },
                    "content": { "type": "string", "description": "The full Markdown instructions." },
                    "scope": { "type": "string", "enum": ["user", "project"], "description": "user = all projects (default); project = saved in .pilunch/skills of this project." }
                },
                "required": ["name", "description", "content"]
            }
        }),
    ]
}

/// Load skills, custom tools, plugins and MCP servers for a run.
pub(super) async fn load(state: &AppState, s: &crate::settings::Settings, project: Option<&std::path::Path>, sink: &mut Sink) -> Extras {
    let config_dir = state.settings.config_dir().to_path_buf();
    let loaded = ext::load(&config_dir, project, &s.disabled_plugins);
    let mut tools: BTreeMap<String, (ExtTool, Value)> = BTreeMap::new();
    let [load_def, save_def] = skill_defs();
    tools.insert("skill_load".into(), (ExtTool::SkillLoad, load_def));
    tools.insert("skill_save".into(), (ExtTool::SkillSave, save_def));
    if project.is_some() {
        for t in &loaded.tools {
            tools.insert(t.spec.name.clone(), (ExtTool::Custom(t.clone()), ext::custom_tools::definition(t)));
        }
    }
    if !loaded.servers.is_empty() {
        let clients = state.mcp.ensure(&loaded.servers, &state.http, &config_dir).await;
        for st in state.mcp.status().iter().filter(|st| st.state == "error") {
            sink.send(AgentEvent::Notice { message: format!("MCP server “{}” is unavailable: {}", st.name, st.error.as_deref().unwrap_or("error").lines().next().unwrap_or_default()) });
        }
        for c in clients {
            for t in &c.tools {
                let name = mcp::tool_name(&c.name, &t.name);
                let def = json!({ "name": name, "description": format!("[{} MCP] {}", c.name, t.description), "input_schema": t.input_schema });
                tools.insert(name, (ExtTool::Mcp { client: c.clone(), tool: t.name.clone(), read_only: t.read_only }, def));
            }
        }
    }
    if s.browser_use {
        tools.insert("browser".into(), (ExtTool::Browser, crate::browser::tool_definition()));
    }
    if s.computer_use {
        tools.insert("computer".into(), (ExtTool::Computer, crate::computer::tool_definition()));
    }
    let skills = loaded.skills;
    let prompt = skills::prompt_section(&skills.list());
    Extras { tools, skills, prompt }
}

fn images_of(shot: Option<&crate::media::Shot>) -> Vec<Image> {
    shot.map(|s| vec![s.image()]).unwrap_or_default()
}

fn image_ui(mut r: ToolResult, shot: Option<&crate::media::Shot>) -> ToolResult {
    if let Some(s) = shot {
        r.ui.detail = Some(crate::media::data_url(s));
        r.ui.detail_kind = Some("image".into());
    }
    r
}

impl<R: Runtime> Run<'_, R> {
    /// Run an extension tool; `None` if `name` isn't one.
    pub(super) async fn run_ext(&mut self, id: &str, name: &str, input: &Value, sink: &mut Sink) -> Option<ToolResult> {
        let (tool, _) = self.extras.tools.get(name)?.clone();
        let mode = self.state.settings.permission_mode();
        let grants = self.state.agent.grants(&self.conv.meta.id);
        let summary = describe(&tool, name, input);
        let running = |r: &mut Self, sink: &mut Sink| r.set_tool_ui(sink, id, ToolUi { status: ToolStatus::Running, summary: summary.clone(), detail: None, detail_kind: None, path: None });
        Some(match tool {
            ExtTool::SkillLoad => {
                let n = input["name"].as_str().unwrap_or_default();
                match self.extras.skills.list().into_iter().find(|s| s.name == n) {
                    Some(sk) => match std::fs::read_to_string(&sk.path) {
                        Ok(text) => ToolResult::done(skills::parse(&text).1.trim().to_string(), summary, None),
                        Err(e) => ToolResult::error(format!("Couldn't read {}: {e}", sk.path), summary),
                    },
                    None => ToolResult::error(format!("No skill named \"{n}\". Available: {}", self.extras.skills.list().iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")), summary),
                }
            }
            ExtTool::SkillSave => self.skill_save(id, input, summary, mode, grants.edits, sink).await,
            ExtTool::Custom(t) => {
                if mode == PermissionMode::Plan {
                    return Some(ToolResult::error("Plan mode is on: custom tools are disabled.", summary));
                }
                let Some(ws) = self.ws.clone() else { return Some(ToolResult::error("Custom tools need an open folder.", summary)) };
                let command = ext::custom_tools::expand(&t.spec.command, input);
                if matches!(mode, PermissionMode::Ask | PermissionMode::AcceptEdits) && !grants.tools {
                    match self.ask(sink, id, "command", &summary, &command).await {
                        None => return Some(cancelled_result(summary)),
                        Some((Decision::Deny, fb)) => return Some(denied_result("command", summary, fb, None)),
                        Some((Decision::AllowSession, _)) => self.grant(|g| g.tools = true),
                        Some((Decision::Allow, _)) => {}
                    }
                }
                running(self, sink);
                let env = ext::custom_tools::env(input, t.plugin_dir.as_deref());
                let buf = Mutex::new(String::new());
                let fut = super::tools::run_command_env(&command, t.spec.timeout, &ws, &self.cancel, &env, |s| buf.lock().unwrap().push_str(s));
                tokio::pin!(fut);
                let mut tick = tokio::time::interval(Duration::from_millis(50));
                let mut res = loop {
                    tokio::select! {
                        r = &mut fut => break r,
                        _ = tick.tick() => {
                            let text = std::mem::take(&mut *buf.lock().unwrap());
                            if !text.is_empty() {
                                sink.send(AgentEvent::ToolOutput { tool_id: id.to_string(), text });
                            }
                        }
                    }
                };
                res.ui.summary = summary;
                res
            }
            ExtTool::Mcp { client, tool, read_only } => {
                if mode == PermissionMode::Plan && !read_only {
                    return Some(ToolResult::error("Plan mode is on: only read-only MCP tools can run.", summary));
                }
                if !read_only && mode != PermissionMode::Bypass && !grants.tools {
                    let detail = serde_json::to_string_pretty(input).unwrap_or_default();
                    match self.ask(sink, id, "tool", &summary, &detail).await {
                        None => return Some(cancelled_result(summary)),
                        Some((Decision::Deny, fb)) => return Some(denied_result("tool call", summary, fb, None)),
                        Some((Decision::AllowSession, _)) => self.grant(|g| g.tools = true),
                        Some((Decision::Allow, _)) => {}
                    }
                }
                running(self, sink);
                let out = tokio::select! {
                    r = client.call(&tool, input.clone()) => r,
                    _ = self.cancel.cancelled() => return Some(cancelled_result(summary)),
                };
                match out {
                    Ok(o) => {
                        let images: Vec<Image> = o.images.into_iter().map(|(media_type, data)| Image { media_type, data }).collect();
                        let text = if o.text.is_empty() { "(no output)".to_string() } else { o.text };
                        let mut r = ToolResult::done(text.clone(), summary, Some(text)).with_images(images);
                        if o.is_error {
                            r.ui.status = ToolStatus::Error;
                            r.is_error = true;
                        }
                        r
                    }
                    Err(e) => {
                        if !client.is_alive() {
                            self.state.mcp.disconnect(&client.name).await;
                        }
                        ToolResult::error(format!("MCP error: {e}"), summary)
                    }
                }
            }
            ExtTool::Browser => {
                let action: crate::browser::Action = match serde_json::from_value(input.clone()) {
                    Ok(a) => a,
                    Err(e) => return Some(ToolResult::error(format!("Invalid tool input: {e}"), summary)),
                };
                if mode == PermissionMode::Plan && !crate::browser::is_read_only(&action.action) && action.action != "navigate" {
                    return Some(ToolResult::error("Plan mode is on: the browser is read-only (navigate, snapshot, screenshot, scroll).", summary));
                }
                if action.action == "navigate" && mode != PermissionMode::Bypass && !grants.network {
                    let url = action.url.clone().unwrap_or_default();
                    match self.ask(sink, id, "network", &summary, &url).await {
                        None => return Some(cancelled_result(summary)),
                        Some((Decision::Deny, fb)) => return Some(denied_result("request", summary, fb, None)),
                        Some((Decision::AllowSession, _)) => self.grant(|g| g.network = true),
                        Some((Decision::Allow, _)) => {}
                    }
                }
                running(self, sink);
                let opts = browser_options(&self.state.settings.get());
                let res = tokio::select! {
                    r = self.state.browser.run(&self.state.http, &opts, &action) => r,
                    _ = self.cancel.cancelled() => return Some(cancelled_result(summary)),
                };
                let _ = self.app_emit_browser();
                match res {
                    Ok(o) => {
                        let content = if o.url.is_empty() || o.text.starts_with("URL:") { o.text.clone() } else { format!("{}\nNow at: {} — {}", o.text, o.url, o.title) };
                        let r = ToolResult::done(content, summary, Some(o.text.clone())).with_images(images_of(o.shot.as_ref()));
                        image_ui(r, o.shot.as_ref())
                    }
                    Err(e) => ToolResult::error(e, summary),
                }
            }
            ExtTool::Computer => {
                let action: crate::computer::Action = match serde_json::from_value(input.clone()) {
                    Ok(a) => a,
                    Err(e) => return Some(ToolResult::error(format!("Invalid tool input: {e}"), summary)),
                };
                if mode == PermissionMode::Plan && action.action != "screenshot" {
                    return Some(ToolResult::error("Plan mode is on: computer use is limited to screenshots.", summary));
                }
                if action.action != "wait" && mode != PermissionMode::Bypass && !grants.computer {
                    match self.ask(sink, id, "computer", &summary, &crate::computer::describe(&action)).await {
                        None => return Some(cancelled_result(summary)),
                        Some((Decision::Deny, fb)) => return Some(denied_result("action", summary, fb, None)),
                        Some((Decision::AllowSession, _)) => self.grant(|g| g.computer = true),
                        Some((Decision::Allow, _)) => {}
                    }
                }
                running(self, sink);
                let res = tokio::select! {
                    r = crate::computer::perform(&action) => r,
                    _ = self.cancel.cancelled() => return Some(cancelled_result(summary)),
                };
                match res {
                    Ok((text, shot)) => image_ui(ToolResult::done(text, summary, None).with_images(images_of(shot.as_ref())), shot.as_ref()),
                    Err(e) => ToolResult::error(e, summary),
                }
            }
        })
    }

    async fn skill_save(&mut self, id: &str, input: &Value, summary: String, mode: PermissionMode, granted: bool, sink: &mut Sink) -> ToolResult {
        let get = |k: &str| input[k].as_str().unwrap_or_default().trim().to_string();
        let (name, description, content) = (get("name"), get("description"), get("content"));
        if !skills::valid_name(&name) {
            return ToolResult::error("Skill names use letters, digits, '-' and '_' (max 64 characters).", summary);
        }
        if content.is_empty() {
            return ToolResult::error("The skill content is empty.", summary);
        }
        let scope = if get("scope") == "project" && self.ws.is_some() { "project" } else { "user" };
        let Some(dir) = self.extras.skills.dir_for(scope).map(|d| d.to_path_buf()) else { return ToolResult::error("No folder is open for a project skill.", summary) };
        let text = skills::render(&name, &description, &content);
        let file = dir.join(&name).join("SKILL.md");
        let old = std::fs::read_to_string(&file).ok();
        let rel = format!("{}skills/{name}/SKILL.md", if scope == "project" { ".pilunch/" } else { "~/.config/pilunch/" });
        let (diff, _, _) = super::tools::diff_with_stats(&rel, old.as_deref(), &text);
        if matches!(mode, PermissionMode::Ask | PermissionMode::Plan) && !granted {
            match self.ask(sink, id, "edit", &summary, &diff).await {
                None => return cancelled_result(summary),
                Some((Decision::Deny, fb)) => return denied_result("skill", summary, fb, Some(diff)),
                Some((Decision::AllowSession, _)) => self.grant(|g| g.edits = true),
                Some((Decision::Allow, _)) => {}
            }
        }
        match skills::write(&dir, &name, &text) {
            Ok(path) => {
                let verb = if old.is_some() { "Updated" } else { "Saved" };
                let mut r = ToolResult::done(format!("{verb} skill \"{name}\" at {}.", path.display()), summary, None);
                r.ui.detail = Some(diff);
                r.ui.detail_kind = Some("diff".into());
                r
            }
            Err(e) => ToolResult::error(format!("Couldn't save the skill: {e}"), summary),
        }
    }

    /// Tell the Browser panel to refresh after the agent used the browser.
    fn app_emit_browser(&self) -> tauri::Result<()> {
        use tauri::Emitter;
        self.app.emit("browser-changed", ())
    }
}

pub fn browser_options(s: &crate::settings::Settings) -> crate::browser::Options {
    crate::browser::Options { geckodriver: s.geckodriver_path.clone(), firefox: s.firefox_path.clone(), headless: s.browser_headless }
}

fn describe(tool: &ExtTool, name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match tool {
        ExtTool::SkillLoad => format!("Load skill {}", s("name")),
        ExtTool::SkillSave => format!("Save skill {}", s("name")),
        ExtTool::Custom(t) => format!("{} ({})", t.spec.name, crate::util::truncate_end(&t.spec.description, 60)),
        ExtTool::Mcp { client, tool, .. } => format!("{tool} · {} MCP", client.name),
        ExtTool::Browser => match s("action").as_str() {
            "navigate" => format!("Open {}", s("url")),
            "click" => match input.get("ref") {
                Some(r) => format!("Click [{r}] in the browser"),
                None => "Click in the browser".into(),
            },
            "type" => format!("Type “{}” in the browser", crate::util::truncate_end(&s("text"), 60)),
            a => format!("Browser: {a}"),
        },
        ExtTool::Computer => serde_json::from_value::<crate::computer::Action>(input.clone()).map(|a| crate::computer::describe(&a)).unwrap_or_else(|_| name.to_string()),
    }
}
