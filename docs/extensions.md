# Extending PiLunch

PiLunch's agent can be extended in four ways, from the simplest to the most powerful:

| | What it is | Where it lives |
|---|---|---|
| **Skills** | Markdown instructions the agent loads when a task matches, and writes itself | `~/.config/pilunch/skills/<name>/SKILL.md`, `<project>/.pilunch/skills/` |
| **Custom tools** | A shell command with a JSON Schema for its inputs | `~/.config/pilunch/tools/<name>.json`, `<project>/.pilunch/tools/` |
| **MCP servers** | Any [Model Context Protocol](https://modelcontextprotocol.io) server (stdio or HTTP) | `~/.config/pilunch/mcp.json` |
| **Plugins** | A folder bundling skills, tools and MCP servers. **Rust extensions** are plugins whose tools are a compiled Rust program | `~/.config/pilunch/plugins/<id>/` |

Everything is managed from **Customize** (the puzzle icon in the sidebar, or *Customize* in the command palette). Changes apply from the next message, so you don't need to restart.

On Windows, `~/.config/pilunch` is `%APPDATA%\pilunch`.

## Skills

A skill is a folder with a `SKILL.md`:

```markdown
---
name: release-checklist
description: How to cut a release of this project
---

1. Run `npm run typecheck` and `cargo test`.
2. Bump the version in package.json, src-tauri/Cargo.toml and tauri.conf.json.
3. …
```

The system prompt lists every skill's `name` and `description`. When a task matches, the agent calls `skill_load` to read the full text. With `skill_save` the agent writes new skills or improves existing ones. Ask it to "save what you learned as a skill" after it researches an API or finally gets a tricky build working. Saving a skill shows a diff and asks first, like a file edit.

If two skills share a name, the first found wins: project, then yours, then plugins.

## Custom tools

```json
{
  "name": "deploy_preview",
  "description": "Deploy the current branch to a preview environment and print its URL.",
  "parameters": {
    "type": "object",
    "properties": { "env": { "type": "string", "enum": ["staging", "qa"] } },
    "required": ["env"]
  },
  "command": "./scripts/deploy.sh {{env}}",
  "timeout": 300
}
```

- `{{param}}` is replaced with the **shell-quoted** value, so inputs can't inject commands.
- Each input is also available as an environment variable `PILUNCH_ARG_<NAME>`, and the whole input is in `PILUNCH_INPUT` as JSON. That makes it easy to call a script in any language: `python3 tools/report.py` can read `PILUNCH_INPUT`.
- Tools run in the project folder with bash (PowerShell on Windows). They ask for approval like `run_command`, unless you allow tools for the chat or use Bypass mode. A failing exit code is reported to the agent.
- Names must be letters, digits, `_` and `-`, and can't shadow a built-in tool.

## MCP servers

`mcp.json` uses the same format as Claude Desktop and other MCP hosts, so you can paste existing configs:

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "…" }
    },
    "docs": {
      "url": "https://example.com/mcp",
      "headers": { "Authorization": "Bearer …" }
    },
    "old-one": { "command": "my-server", "disabled": true }
  }
}
```

- **stdio** servers (`command`, `args`, `env`, `cwd`) are started by PiLunch and kept running between messages.
- **Streamable HTTP** servers (`url`, `headers`) are called with JSON-RPC over POST. Both JSON and SSE responses are supported, and so are session IDs.
- Their tools appear to the agent as `mcp__<server>__<tool>`. Tools annotated `readOnlyHint: true` run without asking and are available in Plan mode. Other tools ask first.
- Text, images and embedded resources in results are passed to the model.
- `mcp.json` is saved with owner-only permissions because it often holds tokens.

## Plugins

```text
my-plugin/
  plugin.json
  skills/<name>/SKILL.md
  tools/<name>.json
```

```json
{
  "name": "My Plugin",
  "version": "1.0.0",
  "description": "Release tooling for our repos",
  "mcpServers": {
    "release": { "command": "${PLUGIN_DIR}/bin/release-server" }
  }
}
```

`${PLUGIN_DIR}` expands to the installed plugin's folder. Custom tools from a plugin also get `PILUNCH_PLUGIN_DIR`. You can install a plugin in three ways:

- **From a folder:** the folder is copied into `plugins/`.
- **From a git URL:** shallow-cloned.
- **By building a Rust extension:** see below.

You can turn a plugin off without removing it.

## Rust extensions

A Rust extension is a small program that gives the agent new tools, built on the [`pilunch-extension`](../extensions/pilunch-extension) crate. The crate implements the MCP stdio protocol, so you only describe tools and write handlers. The result also works in any other MCP host.

```toml
# Cargo.toml
[package]
name = "my-ext"
version = "0.1.0"
edition = "2021"

[dependencies]
pilunch-extension = { git = "https://github.com/surelynotvain/pilunch" }
```

```rust
// src/main.rs
use pilunch_extension::{json, Extension, Output};

fn main() {
    Extension::new("my-ext", env!("CARGO_PKG_VERSION"))
        .instructions("Tools for working with our internal services.")
        // Read-only tools run without approval and work in Plan mode.
        .read_only_tool(
            "word_count",
            "Count words in a text.",
            json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }),
            |args| {
                let text = args["text"].as_str().ok_or("text is required")?;
                Ok(Output::text(format!("{} words", text.split_whitespace().count())))
            },
        )
        // Other tools ask the user first.
        .tool(
            "greet",
            "Write a greeting.",
            json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }),
            |args| Ok(Output::text(format!("Hello, {}!", args["name"].as_str().unwrap_or("world")))),
        )
        .run();
}
```

Handlers get the tool input as `serde_json::Value`. They return:

- `Ok(Output::text(…))` for normal output.
- `Output::image(base64, "image/png")` for images (combine several outputs with `.and(…)`).
- `Err(message)`, which the model sees as a failed call it can recover from.

A panic in a handler is caught and reported the same way. Use `eprintln!` for logging, because stdout is the protocol channel.

**Install it:** open *Customize → Plugins & extensions → Build Rust extension…* and pick the crate folder. PiLunch then:

1. runs `cargo build --release` (you need [Rust](https://rustup.rs) installed)
2. copies the binary to `plugins/<crate-name>/bin/`
3. copies `skills/` and `tools/` from the crate folder, so an extension can ship skills that teach the agent to use its tools
4. writes `plugin.json` with an MCP server pointing at the binary (if your crate has its own `plugin.json`, its fields are kept)

To update an extension, build it again: the previous install is replaced.

A complete example lives in [`extensions/examples/hello-ext`](../extensions/examples/hello-ext). To test an extension without PiLunch, pipe JSON-RPC to it:

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"greet","arguments":{"name":"Ada"}}}' \
  | cargo run -q
```

## The built-in browser and computer use

These are not extensions, but they come up next to them:

- **Browser** (*Settings → Browser & computer use*, on by default). The agent drives a Firefox session through [geckodriver](https://github.com/mozilla/geckodriver/releases). Install Firefox and put geckodriver on your PATH, or set its path in Settings.
  - The agent's `browser` tool can navigate, snapshot (page text plus numbered elements), take screenshots, click, type, press keys and scroll.
  - The **Browser panel** (globe icon, `Ctrl+Shift+B`) shows the same session live, and you can click, type and scroll in it yourself.
  - Opening a page asks first.
- **Computer use** (off by default). The `computer` tool takes screenshots and controls the mouse and keyboard.
  - On X11 it needs `xdotool` and ImageMagick (`import`) or `scrot`.
  - On Wayland it needs `grim` and `ydotool` or `wtype`.
  - On Windows it uses PowerShell.
  - Every action asks first unless you allow computer use for the chat.
