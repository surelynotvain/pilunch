//! The open folder ("workspace") and the path sandbox every file operation goes through.
//!
//! Both the UI and the AI agent address files by paths relative to the workspace root.
//! `Workspace::resolve` turns such a path into an absolute one and guarantees it stays
//! inside the root — rejecting `..` escapes, absolute paths elsewhere, and symlinks that
//! point outside.

use crate::error::{Error, Result};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let root = dunce::canonicalize(path.as_ref())
            .map_err(|e| Error::msg(format!("Cannot open {}: {e}", path.as_ref().display())))?;
        if !root.is_dir() {
            return Err(Error::msg(format!("{} is not a folder", root.display())));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    /// Resolve a relative (or absolute) path to an absolute path inside the workspace.
    /// The path does not need to exist (so it can be used for new files).
    pub fn resolve(&self, input: &str) -> Result<PathBuf> {
        let input = input.trim();
        let raw = Path::new(if input.is_empty() { "." } else { input });
        let joined = if raw.is_absolute() { raw.to_path_buf() } else { self.root.join(raw) };
        let normalized = normalize_lexically(&joined);
        if !normalized.starts_with(&self.root) {
            return Err(Error::OutsideWorkspace(input.to_string()));
        }
        // Symlinks: canonicalize the deepest existing ancestor and make sure it is still inside.
        let mut existing = normalized.as_path();
        loop {
            if existing.exists() {
                let real = dunce::canonicalize(existing)?;
                if !real.starts_with(&self.root) {
                    return Err(Error::OutsideWorkspace(input.to_string()));
                }
                break;
            }
            match existing.parent() {
                Some(p) => existing = p,
                None => break,
            }
        }
        Ok(normalized)
    }

    /// Workspace-relative path with forward slashes ("." for the root itself).
    pub fn relative(&self, abs: &Path) -> String {
        match abs.strip_prefix(&self.root) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
            Ok(rel) => to_slash(rel),
            Err(_) => abs.display().to_string(),
        }
    }
}

/// Collapse `.` and `..` without touching the filesystem.
fn normalize_lexically(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                // Never pop past the root/prefix.
                if !matches!(out.components().next_back(), Some(Component::RootDir | Component::Prefix(_)) | None) {
                    out.pop();
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn to_slash(p: &Path) -> String {
    let s = p.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '\\' {
        s.replace('\\', "/")
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let w = Workspace::open(dir.path()).unwrap();
        (dir, w)
    }

    #[test]
    fn resolves_relative_paths_inside_root() {
        let (_d, w) = ws();
        let p = w.resolve("src/main.rs").unwrap();
        assert!(p.ends_with("src/main.rs"));
        assert_eq!(w.relative(&p), "src/main.rs");
        assert_eq!(w.relative(&w.resolve(".").unwrap()), ".");
        assert_eq!(w.relative(&w.resolve("").unwrap()), ".");
        // new files are allowed
        assert!(w.resolve("src/new/file.txt").is_ok());
        // `..` that stays inside is fine
        assert!(w.resolve("src/nested/../main.rs").unwrap().ends_with("src/main.rs"));
    }

    #[test]
    fn rejects_escapes() {
        let (_d, w) = ws();
        assert!(matches!(w.resolve("../etc/passwd"), Err(Error::OutsideWorkspace(_))));
        assert!(matches!(w.resolve("src/../../x"), Err(Error::OutsideWorkspace(_))));
        assert!(matches!(w.resolve("/etc/passwd"), Err(Error::OutsideWorkspace(_))));
        // absolute path inside the root is accepted
        let abs = w.root().join("src/main.rs");
        assert!(w.resolve(abs.to_str().unwrap()).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let (d, w) = ws();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("link")).unwrap();
        assert!(matches!(w.resolve("link/secret"), Err(Error::OutsideWorkspace(_))));
        assert!(matches!(w.resolve("link"), Err(Error::OutsideWorkspace(_))));
    }
}
