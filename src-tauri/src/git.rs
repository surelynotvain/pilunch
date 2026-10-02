//! Lightweight git integration via the `git` CLI (no libgit2 build dependency).

use crate::process::command;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Workspace-relative path → status letter: M (modified), A (added), D (deleted),
    /// R (renamed), U (untracked), C (conflict).
    pub files: HashMap<String, char>,
}

pub async fn status(root: &Path) -> Option<GitStatus> {
    let out = command("git")
        .args(["status", "--porcelain=v1", "-b", "-z", "--untracked-files=all"])
        .current_dir(root)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // Paths are relative to the repo root; map them onto the workspace when it is a subfolder.
    let prefix = command("git")
        .args(["rev-parse", "--show-prefix"])
        .current_dir(root)
        .output()
        .await
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    Some(parse_porcelain(&String::from_utf8_lossy(&out.stdout), &prefix))
}

pub fn parse_porcelain(s: &str, prefix: &str) -> GitStatus {
    let mut st = GitStatus::default();
    let mut parts = s.split('\0').filter(|p| !p.is_empty());
    while let Some(entry) = parts.next() {
        if let Some(head) = entry.strip_prefix("## ") {
            parse_branch(head, &mut st);
            continue;
        }
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let (x, y) = (code.as_bytes()[0] as char, code.as_bytes()[1] as char);
        if x == 'R' || x == 'C' {
            parts.next(); // the original path of a rename/copy
        }
        let letter = match (x, y) {
            ('?', '?') => 'U',
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => 'C',
            ('A', _) => 'A',
            ('R', _) => 'R',
            ('D', _) | (_, 'D') => 'D',
            _ => 'M',
        };
        let Some(rel) = path.strip_prefix(prefix) else { continue };
        let rel = rel.trim_end_matches('/');
        st.files.insert(rel.to_string(), letter);
    }
    st
}

fn parse_branch(head: &str, st: &mut GitStatus) {
    let (name_part, tracking) = match head.find(" [") {
        Some(i) => (&head[..i], Some(&head[i + 2..head.len().saturating_sub(1)])),
        None => (head, None),
    };
    let name = if let Some(rest) = name_part.strip_prefix("No commits yet on ") {
        rest
    } else if name_part.starts_with("HEAD (no branch)") {
        "HEAD"
    } else {
        name_part.split("...").next().unwrap_or(name_part)
    };
    st.branch = Some(name.to_string());
    if let Some(t) = tracking {
        for item in t.split(", ") {
            if let Some(n) = item.strip_prefix("ahead ") {
                st.ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = item.strip_prefix("behind ") {
                st.behind = n.parse().unwrap_or(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v1_z() {
        let raw = "## main...origin/main [ahead 2, behind 1]\0 M src/a.rs\0?? new.txt\0R  b2.rs\0b.rs\0A  added.rs\0 D gone.rs\0UU conflict.rs\0";
        let st = parse_porcelain(raw, "");
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!((st.ahead, st.behind), (2, 1));
        assert_eq!(st.files["src/a.rs"], 'M');
        assert_eq!(st.files["new.txt"], 'U');
        assert_eq!(st.files["b2.rs"], 'R');
        assert!(!st.files.contains_key("b.rs"));
        assert_eq!(st.files["added.rs"], 'A');
        assert_eq!(st.files["gone.rs"], 'D');
        assert_eq!(st.files["conflict.rs"], 'C');
    }

    #[test]
    fn parses_branch_variants_and_prefix() {
        let st = parse_porcelain("## No commits yet on dev\0?? app/x.ts\0?? other/y.ts\0", "app/");
        assert_eq!(st.branch.as_deref(), Some("dev"));
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files["x.ts"], 'U');
        let st = parse_porcelain("## feature\0", "");
        assert_eq!(st.branch.as_deref(), Some("feature"));
    }

    #[tokio::test]
    async fn real_git_repo() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(d.path()).output().unwrap();
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(d.path().join("f.txt"), "x").unwrap();
        let st = status(d.path()).await.unwrap();
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.files["f.txt"], 'U');
        let not_repo = tempfile::tempdir().unwrap();
        // A temp dir outside any repo has no status (unless /tmp itself is in a repo).
        let _ = status(not_repo.path()).await;
    }
}
