// End-to-end test of the real PiLunch app (Rust core + WebKitGTK webview) driven over
// WebDriver (tauri-driver + WebKitWebDriver), with the Claude API replaced by
// e2e/mock-anthropic.mjs. Saves a screenshot of every step to e2e/out/.
//
//   xvfb-run -a dbus-run-session -- node e2e/run.mjs
//
// Requires: a built app (npm run tauri build -- --debug --no-bundle), `tauri-driver`
// (cargo install tauri-driver) and WebKitWebDriver (apt install webkit2gtk-driver).
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const APP = process.env.PILUNCH_BIN ?? path.join(ROOT, "src-tauri/target/debug/pilunch");
const OUT = path.join(ROOT, "e2e/out");
const DRIVER = "http://127.0.0.1:4444";
const MOCK_PORT = 8787;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------------------- fixtures
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "pilunch-e2e-"));
const configDir = path.join(tmp, "config");
const dataDir = path.join(tmp, "data");
const ws = path.join(tmp, "demo-project");
const mockLog = path.join(tmp, "requests.jsonl");
fs.mkdirSync(configDir, { recursive: true });
fs.mkdirSync(path.join(ws, "src"), { recursive: true });
fs.writeFileSync(
  path.join(ws, "src/main.rs"),
  `use std::io;\n\n/// Entry point of the demo project.\nfn main() {\n    let greeting = greet("world");\n    println!("{greeting}");\n}\n\nfn greet(name: &str) -> String {\n    format!("Hello, {name}!")\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn greets() {\n        assert_eq!(greet("PiLunch"), "Hello, PiLunch!");\n    }\n}\n`,
);
fs.writeFileSync(path.join(ws, "src/lib.rs"), "pub mod util;\n");
fs.writeFileSync(path.join(ws, "Cargo.toml"), `[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n`);
fs.writeFileSync(path.join(ws, "README.md"), "# Demo project\n\nA tiny project used by the PiLunch end-to-end test.\n");
fs.writeFileSync(path.join(ws, ".gitignore"), "target/\n");
const git = (...args) => execFileSync("git", args, { cwd: ws, stdio: "ignore" });
git("init", "-q", "-b", "main");
git("-c", "user.name=e2e", "-c", "user.email=e2e@example.com", "add", ".");
git("-c", "user.name=e2e", "-c", "user.email=e2e@example.com", "commit", "-qm", "init");
fs.appendFileSync(path.join(ws, "README.md"), "\nUncommitted change.\n");

fs.writeFileSync(
  path.join(configDir, "settings.json"),
  JSON.stringify({ baseUrl: `http://127.0.0.1:${MOCK_PORT}`, permissionMode: "ask", recentWorkspaces: [ws], theme: "dark" }),
);
fs.writeFileSync(path.join(configDir, "secrets.json"), JSON.stringify({ anthropicApiKey: "test-key" }), { mode: 0o600 });
fs.rmSync(OUT, { recursive: true, force: true });
fs.mkdirSync(OUT, { recursive: true });

// ---------------------------------------------------------------------------- processes
const procs = [];
function start(cmd, args, env = {}) {
  const p = spawn(cmd, args, { env: { ...process.env, ...env }, stdio: ["ignore", "pipe", "pipe"] });
  p.stdout.on("data", (d) => process.env.E2E_VERBOSE && process.stdout.write(`[${cmd}] ${d}`));
  p.stderr.on("data", (d) => process.env.E2E_VERBOSE && process.stderr.write(`[${cmd}] ${d}`));
  procs.push(p);
  return p;
}
function cleanup() {
  for (const p of procs) p.kill("SIGTERM");
}
process.on("exit", cleanup);

start("node", [path.join(ROOT, "e2e/mock-anthropic.mjs"), String(MOCK_PORT), mockLog]);
start("tauri-driver", ["--port", "4444"], {
  PILUNCH_CONFIG_DIR: configDir,
  PILUNCH_DATA_DIR: dataDir,
  // Isolate WebKit's own storage (localStorage holds the layout) from previous runs.
  XDG_DATA_HOME: path.join(tmp, "xdg-data"),
  XDG_CACHE_HOME: path.join(tmp, "xdg-cache"),
  XDG_CONFIG_HOME: path.join(tmp, "xdg-config"),
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
});

// ---------------------------------------------------------------------------- webdriver
async function wd(method, url, body) {
  const res = await fetch(DRIVER + url, { method, headers: { "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
  const json = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(`${method} ${url} -> ${res.status} ${JSON.stringify(json.value ?? json)}`);
  return json.value;
}

for (let i = 0; ; i++) {
  try {
    await fetch(DRIVER + "/status");
    break;
  } catch {
    if (i > 100) throw new Error("tauri-driver did not start");
    await sleep(100);
  }
}

const session = await wd("POST", "/session", {
  capabilities: { alwaysMatch: { browserName: "wry", "tauri:options": { application: APP } } },
});
const S = `/session/${session.sessionId}`;
const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";

const find = async (css) => {
  try {
    return (await wd("POST", `${S}/element`, { using: "css selector", value: css }))[ELEMENT];
  } catch {
    return null;
  }
};
const findAll = async (css) => (await wd("POST", `${S}/elements`, { using: "css selector", value: css })).map((e) => e[ELEMENT]);
const exec = (script, args = []) => wd("POST", `${S}/execute/sync`, { script, args });
async function waitFor(desc, fn, timeout = 15000) {
  const t0 = Date.now();
  for (;;) {
    const v = await fn().catch(() => null);
    if (v) return v;
    if (Date.now() - t0 > timeout) throw new Error(`Timed out waiting for: ${desc}`);
    await sleep(150);
  }
}
const waitEl = (css, timeout) => waitFor(`element ${css}`, () => find(css), timeout);
async function click(css) {
  const el = await waitEl(css);
  await wd("POST", `${S}/element/${el}/click`, {});
}
async function type(css, text) {
  const el = await waitEl(css);
  await wd("POST", `${S}/element/${el}/value`, { text });
}
// Monaco renders spaces as U+00A0; normalize so assertions can use plain text.
const textOf = (css) => exec(`const e = document.querySelector(arguments[0]); return e ? e.innerText.replace(/\u00a0/g, ' ') : null;`, [css]);
const pressShortcut = (key, opts = {}) =>
  exec(
    `window.dispatchEvent(new KeyboardEvent('keydown', { key: arguments[0], code: arguments[1], ctrlKey: true, shiftKey: !!arguments[2], bubbles: true }))`,
    [key, key.length === 1 ? `Key${key.toUpperCase()}` : key, opts.shift],
  );
let shotNo = 0;
async function shot(name) {
  const png = await wd("GET", `${S}/screenshot`);
  const file = path.join(OUT, `${String(++shotNo).padStart(2, "0")}-${name}.png`);
  fs.writeFileSync(file, Buffer.from(png, "base64"));
  console.log(`  📸 ${path.relative(ROOT, file)}`);
}
const ENTER = "\uE007"; // WebDriver key codes
const CTRL = "\uE009";
/** Type with W3C key actions (plain key events, no caret games on the focused element). */
async function keys(text, modifier) {
  const actions = [];
  if (modifier) actions.push({ type: "keyDown", value: modifier });
  for (const ch of text) {
    const k = ch === "\n" ? ENTER : ch;
    actions.push({ type: "keyDown", value: k }, { type: "keyUp", value: k });
  }
  if (modifier) actions.push({ type: "keyUp", value: modifier });
  await wd("POST", `${S}/actions`, { actions: [{ type: "key", id: "kbd", actions }] });
  await wd("DELETE", `${S}/actions`);
}
let failures = 0;
async function step(name, fn) {
  process.stdout.write(`• ${name}\n`);
  try {
    await fn();
  } catch (e) {
    failures++;
    console.error(`  ✗ ${e.message}`);
    await shot("FAILED").catch(() => {});
    throw e;
  }
}
const assert = (cond, msg) => {
  if (!cond) throw new Error(`Assertion failed: ${msg}`);
};
const requests = () =>
  fs.existsSync(mockLog)
    ? fs.readFileSync(mockLog, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l))
    : [];

async function sendChat(text) {
  await type("[data-testid=composer-input]", text);
  await type("[data-testid=composer-input]", ENTER);
}
const idle = () => waitFor("agent to finish", async () => !(await find("[data-testid=stop-button]")), 30000);

// ---------------------------------------------------------------------------- scenario
try {
  await step("app starts on the welcome screen", async () => {
    await waitEl("[data-testid=chat-panel]", 30000);
    await waitEl("[data-testid=recent-workspace]");
    const title = await textOf(".welcome h1");
    assert(title.includes("PiLunch"), `welcome title: ${title}`);
    await sleep(300);
    await shot("welcome");
  });

  await step("open the recent folder", async () => {
    await click("[data-testid=recent-workspace]");
    await waitEl('[data-path="src"]');
    await waitFor("git status in status bar", async () => (await textOf("[data-testid=statusbar]"))?.includes("main"));
    const readmeClass = await exec(`return document.querySelector('[data-path="README.md"] .name').className`);
    assert(readmeClass.includes("git-M"), `README.md should be marked modified (${readmeClass})`);
    await sleep(300);
    await shot("folder-open");
  });

  await step("chat streams a markdown answer", async () => {
    await sendChat("Say hello and show me some Rust");
    await waitFor("assistant answer", async () => (await textOf(".messages"))?.includes("Try asking me"));
    await idle();
    const md = await textOf(".messages");
    assert(md.includes("Hello from {name}!"), "code block rendered");
    assert(await find(".codeblock .hljs-keyword"), "code is syntax highlighted");
    assert(await find(".thinking"), "thinking block shown");
    const req = requests().at(-1);
    assert(req.body.model === "claude-opus-5-5", "default model");
    assert(req.body.stream === true && req.body.thinking?.type === "adaptive", "adaptive thinking + streaming");
    assert(req.body.tools.map((t) => t.name).join(",") === "read_file,list_dir,glob,grep,edit_file,write_file,run_command", "tools sent");
    assert(req.body.system[0].text.includes(ws), "system prompt has workspace root");
    await shot("chat-answer");
  });

  await step("clicking a path reference opens the editor", async () => {
    await click("code.path-link");
    await waitEl(".monaco-editor .view-lines", 20000);
    await waitFor("editor content", async () => (await textOf(".monaco-editor .view-lines"))?.includes("fn main()"));
    assert(await find('[data-testid=editor-tabs] [data-path="src/main.rs"]'), "tab opened");
    await sleep(400);
    await shot("editor-and-docked-chat");
  });

  await step("agent edit asks for approval with a diff", async () => {
    await sendChat("Please create a greeting file");
    await waitEl(".tool.pending", 20000);
    const diff = await textOf(".tool.pending .diff");
    assert(diff.includes("+Hello from PiLunch!"), `diff shows new content: ${diff}`);
    assert(!fs.existsSync(path.join(ws, "greet.txt")), "nothing written before approval");
    await shot("edit-approval");
    await click(".tool.pending .btn.primary");
    await waitFor("file written", async () => fs.existsSync(path.join(ws, "greet.txt")));
    await idle();
    assert(fs.readFileSync(path.join(ws, "greet.txt"), "utf8") === "Hello from PiLunch!\nMade by the agent.\n", "file content");
    await waitEl('[data-path="greet.txt"]');
    const second = requests().at(-1).body.messages;
    assert(second.at(-1).content[0].type === "tool_result", "tool result sent back");
    assert(second.at(-2).content[0].type === "thinking" && second.at(-2).content[0].signature === "sig-create", "thinking replayed with signature");
    await sleep(300);
    await shot("edit-applied");
  });

  await step("agent command runs after approval and streams output", async () => {
    await sendChat("run the checks");
    await waitEl(".tool.pending", 20000);
    const cmd = await textOf(".tool.pending .tool-body");
    assert(cmd.includes("echo e2e-ok"), `command shown: ${cmd}`);
    await shot("command-approval");
    await click(".tool.pending .btn.primary");
    await idle();
    await waitFor("command output in answer", async () => (await textOf(".messages"))?.includes("e2e-ok"));
    await shot("command-done");
  });

  await step("denying an edit sends feedback to Claude", async () => {
    await sendChat("create it again");
    await waitEl(".tool.pending", 20000);
    await click(".tool.pending .btn.danger");
    await type(".tool.pending .approval input", "Use a markdown file instead");
    await type(".tool.pending .approval input", ENTER);
    await idle();
    const res = requests().at(-1).body.messages.at(-1).content[0];
    assert(res.is_error === true && res.content.includes("Use a markdown file instead"), "feedback in tool_result");
  });

  await step("quick open (Ctrl+P) finds files by fuzzy name", async () => {
    await pressShortcut("p");
    await waitEl("[data-testid=palette] input");
    await type("[data-testid=palette] input", "libr");
    await waitFor("fuzzy result", async () => (await textOf(".palette-item.active"))?.includes("lib.rs"));
    await shot("quick-open");
    await type("[data-testid=palette] input", ENTER);
    await waitEl('[data-testid=editor-tabs] [data-path="src/lib.rs"]');
  });

  await step("search panel finds text across files", async () => {
    await pressShortcut("f", { shift: true });
    await type("[data-testid=search-input]", "greet");
    await waitFor("search hits", async () => (await findAll(".search-hit")).length >= 3);
    await shot("search");
  });

  await step("terminal runs a real shell", async () => {
    await pressShortcut("j");
    await waitEl(".xterm-helper-textarea", 20000);
    await sleep(800);
    // The WebGL renderer draws to a canvas, so check the shell's side effect on disk.
    await type(".xterm-helper-textarea", "echo pilunch-terminal-$((40+2)) | tee term-check.txt" + ENTER);
    await waitFor("terminal command ran", async () => fs.readFileSync(path.join(ws, "term-check.txt"), "utf8").trim() === "pilunch-terminal-42", 15000);
    await sleep(1200);
    await shot("terminal");
    await pressShortcut("j");
  });

  await step("editing and saving a file in Monaco", async () => {
    const before = fs.readFileSync(path.join(ws, "src/main.rs"), "utf8");
    await click('[data-testid=editor-tabs] [data-path="src/main.rs"]');
    await exec(`document.querySelector('.monaco-editor textarea').focus()`);
    // The cursor sits at 1:1 (revealed from the chat link earlier).
    await keys("// edited in PiLunch\n");
    await waitEl(".tab.active .dirty-dot");
    await keys("s", CTRL);
    const expected = "// edited in PiLunch\n" + before;
    await waitFor("saved to disk", async () => fs.readFileSync(path.join(ws, "src/main.rs"), "utf8") === expected);
    await waitFor("dirty dot cleared", async () => !(await find(".tab.active .dirty-dot")));
  });

  await step("settings dialog", async () => {
    await click("[data-testid=activity-settings]");
    await waitEl("[data-testid=settings]");
    await sleep(200);
    await shot("settings");
    await exec(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
  });

  await step("chat history lists the conversation", async () => {
    await click("[data-testid=activity-chats]");
    await waitFor("history entry", async () => (await textOf("[data-testid=chat-list]"))?.includes("Say hello"));
    await shot("chat-history");
  });

  await step("light theme", async () => {
    await exec(`document.documentElement.dataset.theme = 'light'; window.dispatchEvent(new CustomEvent('pilunch-theme'))`);
    await click("[data-testid=activity-explorer]");
    await sleep(500);
    await shot("light-theme");
  });

  console.log(`\n✅ All end-to-end steps passed (${shotNo} screenshots in e2e/out/)`);
} catch (e) {
  console.error(`\n❌ E2E failed: ${e.message}`);
  if (!failures) failures++;
} finally {
  await wd("DELETE", S).catch(() => {});
  cleanup();
  process.exit(failures ? 1 : 0);
}
