//! Ways to extend the agent: skills, custom tools, MCP servers and plugins (which bundle
//! the other three; Rust extensions are plugins built from a Rust crate).

pub mod custom_tools;
pub mod mcp;
pub mod plugins;
pub mod skills;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Everything the extension sources provide for one run (re-read at the start of each run,
/// so edits apply to the next message without a restart).
pub struct Loaded {
    pub skills: skills::SkillDirs,
    pub tools: Vec<custom_tools::CustomTool>,
    pub tool_errors: Vec<String>,
    pub servers: BTreeMap<String, mcp::Source>,
}

pub fn load(config_dir: &Path, project: Option<&Path>, disabled_plugins: &[String]) -> Loaded {
    let plugins = plugins::list(config_dir, disabled_plugins);
    let enabled: Vec<&plugins::Plugin> = plugins.iter().filter(|p| p.enabled).collect();
    let plugin_dirs: Vec<(String, PathBuf)> = enabled.iter().map(|p| (p.id.clone(), p.dir.clone())).collect();

    let mut tool_sources: Vec<(String, PathBuf, Option<PathBuf>)> = Vec::new();
    if let Some(p) = project {
        tool_sources.push(("project".into(), p.join(".pilunch").join("tools"), None));
    }
    tool_sources.push(("user".into(), config_dir.join("tools"), None));
    for p in &enabled {
        tool_sources.push((format!("plugin:{}", p.id), p.dir.join("tools"), Some(p.dir.clone())));
    }
    let (tools, tool_errors) = custom_tools::load(&tool_sources);

    let mut servers: BTreeMap<String, mcp::Source> = BTreeMap::new();
    for (name, config) in mcp::McpFile::load(config_dir).servers {
        servers.insert(name, mcp::Source { config, origin: "mcp.json".into(), plugin_dir: None });
    }
    for p in &enabled {
        for (name, config) in &p.manifest.mcp_servers {
            servers.entry(name.clone()).or_insert_with(|| mcp::Source { config: config.clone(), origin: format!("plugin:{}", p.id), plugin_dir: Some(p.dir.clone()) });
        }
    }

    Loaded { skills: skills::SkillDirs::new(config_dir, project, &plugin_dirs), tools, tool_errors, servers }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_contribute_skills_tools_and_servers() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path();
        let p = cfg.join("plugins/kit");
        std::fs::create_dir_all(p.join("tools")).unwrap();
        std::fs::write(p.join("tools/greet.json"), r#"{"name":"greet","description":"Greet","command":"echo hi"}"#).unwrap();
        skills::write(&p.join("skills"), "style", &skills::render("style", "House style", "Use tabs")).unwrap();
        std::fs::write(p.join("plugin.json"), r#"{"name":"Kit","mcpServers":{"kit":{"command":"${PLUGIN_DIR}/bin/kit"}}}"#).unwrap();
        let l = load(cfg, None, &[]);
        assert_eq!(l.tools.len(), 1);
        assert_eq!(l.tools[0].plugin_dir.as_deref(), Some(p.as_path()));
        assert_eq!(l.skills.list()[0].scope, "plugin:kit");
        assert_eq!(l.servers["kit"].origin, "plugin:kit");
        let off = load(cfg, None, &["kit".into()]);
        assert!(off.tools.is_empty() && off.servers.is_empty() && off.skills.list().is_empty());
    }
}
