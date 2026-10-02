//! File operations used by the editor UI. Every path is resolved through the workspace sandbox.

use crate::error::{Error, Result};
use crate::util::{atomic_write, looks_binary};
use crate::workspace::Workspace;
use serde::Serialize;
use std::cmp::Ordering;

/// Largest file the editor will open.
pub const MAX_EDITOR_FILE: u64 = 20 * 1024 * 1024;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryInfo {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
}

/// One directory level, directories first, then natural case-insensitive order.
/// The explorer loads children lazily, so even huge trees cost one `read_dir` per expand.
pub fn list_dir(ws: &Workspace, rel: &str) -> Result<Vec<DirEntryInfo>> {
    let dir = ws.resolve(rel)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let is_symlink = ft.is_symlink();
        // Follow symlinks for the dir/file decision (target may be outside; resolve() guards access).
        let is_dir = if is_symlink { entry.path().is_dir() } else { ft.is_dir() };
        out.push(DirEntryInfo { path: ws.relative(&entry.path()), name, is_dir, is_symlink });
    }
    out.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => natural_cmp(&a.name, &b.name),
    });
    Ok(out)
}

/// Case-insensitive comparison where digit runs compare numerically ("file2" < "file10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub path: String,
    pub content: Option<String>,
    pub size: u64,
    pub binary: bool,
    pub too_large: bool,
    /// Not valid UTF-8: shown read-only so saving can't corrupt it.
    pub readonly: bool,
}

pub fn read_file(ws: &Workspace, rel: &str) -> Result<FileContent> {
    let path = ws.resolve(rel)?;
    let meta = std::fs::metadata(&path)?;
    if meta.is_dir() {
        return Err(Error::msg(format!("{rel} is a folder")));
    }
    let rel = ws.relative(&path);
    let size = meta.len();
    if size > MAX_EDITOR_FILE {
        return Ok(FileContent { path: rel, content: None, size, binary: false, too_large: true, readonly: true });
    }
    let bytes = std::fs::read(&path)?;
    if looks_binary(&bytes) {
        return Ok(FileContent { path: rel, content: None, size, binary: true, too_large: false, readonly: true });
    }
    let (content, readonly) = match String::from_utf8(bytes) {
        Ok(s) => (s, false),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
    };
    Ok(FileContent { path: rel, content: Some(content), size, binary: false, too_large: false, readonly })
}

pub fn write_file(ws: &Workspace, rel: &str, content: &str) -> Result<()> {
    let path = ws.resolve(rel)?;
    if path.is_dir() {
        return Err(Error::msg(format!("{rel} is a folder")));
    }
    atomic_write(&path, content.as_bytes())?;
    Ok(())
}

pub fn create_file(ws: &Workspace, rel: &str) -> Result<String> {
    let path = ws.resolve(rel)?;
    if path.exists() {
        return Err(Error::msg(format!("{rel} already exists")));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::File::create(&path)?;
    Ok(ws.relative(&path))
}

pub fn create_dir(ws: &Workspace, rel: &str) -> Result<String> {
    let path = ws.resolve(rel)?;
    if path.exists() {
        return Err(Error::msg(format!("{rel} already exists")));
    }
    std::fs::create_dir_all(&path)?;
    Ok(ws.relative(&path))
}

pub fn rename(ws: &Workspace, from: &str, to: &str) -> Result<String> {
    let src = ws.resolve(from)?;
    let dst = ws.resolve(to)?;
    if src == ws.root() {
        return Err(Error::msg("cannot rename the workspace root"));
    }
    if dst.exists() {
        return Err(Error::msg(format!("{to} already exists")));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&src, &dst)?;
    Ok(ws.relative(&dst))
}

/// Move to the system trash; with `permanent` delete outright (used when no trash is available).
pub fn delete(ws: &Workspace, rel: &str, permanent: bool) -> Result<()> {
    let path = ws.resolve(rel)?;
    if path == ws.root() {
        return Err(Error::msg("cannot delete the workspace root"));
    }
    if !path.exists() && !path.is_symlink() {
        return Err(Error::msg(format!("{rel} does not exist")));
    }
    if permanent {
        if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    } else {
        trash::delete(&path).map_err(|e| Error::msg(format!("Could not move to trash: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a", "B"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a", "B", "file1", "File2", "file10"]);
    }

    #[test]
    fn crud_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::open(d.path()).unwrap();
        assert_eq!(create_dir(&ws, "src").unwrap(), "src");
        assert_eq!(create_file(&ws, "src/a.txt").unwrap(), "src/a.txt");
        assert!(create_file(&ws, "src/a.txt").is_err());
        write_file(&ws, "src/a.txt", "hello").unwrap();
        let f = read_file(&ws, "src/a.txt").unwrap();
        assert_eq!(f.content.as_deref(), Some("hello"));
        assert!(!f.readonly);
        assert_eq!(rename(&ws, "src/a.txt", "src/b.txt").unwrap(), "src/b.txt");
        let listing = list_dir(&ws, "src").unwrap();
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].path, "src/b.txt");
        delete(&ws, "src/b.txt", true).unwrap();
        assert!(list_dir(&ws, "src").unwrap().is_empty());
        assert!(delete(&ws, ".", true).is_err());
        assert!(write_file(&ws, "../escape.txt", "x").is_err());
    }

    #[test]
    fn binary_and_non_utf8() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::open(d.path()).unwrap();
        std::fs::write(d.path().join("img.bin"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(d.path().join("latin1.txt"), [b'c', b'a', b'f', 0xE9]).unwrap();
        assert!(read_file(&ws, "img.bin").unwrap().binary);
        let f = read_file(&ws, "latin1.txt").unwrap();
        assert!(f.readonly);
        assert!(f.content.unwrap().starts_with("caf"));
    }
}
