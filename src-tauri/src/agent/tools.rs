//! The agent's tools: definitions sent to the API and their implementations.
//!
//! Read-only tools (read_file, list_dir, glob, grep) run immediately. File changes
//! (edit_file, write_file) are *prepared* first — the new content and a unified diff are
//! computed so the user can review them — and only written after approval. Commands run
//! in their own process group so the whole tree can be killed on timeout or cancel.

use crate::conversations::{ToolStatus, ToolUi};
use crate::search::{self, FileIndex, GrepOptions};
use crate::settings::PermissionMode;
use crate::util::{atomic_write, looks_binary, truncate_end, truncate_middle};
use crate::workspace::Workspace;
use serde::Deserialize;
use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

/// Max characters of tool output returned to the model.
const MAX_RESULT: usize = 40_000;
/// Max characters of detail kept for the UI.
const MAX_UI_DETAIL: usize = 100_000;
const READ_DEFAULT_LINES: usize = 2000;
const MAX_LINE_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    Read,
    Edit,
    Command,
    /// Network access (web_fetch): asks in Ask / Auto-accept-edits modes.
    Network,
    /// Bookkeeping (todo_write): never asks, allowed in Plan mode.
    Meta,
}

pub fn class_of(name: &str) -> Option<ToolClass> {
    match name {
        "read_file" | "list_dir" | "glob" | "grep" | "read_many_files" | "file_info" | "git_status" | "git_diff" | "git_log" => Some(ToolClass::Read),
        "edit_file" | "write_file" | "multi_edit" | "find_replace" | "create_directory" | "move_path" | "delete_path" => Some(ToolClass::Edit),
        "run_command" => Some(ToolClass::Command),
        "web_fetch" => Some(ToolClass::Network),
        "todo_write" => Some(ToolClass::Meta),
        _ => None,
    }
}

/// Tool definitions, in a fixed order (prompt-cache friendly). Plan mode exposes only
/// read-only tools. `eager` streams tool inputs as they are generated (Claude API only).
pub struct ToolOptions {
    pub mode: PermissionMode,
    /// Stream tool inputs as generated (Claude API only).
    pub eager: bool,
    /// Add Anthropic's server-side web search tool.
    pub web_search: bool,
}

pub fn definitions(opts: &ToolOptions) -> Vec<Value> {
    let (mode, eager) = (opts.mode, opts.eager);
    let mut tools = vec![
        json!({
            "name": "read_file",
            "description": "Read a text file from the workspace. Output lines are prefixed with their line number and a tab (`  12\\t...`); the prefix is not part of the file. Large files are returned in pages: use start_line/end_line to read other parts. Read a file before editing it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path relative to the workspace root." },
                    "start_line": { "type": "integer", "description": "First line to read (1-based)." },
                    "end_line": { "type": "integer", "description": "Last line to read (inclusive)." }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "list_dir",
            "description": "List a directory as an indented tree (directories end with '/'). Respects .gitignore. Use depth 2-3 to see nested structure.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory relative to the workspace root. Defaults to the root." },
                    "depth": { "type": "integer", "description": "How many levels to show (1-4, default 1)." }
                }
            }
        }),
        json!({
            "name": "glob",
            "description": "Find files by glob pattern, e.g. `**/*.rs` or `src/**/test_*.py`. Patterns without '/' match at any depth. Respects .gitignore.",
            "input_schema": {
                "type": "object",
                "properties": { "pattern": { "type": "string" } },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "grep",
            "description": "Search file contents with a regular expression (Rust regex syntax, like ripgrep). Returns `path:line: text` lines. Respects .gitignore and skips binary files.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regular expression." },
                    "path": { "type": "string", "description": "Directory or file to search, relative to the root. Defaults to the root." },
                    "include": { "type": "string", "description": "Only search files matching these comma-separated globs, e.g. `*.ts,*.tsx`." },
                    "case_insensitive": { "type": "boolean" },
                    "max_results": { "type": "integer", "description": "Default 200." }
                },
                "required": ["pattern"]
            }
        }),
    ];
    tools.extend(super::toolbox::read_defs());
    tools.extend(super::toolbox::web_defs());
    tools.push(super::toolbox::todo_def());
    if mode != PermissionMode::Plan {
        tools.push(json!({
            "name": "edit_file",
            "description": "Replace text in an existing file. `old_string` must match the current file content exactly (including whitespace and indentation, without line-number prefixes) and must be unique in the file unless `replace_all` is true; include surrounding lines to make it unique. Prefer this over write_file for changes to existing files.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string", "description": "Exact text to replace." },
                    "new_string": { "type": "string", "description": "Replacement text." },
                    "replace_all": { "type": "boolean", "description": "Replace every occurrence." }
                },
                "required": ["path", "old_string", "new_string"]
            }
        }));
        tools.push(json!({
            "name": "write_file",
            "description": "Create a new file, or replace an existing file's entire content. Parent directories are created as needed. To modify part of an existing file use edit_file instead.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string", "description": "The complete file content." }
                },
                "required": ["path", "content"]
            }
        }));
        tools.push(json!({
            "name": "run_command",
            "description": "Run a shell command in the workspace root and return its exit code and combined stdout/stderr. Use it for builds, tests, linters, git and package managers. There is no stdin: don't run interactive programs or commands that never exit (dev servers, watch modes). Default timeout 120 s (max 600).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "timeout_secs": { "type": "integer" }
                },
                "required": ["command"]
            }
        }));
    }
    if mode != PermissionMode::Plan {
        tools.extend(super::toolbox::edit_defs());
    }
    if eager {
        for t in &mut tools {
            t["eager_input_streaming"] = json!(true);
        }
    }
    if opts.web_search {
        // Server tool: no eager_input_streaming field allowed.
        tools.push(super::toolbox::web_search_def());
    }
    tools
}

// ---------------------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct ReadFileIn {
    path: String,
    start_line: Option<u64>,
    end_line: Option<u64>,
}

#[derive(Deserialize)]
struct ListDirIn {
    path: Option<String>,
    depth: Option<u64>,
}

#[derive(Deserialize)]
struct GlobIn {
    pattern: String,
}

#[derive(Deserialize)]
struct GrepIn {
    pattern: String,
    path: Option<String>,
    include: Option<String>,
    case_insensitive: Option<bool>,
    max_results: Option<u64>,
}

#[derive(Deserialize)]
struct EditFileIn {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[derive(Deserialize)]
struct WriteFileIn {
    path: String,
    content: String,
}

#[derive(Deserialize)]
pub struct RunCommandIn {
    pub command: String,
    pub timeout_secs: Option<u64>,
}

fn parse<T: for<'de> Deserialize<'de>>(input: &Value) -> Result<T, String> {
    T::deserialize(input).map_err(|e| format!("Invalid tool input: {e}"))
}

pub fn parse_command(input: &Value) -> Result<RunCommandIn, String> {
    let c: RunCommandIn = parse(input)?;
    if c.command.trim().is_empty() {
        return Err("Invalid tool input: `command` is empty".into());
    }
    Ok(c)
}

// ---------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------

pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    pub ui: ToolUi,
}

impl ToolResult {
    pub fn error(msg: impl Into<String>, summary: impl Into<String>) -> Self {
        let msg = msg.into();
        Self {
            ui: ToolUi { status: ToolStatus::Error, summary: summary.into(), detail: Some(msg.clone()), detail_kind: Some("text".into()), path: None },
            content: msg,
            is_error: true,
        }
    }

    fn ok(content: String, summary: String, detail: Option<(String, &str)>, path: Option<String>) -> Self {
        let (detail, detail_kind) = match detail {
            Some((d, k)) => (Some(truncate_middle(&d, MAX_UI_DETAIL)), Some(k.to_string())),
            None => (None, None),
        };
        Self {
            content: truncate_middle(&content, MAX_RESULT),
            is_error: false,
            ui: ToolUi { status: ToolStatus::Done, summary, detail, detail_kind, path },
        }
    }
}

/// One-line description of a tool call, shown while it runs / awaits approval.
pub fn describe(name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match name {
        "read_file" => format!("Read {}", s("path")),
        "list_dir" => format!("List {}", if s("path").is_empty() { ".".into() } else { s("path") }),
        "glob" => format!("Find {}", s("pattern")),
        "grep" => format!("Search for {}", s("pattern")),
        "edit_file" => format!("Edit {}", s("path")),
        "write_file" => format!("Write {}", s("path")),
        "run_command" => format!("Run {}", truncate_end(s("command").lines().next().unwrap_or(""), 120)),
        "multi_edit" => format!("Edit {}", s("path")),
        "find_replace" => format!("Replace {} → {}", s("pattern"), s("replacement")),
        "create_directory" => format!("Create {}/", s("path")),
        "move_path" => format!("Move {} → {}", s("from"), s("to")),
        "delete_path" => format!("Delete {}", s("path")),
        "read_many_files" => format!("Read {} files", input.get("paths").and_then(Value::as_array).map_or(0, Vec::len)),
        "file_info" => format!("Inspect {}", s("path")),
        "git_status" => "Git status".into(),
        "git_diff" => "Git diff".into(),
        "git_log" => "Git log".into(),
        "web_fetch" => format!("Fetch {}", truncate_end(&s("url"), 100)),
        "todo_write" => "Update plan".into(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------------------
// Read-only tools (blocking; run on the blocking thread pool)
// ---------------------------------------------------------------------------------------

pub fn run_read(name: &str, input: &Value, ws: &Workspace, index: &FileIndex) -> ToolResult {
    let res = match name {
        "read_file" => read_file(input, ws),
        "list_dir" => list_dir(input, ws),
        "glob" => glob(input, ws, index),
        "grep" => grep(input, ws),
        "read_many_files" => super::toolbox::read_many_files(input, ws),
        "file_info" => super::toolbox::file_info(input, ws),
        "git_status" | "git_diff" | "git_log" => super::toolbox::git_tool(name, input, ws),
        _ => Err(format!("Unknown tool {name}")),
    };
    res.unwrap_or_else(|e| ToolResult::error(e, describe(name, input)))
}

fn read_file(input: &Value, ws: &Workspace) -> Result<ToolResult, String> {
    let i: ReadFileIn = parse(input)?;
    let path = ws.resolve(&i.path).map_err(|e| e.to_string())?;
    let rel = ws.relative(&path);
    if path.is_dir() {
        return Err(format!("{rel} is a directory; use list_dir"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("Cannot read {rel}: {e}"))?;
    if looks_binary(&bytes) {
        return Err(format!("{rel} is a binary file ({} bytes)", bytes.len()));
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let start = i.start_line.unwrap_or(1).max(1) as usize;
    let end = i.end_line.map(|e| e as usize).unwrap_or(start + READ_DEFAULT_LINES - 1).min(total);
    if total == 0 {
        return Ok(ToolResult::ok("(empty file)".into(), format!("Read {rel}"), None, Some(rel)));
    }
    if start > total {
        return Err(format!("{rel} has only {total} lines"));
    }
    let mut out = String::with_capacity((end - start + 1) * 40);
    for (n, line) in lines[start - 1..end].iter().enumerate() {
        let line = if line.len() > MAX_LINE_CHARS { format!("{}… [line truncated]", truncate_end(line, MAX_LINE_CHARS)) } else { (*line).to_string() };
        out.push_str(&format!("{:>6}\t{}\n", start + n, line));
        if out.len() > MAX_RESULT {
            out.push_str(&format!("[output truncated at line {}; use start_line to continue]\n", start + n));
            break;
        }
    }
    if end < total && out.len() <= MAX_RESULT {
        out.push_str(&format!("[showing lines {start}-{end} of {total}; use start_line/end_line to read more]\n"));
    }
    let summary = if start == 1 && end == total { format!("Read {rel}") } else { format!("Read {rel} (lines {start}-{end})") };
    Ok(ToolResult::ok(out, summary, None, Some(rel)))
}

fn list_dir(input: &Value, ws: &Workspace) -> Result<ToolResult, String> {
    let i: ListDirIn = parse(input)?;
    let rel_in = i.path.unwrap_or_else(|| ".".into());
    let dir = ws.resolve(&rel_in).map_err(|e| e.to_string())?;
    if !dir.is_dir() {
        return Err(format!("{rel_in} is not a directory"));
    }
    let depth = i.depth.unwrap_or(1).clamp(1, 4) as usize;
    let mut walker = ignore::WalkBuilder::new(&dir);
    walker
        .hidden(false)
        .require_git(false)
        .max_depth(Some(depth))
        .filter_entry(|e| e.file_name() != ".git")
        .sort_by_file_path(|a, b| a.cmp(b));
    let mut out = String::new();
    let mut count = 0usize;
    let limit = 800;
    for entry in walker.build().flatten() {
        if entry.depth() == 0 {
            continue;
        }
        count += 1;
        if count > limit {
            out.push_str(&format!("[listing truncated at {limit} entries]\n"));
            break;
        }
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let name = entry.file_name().to_string_lossy();
        out.push_str(&"  ".repeat(entry.depth() - 1));
        out.push_str(&name);
        if is_dir {
            out.push('/');
        }
        out.push('\n');
    }
    let rel = ws.relative(&dir);
    if out.is_empty() {
        out = "(empty directory)".into();
    }
    Ok(ToolResult::ok(format!("{rel}/\n{out}"), format!("Listed {rel}"), Some((out.clone(), "output")), Some(rel)))
}

fn glob(input: &Value, ws: &Workspace, index: &FileIndex) -> Result<ToolResult, String> {
    let i: GlobIn = parse(input)?;
    let files = index.files(ws.root());
    let (matches, truncated) = search::glob_files(&files, &i.pattern, 500).map_err(|e| e.to_string())?;
    let n = matches.len();
    let mut out = matches.join("\n");
    if n == 0 {
        out = "No files found".into();
    }
    if truncated {
        out.push_str("\n[more than 500 matches; narrow the pattern]");
    }
    let summary = format!("Found {n}{} file{} matching {}", if truncated { "+" } else { "" }, if n == 1 { "" } else { "s" }, i.pattern);
    Ok(ToolResult::ok(out.clone(), summary, Some((out, "output")), None))
}

fn grep(input: &Value, ws: &Workspace) -> Result<ToolResult, String> {
    let i: GrepIn = parse(input)?;
    let start = ws.resolve(i.path.as_deref().unwrap_or(".")).map_err(|e| e.to_string())?;
    let opts = GrepOptions {
        regex: true,
        case_insensitive: i.case_insensitive.unwrap_or(false),
        whole_word: false,
        include: i.include.clone(),
        max_results: i.max_results.unwrap_or(200).clamp(1, 2000) as usize,
    };
    let res = search::grep(ws.root(), &start, &i.pattern, &opts).map_err(|e| e.to_string())?;
    let n = res.matches.len();
    let mut out = String::new();
    for m in &res.matches {
        out.push_str(&format!("{}:{}: {}\n", m.path, m.line, m.text));
    }
    if n == 0 {
        out = "No matches".into();
    }
    if res.truncated {
        out.push_str(&format!("[results truncated at {n}; narrow the search]\n"));
    }
    let summary = format!("Searched for {} — {n}{} match{}", i.pattern, if res.truncated { "+" } else { "" }, if n == 1 { "" } else { "es" });
    Ok(ToolResult::ok(out.clone(), summary, Some((out, "output")), None))
}

// ---------------------------------------------------------------------------------------
// File edits
// ---------------------------------------------------------------------------------------

/// One filesystem operation of a prepared change.
pub enum ChangeOp {
    Write { abs: PathBuf, rel: String, old: Option<String>, new: String },
    Mkdir { abs: PathBuf, rel: String },
    Move { from: PathBuf, to: PathBuf, rel_from: String, rel_to: String },
    /// Moves to the system trash, so agent deletes are recoverable.
    Delete { abs: PathBuf, rel: String },
}

/// A change computed up front (so it can be reviewed as a diff) and applied after approval.
pub struct PreparedEdit {
    pub ops: Vec<ChangeOp>,
    pub diff: String,
    pub summary: String,
    /// Main path, for "open in editor".
    pub path: Option<String>,
}

impl PreparedEdit {
    fn single(abs: PathBuf, rel: String, old: Option<String>, new: String, verb: &str) -> Self {
        let (diff, add, del) = diff_with_stats(&rel, old.as_deref(), &new);
        let summary = if old.is_none() { format!("Create {rel} (+{add})") } else { format!("{verb} {rel} (+{add} −{del})") };
        PreparedEdit { path: Some(rel.clone()), ops: vec![ChangeOp::Write { abs, rel, old, new }], diff, summary }
    }
}

fn read_existing(abs: &std::path::Path, rel: &str) -> Result<String, String> {
    if !abs.is_file() {
        return Err(format!("{rel} does not exist. Use write_file to create a new file."));
    }
    std::fs::read_to_string(abs).map_err(|e| format!("Cannot read {rel}: {e}"))
}

pub fn prepare_edit(name: &str, input: &Value, ws: &Workspace) -> Result<PreparedEdit, String> {
    match name {
        "edit_file" => {
            let i: EditFileIn = parse(input)?;
            let abs = ws.resolve(&i.path).map_err(|e| e.to_string())?;
            let rel = ws.relative(&abs);
            let old = read_existing(&abs, &rel)?;
            let new = apply_replacement(&old, &i.old_string, &i.new_string, i.replace_all).map_err(|e| format!("{e} (in {rel})"))?;
            Ok(PreparedEdit::single(abs, rel, Some(old), new, "Edit"))
        }
        "write_file" => {
            let i: WriteFileIn = parse(input)?;
            let abs = ws.resolve(&i.path).map_err(|e| e.to_string())?;
            let rel = ws.relative(&abs);
            if abs.is_dir() {
                return Err(format!("{rel} is a directory"));
            }
            let old = if abs.exists() { Some(std::fs::read_to_string(&abs).map_err(|e| format!("Cannot read {rel}: {e}"))?) } else { None };
            Ok(PreparedEdit::single(abs, rel, old, i.content, "Write"))
        }
        "multi_edit" => crate::agent::toolbox::prepare_multi_edit(input, ws),
        "find_replace" => crate::agent::toolbox::prepare_find_replace(input, ws),
        "create_directory" | "move_path" | "delete_path" => crate::agent::toolbox::prepare_fs_op(name, input, ws),
        _ => Err(format!("{name} is not an edit tool")),
    }
}

/// Apply a prepared change. Every precondition is re-checked first (files unchanged since
/// the diff was shown, move targets free), so nothing is half-applied on a conflict.
pub fn apply_edit(p: &PreparedEdit) -> ToolResult {
    for op in &p.ops {
        let problem = match op {
            ChangeOp::Write { abs, rel, old, .. } => (std::fs::read_to_string(abs).ok() != *old)
                .then(|| format!("{rel} changed on disk since this edit was prepared. Read it again and redo the edit.")),
            ChangeOp::Move { from, to, rel_from, rel_to } => {
                if !from.exists() {
                    Some(format!("{rel_from} no longer exists"))
                } else if to.exists() {
                    Some(format!("{rel_to} already exists"))
                } else {
                    None
                }
            }
            ChangeOp::Delete { abs, rel } => (!abs.exists() && !abs.is_symlink()).then(|| format!("{rel} no longer exists")),
            ChangeOp::Mkdir { .. } => None,
        };
        if let Some(msg) = problem {
            return ToolResult::error(msg, p.summary.clone());
        }
    }
    let mut done = Vec::new();
    for op in &p.ops {
        let res = match op {
            ChangeOp::Write { abs, rel, new, old } => atomic_write(abs, new.as_bytes())
                .map(|_| format!("{} {rel} ({} lines)", if old.is_some() { "Updated" } else { "Created" }, new.lines().count()))
                .map_err(|e| format!("Failed to write {rel}: {e}")),
            ChangeOp::Mkdir { abs, rel } => std::fs::create_dir_all(abs).map(|_| format!("Created folder {rel}")).map_err(|e| format!("Failed to create {rel}: {e}")),
            ChangeOp::Move { from, to, rel_from, rel_to } => {
                if let Some(parent) = to.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::rename(from, to).map(|_| format!("Moved {rel_from} → {rel_to}")).map_err(|e| format!("Failed to move {rel_from}: {e}"))
            }
            ChangeOp::Delete { abs, rel } => trash::delete(abs).map(|_| format!("Moved {rel} to the trash")).map_err(|e| format!("Failed to delete {rel}: {e}")),
        };
        match res {
            Ok(m) => done.push(m),
            Err(e) => {
                let msg = if done.is_empty() { e } else { format!("{e}. Already applied: {}", done.join("; ")) };
                return ToolResult::error(msg, p.summary.clone());
            }
        }
    }
    let summary = past_tense(&p.summary);
    let detail = if p.diff.trim().is_empty() { None } else { Some((p.diff.clone(), "diff")) };
    ToolResult::ok(format!("{}.", done.join("\n")), summary, detail, p.path.clone())
}

fn past_tense(s: &str) -> String {
    for (a, b) in [("Edit ", "Edited "), ("Write ", "Wrote "), ("Create ", "Created "), ("Move ", "Moved "), ("Delete ", "Deleted "), ("Replace ", "Replaced ")] {
        if let Some(rest) = s.strip_prefix(a) {
            return format!("{b}{rest}");
        }
    }
    s.to_string()
}

pub fn apply_replacement(content: &str, old: &str, new: &str, replace_all: bool) -> Result<String, String> {
    if old.is_empty() {
        return Err("old_string is empty".into());
    }
    if old == new {
        return Err("old_string and new_string are identical".into());
    }
    let try_replace = |old: &str, new: &str| -> Option<Result<String, String>> {
        let count = content.matches(old).count();
        match count {
            0 => None,
            1 => Some(Ok(content.replacen(old, new, 1))),
            n if replace_all => {
                let _ = n;
                Some(Ok(content.replace(old, new)))
            }
            n => Some(Err(format!(
                "old_string appears {n} times; include more surrounding context to make it unique, or set replace_all"
            ))),
        }
    };
    if let Some(r) = try_replace(old, new) {
        return r;
    }
    // The model writes \n; the file may use \r\n.
    if content.contains("\r\n") && !old.contains("\r\n") {
        let (o, n) = (old.replace('\n', "\r\n"), new.replace('\n', "\r\n"));
        if let Some(r) = try_replace(&o, &n) {
            return r;
        }
    }
    let hint = if content.contains(old.trim()) { " (a whitespace/indentation difference?)" } else { "" };
    Err(format!("old_string was not found in the file{hint}. Read the file again and copy the text exactly"))
}

/// Unified diff plus (added, removed) line counts, computed in one pass. The diff has a
/// deadline so pathological inputs fall back to a coarser (still correct) diff quickly.
pub fn diff_with_stats(rel: &str, old: Option<&str>, new: &str) -> (String, usize, usize) {
    let diff = TextDiff::configure()
        .timeout(Duration::from_millis(1500))
        .diff_lines(old.unwrap_or(""), new);
    let (mut add, mut del) = (0, 0);
    for c in diff.iter_all_changes() {
        match c.tag() {
            ChangeTag::Insert => add += 1,
            ChangeTag::Delete => del += 1,
            ChangeTag::Equal => {}
        }
    }
    let (from, to) = (if old.is_some() { format!("a/{rel}") } else { "/dev/null".into() }, format!("b/{rel}"));
    let text = diff.unified_diff().context_radius(3).header(&from, &to).to_string();
    (text, add, del)
}

// ---------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------

fn ansi_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\r").unwrap())
}

pub fn strip_ansi(s: &str) -> String {
    ansi_re().replace_all(s, "").into_owned()
}

/// Splits a byte stream into valid UTF-8 strings without breaking multi-byte chars.
#[derive(Default)]
struct Utf8Chunker {
    pending: Vec<u8>,
}

impl Utf8Chunker {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(), // incomplete trailing char
            Err(_) => self.pending.len(),                          // genuinely invalid: lossy
        };
        let out = String::from_utf8_lossy(&self.pending[..valid]).into_owned();
        self.pending.drain(..valid);
        out
    }
}

const MAX_CAPTURE: usize = 4 * 1024 * 1024;

#[cfg(unix)]
fn kill_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        // The child leads its own process group: kill the whole group.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.start_kill();
}

#[cfg(windows)]
fn kill_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output();
    }
    let _ = child.start_kill();
}

/// Run a shell command; `on_output` receives live output (ANSI-stripped).
pub async fn run_command(
    command: &str,
    timeout_secs: Option<u64>,
    ws: &Workspace,
    cancel: &CancellationToken,
    mut on_output: impl FnMut(&str),
) -> ToolResult {
    let timeout = Duration::from_secs(timeout_secs.unwrap_or(120).clamp(1, 600));
    let summary = format!("Ran {}", truncate_end(command.lines().next().unwrap_or(""), 120));
    let (shell, flag) = crate::process::command_shell();
    let mut cmd = crate::process::command(&shell);
    cmd.arg(flag)
        .arg(command)
        .current_dir(ws.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("PILUNCH", "1")
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolResult::error(format!("Failed to start {shell}: {e}"), summary),
    };
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let (mut b1, mut b2) = (vec![0u8; 16 * 1024], vec![0u8; 16 * 1024]);
    let (mut out_done, mut err_done) = (false, false);
    let mut captured: Vec<u8> = Vec::new();
    let mut dropped = 0usize;
    let mut chunker = Utf8Chunker::default();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut exit_status: Option<std::process::ExitStatus> = None;
    let grace = tokio::time::sleep(Duration::from_secs(3600));
    tokio::pin!(grace);

    enum End {
        Exited,
        Timeout,
        Cancelled,
    }
    let mut on_bytes = |bytes: &[u8], captured: &mut Vec<u8>, dropped: &mut usize| {
        if captured.len() < MAX_CAPTURE {
            captured.extend_from_slice(bytes);
        } else {
            *dropped += bytes.len();
        }
        let s = chunker.push(bytes);
        if !s.is_empty() {
            on_output(&strip_ansi(&s));
        }
    };
    let end = loop {
        if out_done && err_done && exit_status.is_some() {
            break End::Exited;
        }
        tokio::select! {
            r = stdout.read(&mut b1), if !out_done => match r {
                Ok(0) | Err(_) => out_done = true,
                Ok(n) => on_bytes(&b1[..n], &mut captured, &mut dropped),
            },
            r = stderr.read(&mut b2), if !err_done => match r {
                Ok(0) | Err(_) => err_done = true,
                Ok(n) => on_bytes(&b2[..n], &mut captured, &mut dropped),
            },
            st = child.wait(), if exit_status.is_none() => {
                exit_status = st.ok();
                // Background grandchildren may keep the pipes open; give them a moment.
                grace.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(500));
            },
            _ = &mut grace, if exit_status.is_some() => {
                kill_tree(&mut child);
                break End::Exited;
            },
            _ = &mut deadline => { kill_tree(&mut child); break End::Timeout; },
            _ = cancel.cancelled() => { kill_tree(&mut child); break End::Cancelled; },
        }
    };
    let mut text = strip_ansi(&String::from_utf8_lossy(&captured));
    if dropped > 0 {
        text.push_str(&format!("\n[{dropped} more bytes of output not captured]"));
    }
    let text = text.trim_end().to_string();
    let shown = if text.is_empty() { "(no output)".to_string() } else { text.clone() };
    match end {
        End::Cancelled => {
            let mut r = ToolResult::error(format!("Cancelled by the user.\n{}", truncate_middle(&shown, MAX_RESULT)), summary);
            r.ui.status = ToolStatus::Cancelled;
            r
        }
        End::Timeout => ToolResult::error(
            format!("Command timed out after {}s and was killed. Output so far:\n{}", timeout.as_secs(), truncate_middle(&shown, MAX_RESULT)),
            summary,
        ),
        End::Exited => {
            let code = exit_status.and_then(|s| s.code());
            let code_str = code.map_or_else(|| "killed by signal".to_string(), |c| c.to_string());
            let content = format!("Exit code: {code_str}\n{}", truncate_middle(&shown, MAX_RESULT));
            let mut r = ToolResult::ok(content, summary, Some((format!("{shown}\n\n[exit code {code_str}]"), "output")), None);
            if code != Some(0) {
                r.is_error = false; // a failing command is information for the model, not a tool error
                r.ui.status = ToolStatus::Error;
            }
            r
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::write(d.path().join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        let w = Workspace::open(d.path()).unwrap();
        (d, w)
    }

    #[test]
    fn definitions_respect_mode_and_eager() {
        let names = |t: Vec<Value>| t.iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let opts = |mode, eager, web_search| ToolOptions { mode, eager, web_search };
        let plan = names(definitions(&opts(PermissionMode::Plan, false, false)));
        assert!(plan.contains(&"git_diff".to_string()) && plan.contains(&"todo_write".to_string()));
        assert!(!plan.iter().any(|n| class_of(n).is_some_and(|c| matches!(c, ToolClass::Edit | ToolClass::Command))));
        let all = definitions(&opts(PermissionMode::Ask, true, false));
        assert_eq!(all.len(), 19); // + web_search when enabled = 20
        assert!(names(all.clone()).iter().all(|n| class_of(n).is_some()), "every tool has a class");
        assert!(all.iter().all(|t| t["eager_input_streaming"] == true));
        assert!(definitions(&opts(PermissionMode::Ask, false, false)).iter().all(|t| t.get("eager_input_streaming").is_none()));
        let ws = definitions(&opts(PermissionMode::Ask, true, true));
        let last = ws.last().unwrap();
        assert_eq!(last["type"], "web_search_20260209");
        assert!(last.get("eager_input_streaming").is_none());
    }

    #[test]
    fn replacement_rules() {
        assert_eq!(apply_replacement("a b a", "b", "c", false).unwrap(), "a c a");
        assert!(apply_replacement("a b a", "a", "c", false).unwrap_err().contains("2 times"));
        assert_eq!(apply_replacement("a b a", "a", "c", true).unwrap(), "c b c");
        assert!(apply_replacement("abc", "zzz", "y", false).unwrap_err().contains("not found"));
        assert!(apply_replacement("abc", "", "y", false).is_err());
        assert!(apply_replacement("abc", "b", "b", false).is_err());
        // CRLF files
        assert_eq!(apply_replacement("one\r\ntwo\r\n", "one\ntwo", "1\n2", false).unwrap(), "1\r\n2\r\n");
    }

    #[test]
    fn read_file_numbers_lines_and_pages() {
        let (_d, w) = ws();
        let idx = FileIndex::default();
        let r = run_read("read_file", &json!({"path": "src/main.rs"}), &w, &idx);
        assert!(!r.is_error);
        assert!(r.content.starts_with("     1\tfn main() {\n"));
        let r = run_read("read_file", &json!({"path": "src/main.rs", "start_line": 2, "end_line": 2}), &w, &idx);
        assert_eq!(r.content.lines().next().unwrap(), "     2\t    println!(\"hi\");");
        assert!(r.content.contains("showing lines 2-2 of 3"));
        let r = run_read("read_file", &json!({"path": "../x"}), &w, &idx);
        assert!(r.is_error);
        let r = run_read("read_file", &json!({"nope": 1}), &w, &idx);
        assert!(r.is_error && r.content.contains("Invalid tool input"));
    }

    #[test]
    fn list_glob_grep() {
        let (_d, w) = ws();
        let idx = FileIndex::default();
        let r = run_read("list_dir", &json!({"depth": 2}), &w, &idx);
        assert!(r.content.contains("src/\n  main.rs"), "{}", r.content);
        let r = run_read("glob", &json!({"pattern": "*.rs"}), &w, &idx);
        assert_eq!(r.content, "src/main.rs");
        let r = run_read("grep", &json!({"pattern": "print\\w+"}), &w, &idx);
        assert_eq!(r.content, "src/main.rs:2:     println!(\"hi\");\n");
    }

    #[test]
    fn edit_prepare_and_apply() {
        let (d, w) = ws();
        let p = prepare_edit("edit_file", &json!({"path":"src/main.rs","old_string":"\"hi\"","new_string":"\"hello\""}), &w).unwrap();
        assert!(p.diff.contains("-    println!(\"hi\");"));
        assert!(p.diff.contains("+    println!(\"hello\");"));
        assert_eq!(p.summary, "Edit src/main.rs (+1 −1)");
        let r = apply_edit(&p);
        assert!(!r.is_error, "{}", r.content);
        assert_eq!(r.ui.summary, "Edited src/main.rs (+1 −1)");
        assert!(std::fs::read_to_string(d.path().join("src/main.rs")).unwrap().contains("hello"));
        // stale prepared edit is refused
        let p2 = prepare_edit("edit_file", &json!({"path":"src/main.rs","old_string":"hello","new_string":"bye"}), &w).unwrap();
        std::fs::write(d.path().join("src/main.rs"), "changed").unwrap();
        assert!(apply_edit(&p2).is_error);
        // new file
        let p3 = prepare_edit("write_file", &json!({"path":"docs/new.md","content":"# New\n"}), &w).unwrap();
        assert!(p3.summary.starts_with("Create docs/new.md"));
        assert!(p3.diff.starts_with("--- /dev/null"));
        assert!(!apply_edit(&p3).is_error);
        assert_eq!(std::fs::read_to_string(d.path().join("docs/new.md")).unwrap(), "# New\n");
        assert!(prepare_edit("edit_file", &json!({"path":"missing.rs","old_string":"a","new_string":"b"}), &w).is_err());
        assert!(prepare_edit("write_file", &json!({"path":"/etc/passwd","content":"x"}), &w).is_err());
    }

    #[test]
    fn utf8_chunker_keeps_multibyte_chars() {
        let mut c = Utf8Chunker::default();
        let s = "é😀".as_bytes();
        let mut out = String::new();
        for b in s {
            out.push_str(&c.push(&[*b]));
        }
        assert_eq!(out, "é😀");
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m\r\n"), "red\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_command_output_exit_timeout_cancel() {
        let (_d, w) = ws();
        let cancel = CancellationToken::new();
        let mut live = String::new();
        let r = run_command("echo out; echo err 1>&2; exit 3", None, &w, &cancel, |s| live.push_str(s)).await;
        assert!(r.content.starts_with("Exit code: 3"));
        assert!(r.content.contains("out") && r.content.contains("err"));
        assert!(live.contains("out"));
        assert_eq!(r.ui.status, ToolStatus::Error);

        let r = run_command("pwd", None, &w, &cancel, |_| {}).await;
        assert!(r.content.contains(w.root().to_str().unwrap()));

        let t = std::time::Instant::now();
        let r = run_command("sleep 30", Some(1), &w, &cancel, |_| {}).await;
        assert!(r.is_error && r.content.contains("timed out"));
        assert!(t.elapsed() < Duration::from_secs(5));

        // A background grandchild holding the pipe open doesn't hang the tool.
        let t = std::time::Instant::now();
        let r = run_command("sleep 30 & echo started", Some(20), &w, &cancel, |_| {}).await;
        assert!(r.content.contains("started"));
        assert!(t.elapsed() < Duration::from_secs(5));

        let c2 = CancellationToken::new();
        let c3 = c2.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            c3.cancel();
        });
        let r = run_command("sleep 30", None, &w, &c2, |_| {}).await;
        assert_eq!(r.ui.status, ToolStatus::Cancelled);
    }
}
