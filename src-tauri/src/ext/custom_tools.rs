//! Custom tools: shell commands described in JSON, offered to the agent like built-in tools.
//!
//! `~/.config/pilunch/tools/<name>.json`, `<project>/.pilunch/tools/<name>.json` or a
//! plugin's `tools/` folder:
//!
//! ```json
//! {
//!   "name": "deploy_preview",
//!   "description": "Deploy the current branch to a preview environment and print its URL.",
//!   "parameters": { "type": "object", "properties": { "env": { "type": "string" } }, "required": ["env"] },
//!   "command": "./scripts/deploy.sh {{env}}",
//!   "timeout": 300
//! }
//! ```
//!
//! `{{param}}` placeholders are replaced with shell-quoted values. The command also gets
//! every input as `PILUNCH_ARG_<NAME>` and the whole input as JSON in `PILUNCH_INPUT`. It
//! runs in the project folder and, like `run_command`, asks for approval first.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(default = "empty_schema")]
    pub parameters: Value,
    pub command: String,
    #[serde(default)]
    pub timeout: Option<u64>,
}

fn empty_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomTool {
    #[serde(flatten)]
    pub spec: ToolSpec,
    /// "user", "project" or "plugin:<name>"
    pub scope: String,
    pub path: String,
    #[serde(skip)]
    pub plugin_dir: Option<PathBuf>,
}

/// Names reserved for built-in and generated tools.
pub fn reserved(name: &str) -> bool {
    crate::agent::tools::class_of(name).is_some()
        || name.starts_with("mcp__")
        || name.starts_with("skill_")
        || matches!(name, "browser" | "computer" | "web_search")
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Load tools from `(scope, dir, plugin_dir)` sources, highest priority first.
pub fn load(sources: &[(String, PathBuf, Option<PathBuf>)]) -> (Vec<CustomTool>, Vec<String>) {
    let mut tools: Vec<CustomTool> = Vec::new();
    let mut errors = Vec::new();
    for (scope, dir, plugin_dir) in sources {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for f in files {
            let spec = match std::fs::read(&f).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<ToolSpec>(&b).map_err(|e| e.to_string())) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("{}: {e}", f.display()));
                    continue;
                }
            };
            if !valid_name(&spec.name) || reserved(&spec.name) {
                errors.push(format!("{}: invalid or reserved tool name \"{}\"", f.display(), spec.name));
                continue;
            }
            if tools.iter().any(|t| t.spec.name == spec.name) {
                continue;
            }
            tools.push(CustomTool { spec, scope: scope.clone(), path: f.display().to_string(), plugin_dir: plugin_dir.clone() });
        }
    }
    (tools, errors)
}

pub fn definition(t: &CustomTool) -> Value {
    let mut schema = t.spec.parameters.clone();
    if !schema.is_object() {
        schema = empty_schema();
    }
    if schema.get("type").is_none() {
        schema["type"] = json!("object");
    }
    json!({ "name": t.spec.name, "description": t.spec.description, "input_schema": schema })
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Quote a value for the platform shell used by `run_command`.
pub fn shell_quote(s: &str) -> String {
    if cfg!(windows) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Replace `{{key}}` placeholders with quoted input values (missing keys become '').
pub fn expand(command: &str, input: &Value) -> String {
    let mut out = String::with_capacity(command.len());
    let mut rest = command;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start + 2..].find("}}") else { break };
        out.push_str(&rest[..start]);
        let key = rest[start + 2..start + 2 + len].trim();
        out.push_str(&shell_quote(&input.get(key).map(as_text).unwrap_or_default()));
        rest = &rest[start + 4 + len..];
    }
    out.push_str(rest);
    out
}

pub fn env(input: &Value, plugin_dir: Option<&Path>) -> Vec<(String, String)> {
    let mut env = vec![("PILUNCH_INPUT".to_string(), input.to_string())];
    if let Some(obj) = input.as_object() {
        for (k, v) in obj {
            let key: String = k.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
            env.push((format!("PILUNCH_ARG_{key}"), as_text(v)));
        }
    }
    if let Some(d) = plugin_dir {
        env.push(("PILUNCH_PLUGIN_DIR".into(), d.display().to_string()));
    }
    env
}

/// Save a tool spec as `<dir>/<name>.json`.
pub fn save(dir: &Path, spec: &ToolSpec) -> Result<PathBuf, String> {
    if !valid_name(&spec.name) || reserved(&spec.name) {
        return Err(format!("\"{}\" is not a valid tool name (letters, digits, '_' and '-'; not a built-in tool)", spec.name));
    }
    if spec.command.trim().is_empty() {
        return Err("the command is empty".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", spec.name));
    let data = serde_json::to_vec_pretty(spec).map_err(|e| e.to_string())?;
    crate::util::atomic_write(&path, &data).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_quoted() {
        let input = json!({"env":"prod; rm -rf /","n":3});
        let cmd = expand("deploy {{env}} --n {{ n }} {{missing}}", &input);
        if cfg!(windows) {
            assert_eq!(cmd, "deploy 'prod; rm -rf /' --n '3' ''");
        } else {
            assert_eq!(cmd, "deploy 'prod; rm -rf /' --n '3' ''");
            assert_eq!(expand("echo {{q}}", &json!({"q":"it's"})), r"echo 'it'\''s'");
        }
        let e = env(&input, None);
        assert!(e.contains(&("PILUNCH_ARG_ENV".into(), "prod; rm -rf /".into())));
        assert!(e.iter().any(|(k, v)| k == "PILUNCH_INPUT" && v.contains("prod")));
    }

    #[test]
    fn tools_load_validate_and_save() {
        let d = tempfile::tempdir().unwrap();
        let spec = ToolSpec { name: "hello".into(), description: "Say hi".into(), parameters: json!({"properties":{"who":{"type":"string"}}}), command: "echo hi {{who}}".into(), timeout: None };
        save(d.path(), &spec).unwrap();
        std::fs::write(d.path().join("bad.json"), r#"{"name":"read_file","description":"x","command":"x"}"#).unwrap();
        std::fs::write(d.path().join("broken.json"), "{").unwrap();
        let (tools, errors) = load(&[("user".into(), d.path().to_path_buf(), None)]);
        assert_eq!(tools.len(), 1);
        assert_eq!(errors.len(), 2);
        let def = definition(&tools[0]);
        assert_eq!(def["input_schema"]["type"], "object");
        assert_eq!(def["name"], "hello");
        assert!(save(d.path(), &ToolSpec { name: "mcp__x".into(), ..spec.clone() }).is_err());
    }
}
