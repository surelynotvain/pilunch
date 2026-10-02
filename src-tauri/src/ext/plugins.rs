//! Plugins: folders in `~/.config/pilunch/plugins/` that bundle skills, custom tools and
//! MCP servers. A Rust extension is a plugin whose MCP server is a Rust binary built from
//! source with `cargo` (see docs/extensions.md and the `pilunch-extension` crate).
//!
//! ```text
//! plugins/my-plugin/
//!   plugin.json        { "name", "version", "description", "mcpServers": { … } }
//!   skills/<name>/SKILL.md
//!   tools/<name>.json
//!   bin/               (Rust extensions: the built server)
//! ```
//!
//! In `plugin.json`, `${PLUGIN_DIR}` expands to the plugin's folder.

use super::mcp::ServerConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(rename = "mcpServers")]
    pub mcp_servers: BTreeMap<String, ServerConfig>,
    /// Set for Rust extensions: where it was built from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<ExtensionInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtensionInfo {
    pub language: String,
    pub source: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    /// Folder name (the plugin's id).
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub path: String,
    pub enabled: bool,
    /// "plugin" or "rust-extension"
    pub kind: String,
    pub skills: usize,
    pub tools: usize,
    pub servers: Vec<String>,
    pub error: Option<String>,
    #[serde(skip)]
    pub manifest: Manifest,
    #[serde(skip)]
    pub dir: PathBuf,
}

pub fn plugins_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("plugins")
}

fn count(dir: &Path, f: impl Fn(&Path) -> bool) -> usize {
    std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| f(&e.path())).count()
}

pub fn list(config_dir: &Path, disabled: &[String]) -> Vec<Plugin> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(plugins_dir(config_dir)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.into_iter()
        .map(|dir| {
            let id = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            let (manifest, error) = match std::fs::read(dir.join("plugin.json")) {
                Ok(b) => match serde_json::from_slice::<Manifest>(&b) {
                    Ok(m) => (m, None),
                    Err(e) => (Manifest::default(), Some(format!("plugin.json: {e}"))),
                },
                Err(_) => (Manifest::default(), None),
            };
            Plugin {
                name: if manifest.name.is_empty() { id.clone() } else { manifest.name.clone() },
                version: manifest.version.clone(),
                description: manifest.description.clone(),
                path: dir.display().to_string(),
                enabled: !disabled.contains(&id) && error.is_none(),
                kind: if manifest.extension.as_ref().is_some_and(|e| e.language == "rust") { "rust-extension".into() } else { "plugin".into() },
                skills: count(&dir.join("skills"), |p| p.join("SKILL.md").is_file()),
                tools: count(&dir.join("tools"), |p| p.extension().is_some_and(|x| x == "json")),
                servers: manifest.mcp_servers.keys().cloned().collect(),
                error,
                id,
                manifest,
                dir,
            }
        })
        .collect()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') && !id.starts_with('.')
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        if matches!(name.to_str(), Some(".git" | "target" | "node_modules")) {
            continue;
        }
        let ty = e.file_type()?;
        if ty.is_dir() {
            copy_dir(&e.path(), &to.join(&name))?;
        } else if ty.is_file() {
            std::fs::copy(e.path(), to.join(&name))?;
        }
    }
    Ok(())
}

/// Install a plugin by copying a folder (it should contain plugin.json, skills/ or tools/).
pub fn install_folder(config_dir: &Path, src: &Path) -> Result<String, String> {
    if !src.is_dir() {
        return Err(format!("{} is not a folder", src.display()));
    }
    let has_content = src.join("plugin.json").is_file() || src.join("skills").is_dir() || src.join("tools").is_dir();
    if !has_content {
        return Err("That folder has no plugin.json, skills/ or tools/. For a Rust extension, use “Build Rust extension”.".into());
    }
    let id = src.file_name().and_then(|n| n.to_str()).filter(|n| valid_id(n)).ok_or("unsupported folder name")?.to_string();
    let dest = plugins_dir(config_dir).join(&id);
    if dest.exists() {
        return Err(format!("A plugin named \"{id}\" is already installed. Remove it first."));
    }
    copy_dir(src, &dest).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Install a plugin from a git repository (shallow clone).
pub async fn install_git(config_dir: &Path, url: &str) -> Result<String, String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("git@") || url.starts_with("ssh://")) {
        return Err("Use an https:// or ssh git URL".into());
    }
    let id = url.trim_end_matches('/').trim_end_matches(".git").rsplit(['/', ':']).next().filter(|n| valid_id(n)).ok_or("can't derive a plugin name from that URL")?.to_string();
    let dest = plugins_dir(config_dir).join(&id);
    if dest.exists() {
        return Err(format!("A plugin named \"{id}\" is already installed. Remove it first."));
    }
    std::fs::create_dir_all(plugins_dir(config_dir)).map_err(|e| e.to_string())?;
    let out = crate::process::command("git")
        .args(["clone", "--depth", "1", "--", url])
        .arg(&dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|e| format!("couldn't run git: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(format!("git clone failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(id)
}

pub fn remove(config_dir: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("invalid plugin id".into());
    }
    let dir = plugins_dir(config_dir).join(id);
    std::fs::remove_dir_all(&dir).map_err(|e| format!("couldn't remove {}: {e}", dir.display()))
}

/// The `[package] name` from a Cargo.toml (enough for building an extension).
fn cargo_package_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_package = l == "[package]";
            continue;
        }
        if in_package {
            if let Some(v) = l.strip_prefix("name").map(str::trim_start).and_then(|r| r.strip_prefix('=')) {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Build a Rust extension with `cargo build --release` and install it as a plugin whose
/// MCP server is the built binary. Skills, tools and plugin.json next to Cargo.toml are
/// copied along. `log` receives cargo's output.
pub async fn build_rust_extension(config_dir: &Path, src: &Path, mut log: impl FnMut(&str)) -> Result<String, String> {
    let toml = std::fs::read_to_string(src.join("Cargo.toml")).map_err(|_| format!("No Cargo.toml in {}", src.display()))?;
    let pkg = cargo_package_name(&toml).ok_or("Cargo.toml has no [package] name")?;
    if !valid_id(&pkg) {
        return Err(format!("unsupported package name {pkg}"));
    }
    log(&format!("Building {pkg} with cargo build --release…\n"));
    let out = crate::process::command("cargo")
        .args(["build", "--release", "--manifest-path"])
        .arg(src.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(src.join("target"))
        .output()
        .await
        .map_err(|e| format!("couldn't run cargo (is Rust installed? https://rustup.rs): {e}"))?;
    log(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return Err("cargo build failed (see the log)".into());
    }
    let exe = format!("{pkg}{}", std::env::consts::EXE_SUFFIX);
    let built = src.join("target/release").join(&exe);
    if !built.is_file() {
        return Err(format!("built binary {exe} not found under target/release (is it a binary crate?)"));
    }
    let dest = plugins_dir(config_dir).join(&pkg);
    let mut manifest: Manifest = std::fs::read(src.join("plugin.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(dest.join("bin")).map_err(|e| e.to_string())?;
    std::fs::copy(&built, dest.join("bin").join(&exe)).map_err(|e| e.to_string())?;
    for sub in ["skills", "tools"] {
        if src.join(sub).is_dir() {
            copy_dir(&src.join(sub), &dest.join(sub)).map_err(|e| e.to_string())?;
        }
    }
    if manifest.name.is_empty() {
        manifest.name = pkg.clone();
    }
    if manifest.mcp_servers.is_empty() {
        manifest.mcp_servers.insert(pkg.clone(), ServerConfig { command: format!("${{PLUGIN_DIR}}/bin/{exe}"), ..Default::default() });
    }
    manifest.extension = Some(ExtensionInfo { language: "rust".into(), source: src.display().to_string() });
    let data = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(dest.join("plugin.json"), data).map_err(|e| e.to_string())?;
    log(&format!("Installed {pkg} to {}\n", dest.display()));
    Ok(pkg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_are_listed_installed_and_removed() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("cfg");
        let src = d.path().join("my-plugin");
        std::fs::create_dir_all(src.join("skills/s1")).unwrap();
        std::fs::write(src.join("skills/s1/SKILL.md"), "x").unwrap();
        std::fs::create_dir_all(src.join("tools")).unwrap();
        std::fs::write(src.join("tools/t.json"), "{}").unwrap();
        std::fs::create_dir_all(src.join(".git")).unwrap();
        std::fs::write(src.join("plugin.json"), r#"{"name":"My Plugin","version":"1.0","mcpServers":{"srv":{"command":"${PLUGIN_DIR}/bin/x"}}}"#).unwrap();
        assert_eq!(install_folder(&cfg, &src).unwrap(), "my-plugin");
        assert!(install_folder(&cfg, &src).is_err(), "already installed");
        assert!(!plugins_dir(&cfg).join("my-plugin/.git").exists());
        let list = list(&cfg, &[]);
        assert_eq!(list.len(), 1);
        let p = &list[0];
        assert_eq!((p.name.as_str(), p.skills, p.tools, p.enabled, p.kind.as_str()), ("My Plugin", 1, 1, true, "plugin"));
        assert_eq!(p.servers, vec!["srv"]);
        assert!(!super::list(&cfg, &["my-plugin".into()])[0].enabled);
        remove(&cfg, "my-plugin").unwrap();
        assert!(super::list(&cfg, &[]).is_empty());
        assert!(remove(&cfg, "../x").is_err());
        let empty = d.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(install_folder(&cfg, &empty).is_err());
    }

    #[test]
    fn reads_cargo_package_name() {
        assert_eq!(cargo_package_name("[package]\nname = \"hello-ext\"\nversion = \"0.1.0\"\n[dependencies]\nname = \"no\"").as_deref(), Some("hello-ext"));
        assert_eq!(cargo_package_name("[workspace]\nmembers=[]"), None);
    }

    /// Builds extensions/examples/hello-ext with cargo and talks to it over MCP. Slow, so
    /// run explicitly: `cargo test -- --ignored rust_extension`.
    #[test]
    #[ignore]
    fn rust_extension_builds_installs_and_serves_tools() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../extensions/examples/hello-ext");
        let d = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut log = String::new();
            let id = build_rust_extension(d.path(), &src, |s| log.push_str(s)).await.unwrap_or_else(|e| panic!("{e}\n{log}"));
            assert_eq!(id, "hello-ext");
            let p = &list(d.path(), &[])[0];
            assert_eq!((p.kind.as_str(), p.skills), ("rust-extension", 1));
            let loaded = crate::ext::load(d.path(), None, &[]);
            let mgr = crate::ext::mcp::McpManager::default();
            let clients = mgr.ensure(&loaded.servers, &reqwest::Client::new(), d.path()).await;
            assert_eq!(clients.len(), 1, "{:?}", mgr.status());
            let out = clients[0].call("word_count", serde_json::json!({"text":"a b c"})).await.unwrap();
            assert_eq!(out.text, "3 words, 1 lines, 5 characters");
        });
    }
}
