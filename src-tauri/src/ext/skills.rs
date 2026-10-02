//! Skills: reusable Markdown instructions the agent loads on demand — and writes itself.
//!
//! A skill is a folder with a `SKILL.md`:
//!
//! ```text
//! ---
//! name: release-checklist
//! description: How to cut a release of this project
//! ---
//! 1. Bump the version in …
//! ```
//!
//! Skills live in `~/.config/pilunch/skills/` (yours, for every project), in
//! `<project>/.pilunch/skills/` (shared with the repo) and inside plugins. The system
//! prompt lists each skill's name and description; the agent reads the full text with
//! `skill_load` and saves what it learns (e.g. after researching an API) with `skill_save`.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// "user", "project" or "plugin:<name>"
    pub scope: String,
    pub path: String,
}

/// Where skills are looked up, highest priority first (a name found earlier wins).
pub struct SkillDirs {
    pub dirs: Vec<(String, PathBuf)>,
}

impl SkillDirs {
    pub fn new(config_dir: &Path, project: Option<&Path>, plugins: &[(String, PathBuf)]) -> Self {
        let mut dirs = Vec::new();
        if let Some(p) = project {
            dirs.push(("project".to_string(), p.join(".pilunch").join("skills")));
        }
        dirs.push(("user".to_string(), config_dir.join("skills")));
        for (name, dir) in plugins {
            dirs.push((format!("plugin:{name}"), dir.join("skills")));
        }
        Self { dirs }
    }

    pub fn list(&self) -> Vec<Skill> {
        let mut out: Vec<Skill> = Vec::new();
        for (scope, dir) in &self.dirs {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
            entries.sort();
            for d in entries {
                let file = d.join("SKILL.md");
                let Ok(text) = std::fs::read_to_string(&file) else { continue };
                let folder = d.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
                let (meta, _) = parse(&text);
                let name = meta.name.filter(|n| valid_name(n)).unwrap_or(folder);
                if out.iter().any(|s| s.name == name) {
                    continue;
                }
                out.push(Skill { name, description: meta.description.unwrap_or_default(), scope: scope.clone(), path: file.display().to_string() });
            }
        }
        out
    }

    /// Directory a new skill with this scope is written to.
    pub fn dir_for(&self, scope: &str) -> Option<&Path> {
        self.dirs.iter().find(|(s, _)| s == scope).map(|(_, d)| d.as_path())
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Meta {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Split YAML-ish front matter (`key: value` lines between `---`) from the body.
pub fn parse(text: &str) -> (Meta, &str) {
    let mut meta = Meta::default();
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = t.strip_prefix("---\n").or_else(|| t.strip_prefix("---\r\n")) else { return (meta, t) };
    let Some(end) = rest.find("\n---") else { return (meta, t) };
    for line in rest[..end].lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            match k.trim() {
                "name" => meta.name = Some(v),
                "description" => meta.description = Some(v),
                _ => {}
            }
        }
    }
    let body = &rest[end + 4..];
    (meta, body.strip_prefix('\n').or_else(|| body.strip_prefix("\r\n")).unwrap_or(body))
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The full SKILL.md text for a skill.
pub fn render(name: &str, description: &str, body: &str) -> String {
    let description = description.replace(['\n', '\r'], " ");
    format!("---\nname: {name}\ndescription: {}\n---\n\n{}\n", description.trim(), body.trim())
}

/// Write (create or replace) a skill; returns the SKILL.md path.
pub fn write(dir: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    if !valid_name(name) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "skill names use letters, digits, '-' and '_' (max 64)"));
    }
    let folder = dir.join(name);
    std::fs::create_dir_all(&folder)?;
    let file = folder.join("SKILL.md");
    crate::util::atomic_write(&file, text.as_bytes())?;
    Ok(file)
}

/// The system-prompt section listing available skills (empty when there are none).
pub fn prompt_section(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return "\n# Skills\nNo skills are saved yet. When you work out a reusable procedure (a project's release steps, how an \
API works after researching it), save it with skill_save so you can reuse it in later conversations.\n"
            .into();
    }
    let mut s = String::from(
        "\n# Skills\nSkills are saved instructions for recurring tasks. Before a task that matches a skill's description, \
read it with skill_load and follow it. When you learn something reusable (after research, or a procedure that took \
several attempts), save or improve a skill with skill_save.\n",
    );
    for sk in skills {
        s.push_str(&format!("- {}: {}\n", sk.name, if sk.description.is_empty() { "(no description)" } else { &sk.description }));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_is_parsed() {
        let (m, body) = parse("---\nname: deploy\ndescription: \"Ship it\"\n---\n\nStep 1\n");
        assert_eq!(m, Meta { name: Some("deploy".into()), description: Some("Ship it".into()) });
        assert_eq!(body, "\nStep 1\n");
        let (m, body) = parse("no front matter");
        assert_eq!(m, Meta::default());
        assert_eq!(body, "no front matter");
    }

    #[test]
    fn skills_are_listed_with_priority_and_written() {
        let d = tempfile::tempdir().unwrap();
        let (cfg, proj, plug) = (d.path().join("cfg"), d.path().join("proj"), d.path().join("plug"));
        let dirs = SkillDirs::new(&cfg, Some(&proj), &[("tools".into(), plug.clone())]);
        write(&cfg.join("skills"), "deploy", &render("deploy", "user version", "u")).unwrap();
        write(&proj.join(".pilunch/skills"), "deploy", &render("deploy", "project version", "p")).unwrap();
        write(&plug.join("skills"), "lint", &render("lint", "from plugin", "l")).unwrap();
        let list = dirs.list();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].name.as_str(), list[0].scope.as_str(), list[0].description.as_str()), ("deploy", "project", "project version"));
        assert_eq!(list[1].scope, "plugin:tools");
        assert!(write(&cfg, "../evil", "x").is_err());
        let p = prompt_section(&list);
        assert!(p.contains("- deploy: project version") && p.contains("- lint: from plugin"));
        assert!(prompt_section(&[]).contains("No skills are saved yet"));
        assert_eq!(dirs.dir_for("user").unwrap(), cfg.join("skills"));
    }
}
