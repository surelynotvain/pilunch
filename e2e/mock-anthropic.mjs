// A scripted stand-in for the Claude Messages API, used by the end-to-end test.
// Streams realistic SSE (thinking, text, tool_use) and logs every request it receives.
//
//   node e2e/mock-anthropic.mjs <port> <log-file>
import http from "node:http";
import fs from "node:fs";

const port = Number(process.argv[2] ?? 8787);
const logFile = process.argv[3] ?? "mock-requests.jsonl";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function lastUserText(messages) {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.role !== "user") continue;
    const blocks = typeof m.content === "string" ? [{ type: "text", text: m.content }] : m.content;
    const results = blocks.filter((b) => b.type === "tool_result");
    if (results.length) return { toolResults: results };
    return { text: blocks.filter((b) => b.type === "text").map((b) => b.text).join("\n") };
  }
  return { text: "" };
}

/** Decide the next assistant turn: a list of content blocks + stop reason. */
function script(body) {
  const last = lastUserText(body.messages);
  if (last.toolResults) {
    const r = last.toolResults[0];
    const content = typeof r.content === "string" ? r.content : JSON.stringify(r.content);
    if (r.is_error) return { blocks: [{ type: "text", text: `Understood — that didn't go through:\n\n> ${content.split("\n")[0]}\n\nTell me how you'd like to proceed.` }], stop: "end_turn" };
    if (content.startsWith("Saved skill")) {
      return { blocks: [{ type: "text", text: "Saved — I'll follow **release-steps** next time you ask for a release." }], stop: "end_turn" };
    }
    if (content.startsWith("Exit code:")) {
      return {
        blocks: [{ type: "text", text: `The command finished. Here's the output:\n\n\`\`\`text\n${content.split("\n").slice(1).join("\n").trim()}\n\`\`\`\n\nEverything looks good.` }],
        stop: "end_turn",
      };
    }
    return {
      blocks: [{ type: "text", text: "Done! I created **`greet.txt:1`** with a friendly greeting.\n\n- It's a plain text file\n- You can open it from the explorer" }],
      stop: "end_turn",
    };
  }
  const t = (last.text ?? "").toLowerCase();
  if (t.includes("skill")) {
    return {
      blocks: [
        { type: "text", text: "I'll save that procedure as a skill." },
        {
          type: "tool_use",
          id: "toolu_skill",
          name: "skill_save",
          input: { name: "release-steps", description: "How to cut a release of the demo project", content: "1. Bump the version in Cargo.toml\n2. Run cargo test\n3. Tag and push" },
        },
      ],
      stop: "tool_use",
    };
  }
  if (t.includes("create")) {
    return {
      blocks: [
        { type: "thinking", thinking: "The user wants a greeting file. I'll write greet.txt at the project root.", signature: "sig-create" },
        { type: "text", text: "I'll create a small greeting file for you." },
        { type: "tool_use", id: "toolu_create", name: "write_file", input: { path: "greet.txt", content: "Hello from PiLunch!\nMade by the agent.\n" } },
      ],
      stop: "tool_use",
    };
  }
  if (t.includes("run")) {
    return {
      blocks: [
        { type: "text", text: "Let me run the project's check script." },
        { type: "tool_use", id: "toolu_run", name: "run_command", input: { command: "echo 'running checks…' && ls && echo e2e-ok" } },
      ],
      stop: "tool_use",
    };
  }
  if (t.includes("read")) {
    return {
      blocks: [{ type: "tool_use", id: "toolu_read", name: "read_file", input: { path: "src/main.rs" } }],
      stop: "tool_use",
    };
  }
  return {
    blocks: [
      { type: "thinking", thinking: "A friendly greeting with a short Rust example should do.", signature: "sig-hello" },
      {
        type: "text",
        text:
          "Hi! I'm **PiLunch**, your AI pair programmer. I can read, edit and run code in this folder — always asking first.\n\n" +
          "Here's a tiny Rust example:\n\n```rust\nfn main() {\n    let name = \"PiLunch\";\n    println!(\"Hello from {name}!\");\n}\n```\n\n" +
          "Try asking me to *create a file* or *run the tests*. The entry point is `src/main.rs:1`.",
      },
    ],
    stop: "end_turn",
  };
}

function* events(turn, model) {
  yield { type: "message_start", message: { id: "msg_mock", type: "message", role: "assistant", model, content: [], usage: { input_tokens: 1200, cache_read_input_tokens: 3400, cache_creation_input_tokens: 0, output_tokens: 1 } } };
  let index = 0;
  for (const b of turn.blocks) {
    if (b.type === "thinking") {
      yield { type: "content_block_start", index, content_block: { type: "thinking", thinking: "", signature: "" } };
      for (const piece of b.thinking.match(/.{1,12}/gs)) yield { type: "content_block_delta", index, delta: { type: "thinking_delta", thinking: piece } };
      yield { type: "content_block_delta", index, delta: { type: "signature_delta", signature: b.signature } };
    } else if (b.type === "text") {
      yield { type: "content_block_start", index, content_block: { type: "text", text: "" } };
      for (const piece of b.text.match(/.{1,6}/gs)) yield { type: "content_block_delta", index, delta: { type: "text_delta", text: piece } };
    } else if (b.type === "tool_use") {
      yield { type: "content_block_start", index, content_block: { type: "tool_use", id: b.id, name: b.name, input: {} } };
      const json = JSON.stringify(b.input);
      for (const piece of json.match(/.{1,10}/gs)) yield { type: "content_block_delta", index, delta: { type: "input_json_delta", partial_json: piece } };
    }
    yield { type: "content_block_stop", index };
    index++;
  }
  yield { type: "message_delta", delta: { stop_reason: turn.stop, stop_sequence: null }, usage: { output_tokens: 180 } };
  yield { type: "message_stop" };
}

const server = http.createServer(async (req, res) => {
  if (req.method === "GET" && req.url?.startsWith("/v1/models")) {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ data: [{ id: "claude-opus-5-5", display_name: "Claude Opus 5.5" }, { id: "claude-sonnet-5-5", display_name: "Claude Sonnet 5.5" }] }));
    return;
  }
  if (req.method !== "POST" || !req.url?.startsWith("/v1/messages")) {
    res.writeHead(404).end();
    return;
  }
  let raw = "";
  for await (const chunk of req) raw += chunk;
  const body = JSON.parse(raw);
  fs.appendFileSync(logFile, JSON.stringify({ headers: req.headers, body }) + "\n");
  if (req.headers["x-api-key"] !== "test-key") {
    res.writeHead(401, { "content-type": "application/json" });
    res.end(JSON.stringify({ type: "error", error: { type: "authentication_error", message: "invalid x-api-key" } }));
    return;
  }
  res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
  for (const ev of events(script(body), body.model)) {
    res.write(`event: ${ev.type}\ndata: ${JSON.stringify(ev)}\n\n`);
    await sleep(ev.type === "content_block_delta" ? 12 : 30);
  }
  res.end();
});

server.listen(port, "127.0.0.1", () => console.log(`mock Anthropic API on http://127.0.0.1:${port}`));
