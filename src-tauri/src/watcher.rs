//! Debounced filesystem watcher for the open workspace.
//!
//! Emits `fs-changed` to the UI (so open editors and the explorer refresh when the agent,
//! a build, or another program changes files) and marks the quick-open index dirty.
//! Changes under gitignored paths (e.g. `target/`, `dist/`) are dropped so builds don't
//! flood the UI.

use crate::state::AppState;
use crate::workspace::to_slash;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub type WatcherHandle = Debouncer<RecommendedWatcher, RecommendedCache>;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FsChanged {
    /// Workspace-relative paths that changed (empty when `overflow`).
    pub paths: Vec<String>,
    /// Files/folders were created, removed or renamed (explorer must refresh).
    pub structural: bool,
    /// Something under .git changed (branch switch, commit, stage).
    pub git: bool,
    /// Too many changes to list: refresh everything.
    pub overflow: bool,
}

const MAX_PATHS: usize = 400;

fn build_ignore(root: &Path) -> Gitignore {
    let mut b = GitignoreBuilder::new(root);
    let _ = b.add(root.join(".gitignore"));
    for d in ["node_modules/", ".git/"] {
        let _ = b.add_line(None, d);
    }
    b.build().unwrap_or_else(|_| Gitignore::empty())
}

pub fn watch(app: AppHandle, root: PathBuf) -> notify::Result<WatcherHandle> {
    let ignore = build_ignore(&root);
    let root_cb = root.clone();
    let mut debouncer = new_debouncer(Duration::from_millis(150), None, move |res: DebounceEventResult| {
        let Ok(events) = res else { return };
        let mut paths = BTreeSet::new();
        let (mut structural, mut git) = (false, false);
        for ev in &events {
            let is_structural = matches!(
                ev.kind,
                EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
            );
            let is_content = is_structural || matches!(ev.kind, EventKind::Modify(_));
            if !is_content {
                continue;
            }
            for p in &ev.paths {
                let Ok(rel) = p.strip_prefix(&root_cb) else { continue };
                if rel.starts_with(".git") {
                    git = true;
                    continue;
                }
                let is_dir = p.is_dir();
                if ignore.matched_path_or_any_parents(rel, is_dir).is_ignore() {
                    continue;
                }
                structural |= is_structural;
                paths.insert(to_slash(rel));
            }
        }
        if paths.is_empty() && !git {
            return;
        }
        if structural {
            app.state::<AppState>().index.mark_dirty();
        }
        let overflow = paths.len() > MAX_PATHS;
        let payload = FsChanged {
            paths: if overflow { Vec::new() } else { paths.into_iter().collect() },
            structural,
            git,
            overflow,
        };
        let _ = app.emit("fs-changed", payload);
    })?;
    debouncer.watch(&root, RecursiveMode::Recursive)?;
    Ok(debouncer)
}
