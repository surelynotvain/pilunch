//! Fast project search, built on ripgrep's own crates.
//!
//! * `walk_files` — parallel, .gitignore-aware listing (the quick-open index).
//! * `fuzzy`      — nucleo (Helix's matcher) over the index for Ctrl+P.
//! * `grep`       — parallel regex/literal search with binary detection.
//! * `glob_files` — glob over the index (used by the agent).

use crate::error::{Error, Result};
use crate::util::{byte_to_utf16_index, char_to_utf16_index, truncate_end};
use crate::workspace::to_slash;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{sinks::Lossy, BinaryDetection, SearcherBuilder};
use ignore::{DirEntry, ParallelVisitor, ParallelVisitorBuilder, WalkBuilder, WalkState};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher as FuzzyMatcher, Utf32Str};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Directories never worth walking, even when a project has no .gitignore.
const ALWAYS_SKIP: &[&str] = &[".git", ".hg", ".svn", "node_modules", "__pycache__", ".DS_Store"];
pub const MAX_INDEXED_FILES: usize = 250_000;

/// `parents(true)` makes searches that start in a subdirectory still honor the
/// .gitignore files above it.
fn walker(start: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(start);
    b.hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .threads(std::thread::available_parallelism().map(|n| n.get().min(12)).unwrap_or(4))
        .filter_entry(|e| !e.file_name().to_str().is_some_and(|n| ALWAYS_SKIP.contains(&n)));
    b
}

// ---------------------------------------------------------------------------------------
// File index
// ---------------------------------------------------------------------------------------

struct Collector<'s> {
    root: &'s Path,
    out: &'s Mutex<Vec<String>>,
    count: &'s AtomicUsize,
}

struct LocalCollector<'s> {
    root: &'s Path,
    out: &'s Mutex<Vec<String>>,
    count: &'s AtomicUsize,
    buf: Vec<String>,
}

impl<'s> ParallelVisitorBuilder<'s> for Collector<'s> {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 's> {
        Box::new(LocalCollector { root: self.root, out: self.out, count: self.count, buf: Vec::with_capacity(1024) })
    }
}

impl ParallelVisitor for LocalCollector<'_> {
    fn visit(&mut self, entry: std::result::Result<DirEntry, ignore::Error>) -> WalkState {
        let Ok(entry) = entry else { return WalkState::Continue };
        if !entry.file_type().is_some_and(|t| t.is_file() || t.is_symlink()) {
            return WalkState::Continue;
        }
        if self.count.fetch_add(1, Ordering::Relaxed) >= MAX_INDEXED_FILES {
            return WalkState::Quit;
        }
        if let Ok(rel) = entry.path().strip_prefix(self.root) {
            self.buf.push(to_slash(rel));
        }
        WalkState::Continue
    }
}

impl Drop for LocalCollector<'_> {
    fn drop(&mut self) {
        if !self.buf.is_empty() {
            self.out.lock().unwrap().append(&mut self.buf);
        }
    }
}

/// All non-ignored files under `root`, as sorted workspace-relative paths.
pub fn walk_files(root: &Path) -> Vec<String> {
    let out = Mutex::new(Vec::new());
    let count = AtomicUsize::new(0);
    walker(root).build_parallel().visit(&mut Collector { root, out: &out, count: &count });
    let mut files = out.into_inner().unwrap();
    files.sort_unstable();
    files.truncate(MAX_INDEXED_FILES);
    files
}

/// Cached file list for the open workspace; rebuilt lazily after the watcher marks it dirty.
#[derive(Default)]
pub struct FileIndex {
    state: RwLock<IndexState>,
    dirty: AtomicBool,
}

#[derive(Default)]
struct IndexState {
    root: Option<PathBuf>,
    files: Arc<Vec<String>>,
}

impl FileIndex {
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    pub fn files(&self, root: &Path) -> Arc<Vec<String>> {
        {
            let s = self.state.read().unwrap();
            if s.root.as_deref() == Some(root) && !self.dirty.load(Ordering::Acquire) {
                return s.files.clone();
            }
        }
        self.dirty.store(false, Ordering::Release);
        let files = Arc::new(walk_files(root));
        let mut s = self.state.write().unwrap();
        s.root = Some(root.to_path_buf());
        s.files = files.clone();
        files
    }
}

// ---------------------------------------------------------------------------------------
// Fuzzy file matching
// ---------------------------------------------------------------------------------------

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    pub path: String,
    pub score: u32,
    /// UTF-16 indices of matched characters (for highlighting in JS).
    pub indices: Vec<u32>,
}

pub fn fuzzy(files: &[String], query: &str, limit: usize) -> Vec<FileMatch> {
    let query = query.trim();
    if query.is_empty() {
        return files.iter().take(limit).map(|p| FileMatch { path: p.clone(), score: 0, indices: vec![] }).collect();
    }
    let mut matcher = FuzzyMatcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize)> = files
        .iter()
        .enumerate()
        .filter_map(|(i, p)| pattern.score(Utf32Str::new(p, &mut buf), &mut matcher).map(|s| (s, i)))
        .collect();
    // Higher score first; shorter path breaks ties.
    let cmp = |a: &(u32, usize), b: &(u32, usize)| b.0.cmp(&a.0).then(files[a.1].len().cmp(&files[b.1].len()));
    if scored.len() > limit {
        scored.select_nth_unstable_by(limit, cmp);
        scored.truncate(limit);
    }
    scored.sort_unstable_by(cmp);
    scored
        .into_iter()
        .map(|(score, i)| {
            let path = &files[i];
            let mut idx = Vec::new();
            pattern.indices(Utf32Str::new(path, &mut buf), &mut matcher, &mut idx);
            idx.sort_unstable();
            idx.dedup();
            let indices = if path.is_ascii() {
                idx
            } else {
                idx.into_iter().map(|c| char_to_utf16_index(path, c as usize) as u32).collect()
            };
            FileMatch { path: path.clone(), score, indices }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// Text search
// ---------------------------------------------------------------------------------------

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct GrepOptions {
    /// Treat the query as a regular expression (otherwise literal).
    pub regex: bool,
    pub case_insensitive: bool,
    pub whole_word: bool,
    /// Comma-separated globs files must match, e.g. "*.rs,*.toml".
    pub include: Option<String>,
    pub max_results: usize,
}

impl Default for GrepOptions {
    fn default() -> Self {
        Self { regex: false, case_insensitive: true, whole_word: false, include: None, max_results: 2000 }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrepMatch {
    pub path: String,
    pub line: u64,
    pub text: String,
    /// UTF-16 [start, end) ranges of the matches within `text`.
    pub ranges: Vec<[u32; 2]>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrepResult {
    pub matches: Vec<GrepMatch>,
    pub truncated: bool,
}

const MAX_LINE_LEN: usize = 400;

fn build_globset(include: Option<&str>) -> Result<Option<GlobSet>> {
    let Some(include) = include.map(str::trim).filter(|s| !s.is_empty()) else { return Ok(None) };
    let mut b = GlobSetBuilder::new();
    for g in include.split(',').map(str::trim).filter(|g| !g.is_empty()) {
        let g = if g.contains('/') { g.to_string() } else { format!("**/{g}") };
        b.add(GlobBuilder::new(&g).literal_separator(true).build().map_err(|e| Error::msg(e.to_string()))?);
    }
    Ok(Some(b.build().map_err(|e| Error::msg(e.to_string()))?))
}

pub fn grep(root: &Path, start: &Path, query: &str, opts: &GrepOptions) -> Result<GrepResult> {
    if query.is_empty() {
        return Ok(GrepResult { matches: vec![], truncated: false });
    }
    let matcher: RegexMatcher = RegexMatcherBuilder::new()
        .case_insensitive(opts.case_insensitive)
        .word(opts.whole_word)
        .fixed_strings(!opts.regex)
        .build(query)
        .map_err(|e| Error::msg(format!("invalid pattern: {e}")))?;
    let globs = build_globset(opts.include.as_deref())?;
    let max = opts.max_results.clamp(1, 20_000);
    let found = Mutex::new(Vec::<GrepMatch>::new());
    let count = AtomicUsize::new(0);
    let truncated = AtomicBool::new(false);

    walker(start).build_parallel().run(|| {
        let matcher = matcher.clone();
        let globs = globs.clone();
        let (found, count, truncated) = (&found, &count, &truncated);
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .line_number(true)
            .build();
        Box::new(move |entry| {
            let Ok(entry) = entry else { return WalkState::Continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            let rel = entry.path().strip_prefix(root).map(to_slash).unwrap_or_default();
            if let Some(g) = &globs {
                if !g.is_match(&rel) {
                    return WalkState::Continue;
                }
            }
            let mut local = Vec::new();
            let _ = searcher.search_path(
                &matcher,
                entry.path(),
                Lossy(|lnum, line| {
                    if count.fetch_add(1, Ordering::Relaxed) >= max {
                        truncated.store(true, Ordering::Relaxed);
                        return Ok(false);
                    }
                    let line = line.trim_end_matches(['\n', '\r']);
                    let text = truncate_end(line, MAX_LINE_LEN);
                    let mut ranges = Vec::new();
                    let _ = matcher.find_iter(text.as_bytes(), |m| {
                        ranges.push([byte_to_utf16_index(text, m.start()) as u32, byte_to_utf16_index(text, m.end()) as u32]);
                        ranges.len() < 50
                    });
                    local.push(GrepMatch { path: rel.clone(), line: lnum, text: text.to_string(), ranges });
                    Ok(true)
                }),
            );
            if !local.is_empty() {
                found.lock().unwrap().append(&mut local);
            }
            if truncated.load(Ordering::Relaxed) { WalkState::Quit } else { WalkState::Continue }
        })
    });

    let mut matches = found.into_inner().unwrap();
    matches.sort_unstable_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    matches.truncate(max);
    Ok(GrepResult { matches, truncated: truncated.load(Ordering::Relaxed) })
}

/// Files in the index matching a glob. Patterns without a `/` match at any depth.
pub fn glob_files(files: &[String], pattern: &str, limit: usize) -> Result<(Vec<String>, bool)> {
    let pattern = pattern.trim().trim_start_matches("./");
    let pattern = if pattern.contains('/') { pattern.to_string() } else { format!("**/{pattern}") };
    let glob = GlobBuilder::new(&pattern)
        .literal_separator(true)
        .build()
        .map_err(|e| Error::msg(format!("invalid glob: {e}")))?
        .compile_matcher();
    let mut out: Vec<String> = files.iter().filter(|f| glob.is_match(f.as_str())).take(limit + 1).cloned().collect();
    let truncated = out.len() > limit;
    out.truncate(limit);
    Ok((out, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        std::fs::create_dir_all(p.join("src/ui")).unwrap();
        std::fs::create_dir_all(p.join("target/debug")).unwrap();
        std::fs::create_dir_all(p.join("node_modules/x")).unwrap();
        std::fs::write(p.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(p.join("src/main.rs"), "fn main() {\n    println!(\"Hello PiLunch\");\n}\n").unwrap();
        std::fs::write(p.join("src/ui/app.tsx"), "export const hello = 'hello world';\n").unwrap();
        std::fs::write(p.join("README.md"), "# Hello\n").unwrap();
        std::fs::write(p.join("debug.log"), "hello log\n").unwrap();
        std::fs::write(p.join("target/debug/out.txt"), "hello target\n").unwrap();
        std::fs::write(p.join("node_modules/x/index.js"), "hello dep\n").unwrap();
        std::fs::write(p.join("src/blob.bin"), b"hello\x00\x01\x02").unwrap();
        d
    }

    #[test]
    fn walk_respects_gitignore_and_skips() {
        let d = project();
        let files = walk_files(d.path());
        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(files.contains(&".gitignore".to_string()));
        assert!(!files.iter().any(|f| f.starts_with("target/") || f.starts_with("node_modules/") || f.ends_with(".log")));
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
    }

    #[test]
    fn fuzzy_ranks_best_match_first() {
        let files: Vec<String> = ["src/main.rs", "src/ui/app.tsx", "README.md", "src/ui/main_menu.tsx"].map(String::from).to_vec();
        let res = fuzzy(&files, "mainrs", 10);
        assert_eq!(res[0].path, "src/main.rs");
        assert!(!res[0].indices.is_empty());
        assert_eq!(fuzzy(&files, "", 2).len(), 2);
        assert!(fuzzy(&files, "zzzz", 10).is_empty());
    }

    #[test]
    fn grep_literal_regex_and_globs() {
        let d = project();
        let r = grep(d.path(), d.path(), "hello", &GrepOptions::default()).unwrap();
        let paths: Vec<_> = r.matches.iter().map(|m| m.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"src/ui/app.tsx"));
        assert!(!paths.contains(&"debug.log"), "gitignored");
        assert!(!paths.contains(&"src/blob.bin"), "binary");
        let m = r.matches.iter().find(|m| m.path == "src/main.rs").unwrap();
        assert_eq!(m.line, 2);
        assert_eq!(m.ranges[0], [14, 19]);

        let opts = GrepOptions { regex: true, case_insensitive: false, include: Some("*.tsx".into()), ..Default::default() };
        let r = grep(d.path(), d.path(), r"hel+o\s\w+", &opts).unwrap();
        assert_eq!(r.matches.len(), 1);
        assert_eq!(r.matches[0].path, "src/ui/app.tsx");

        let opts = GrepOptions { max_results: 1, ..Default::default() };
        let r = grep(d.path(), d.path(), "hello", &opts).unwrap();
        assert_eq!(r.matches.len(), 1);
        assert!(r.truncated);

        assert!(grep(d.path(), d.path(), "(", &GrepOptions { regex: true, ..Default::default() }).is_err());
        // searching a subdirectory
        let r = grep(d.path(), &d.path().join("src/ui"), "hello", &GrepOptions::default()).unwrap();
        assert!(r.matches.iter().all(|m| m.path.starts_with("src/ui/")));
    }

    #[test]
    fn glob_matching() {
        let files: Vec<String> = ["src/main.rs", "src/ui/app.tsx", "build.rs", "README.md"].map(String::from).to_vec();
        let (m, t) = glob_files(&files, "*.rs", 10).unwrap();
        assert_eq!(m, vec!["src/main.rs", "build.rs"]);
        assert!(!t);
        let (m, _) = glob_files(&files, "src/*.rs", 10).unwrap();
        assert_eq!(m, vec!["src/main.rs"]);
        let (m, t) = glob_files(&files, "**/*", 2).unwrap();
        assert_eq!(m.len(), 2);
        assert!(t);
    }
}
