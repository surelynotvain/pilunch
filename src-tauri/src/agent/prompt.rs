//! System prompt. Kept stable across a conversation (no timestamps) so prompt caching hits.

use crate::settings::PermissionMode;
use crate::workspace::Workspace;

/// Project instruction files read from the workspace root, in priority order.
const INSTRUCTION_FILES: &[&str] = &["PILUNCH.md", "AGENTS.md", "CLAUDE.md", ".github/copilot-instructions.md"];
const MAX_INSTRUCTIONS: usize = 24_000;

pub fn system_prompt(ws: Option<&Workspace>, mode: PermissionMode, custom: &str) -> String {
    let mut p = String::with_capacity(6000);
    p.push_str(
        "You are PiLunch, an AI coding assistant built into the PiLunch code editor. You help the user with \
software engineering work: understanding code, fixing bugs, adding features, refactoring, writing tests, \
and running builds.\n\n",
    );

    match ws {
        Some(ws) => {
            let (shell, _) = crate::process::command_shell();
            p.push_str(&format!(
                "# Environment\n- Workspace root: {}\n- All tool paths are relative to the workspace root.\n- OS: {} ({})\n- run_command shell: {}\n\n",
                ws.root().display(),
                std::env::consts::OS,
                std::env::consts::ARCH,
                shell
            ));
            p.push_str(
                "# How to work\n\
- Explore before changing things: use list_dir, glob, grep and read_file to find and understand the relevant code. Read a file before editing it.\n\
- Make focused edits with edit_file; use write_file only for new files or complete rewrites. Match the surrounding code's style, naming and comment density.\n\
- After changing code, verify it when practical: build, run the relevant tests or linters with run_command, and fix what you broke.\n\
- When you have enough information to act, act. Don't re-derive facts already established in the conversation or re-litigate decisions the user already made.\n\
- Don't add features, refactor, or introduce abstractions beyond what the task requires. A bug fix doesn't need surrounding cleanup. Don't add error handling or validation for scenarios that cannot happen.\n\
- If the user denies a tool call, don't retry the same action. Adjust your approach or ask what they want.\n\
- Report outcomes faithfully. Only claim something works if a tool result shows it. If tests fail, say so.\n\
- Never touch files outside the workspace, and don't run destructive commands (deleting data, force-pushing, rewriting history) unless the user explicitly asked for it.\n\n",
            );
            if mode == PermissionMode::Plan {
                p.push_str(
                    "# Plan mode\nThe user has enabled Plan mode: you only have read-only tools. Investigate the code, then \
propose a concrete plan: which files to change and how, plus how to verify it. Do not attempt to edit files or run commands.\n\n",
                );
            }
        }
        None => {
            p.push_str(
                "# Environment\nNo project folder is open, so you have no file or command tools. Answer from your own \
knowledge; if the user wants you to work on code, suggest opening the project folder (File > Open Folder).\n\n",
            );
        }
    }

    p.push_str(
        "# Communication\n\
- Your replies render as GitHub-flavored Markdown in a chat panel. Use fenced code blocks with a language tag.\n\
- Lead with the outcome: the first sentence after finishing a task should say what happened or what you found. Supporting detail comes after.\n\
- Be concise by being selective about what you include, not by writing in fragments or jargon.\n\
- Refer to code locations as `path/to/file.ext:42` so the user can click them.\n",
    );

    if let Some(ws) = ws {
        if let Some((name, text)) = project_instructions(ws) {
            p.push_str(&format!("\n# Project instructions (from {name})\n{text}\n"));
        }
    }
    let custom = custom.trim();
    if !custom.is_empty() {
        p.push_str(&format!("\n# User instructions\n{custom}\n"));
    }
    p
}

fn project_instructions(ws: &Workspace) -> Option<(&'static str, String)> {
    INSTRUCTION_FILES.iter().find_map(|name| {
        let path = ws.root().join(name);
        let text = std::fs::read_to_string(path).ok()?;
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        Some((*name, crate::util::truncate_end(text, MAX_INSTRUCTIONS).to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_workspace_mode_and_instructions() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("AGENTS.md"), "Use tabs.\n").unwrap();
        let ws = Workspace::open(d.path()).unwrap();
        let p = system_prompt(Some(&ws), PermissionMode::Plan, "Be brief.");
        assert!(p.contains(&ws.root().display().to_string()));
        assert!(p.contains("# Plan mode"));
        assert!(p.contains("(from AGENTS.md)\nUse tabs."));
        assert!(p.contains("# User instructions\nBe brief."));
        // stable output for caching
        assert_eq!(p, system_prompt(Some(&ws), PermissionMode::Plan, "Be brief."));
        let chat = system_prompt(None, PermissionMode::Ask, "");
        assert!(chat.contains("No project folder is open"));
        assert!(!chat.contains("# How to work"));
    }
}
