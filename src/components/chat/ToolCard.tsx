import { memo, useEffect, useRef, useState } from "react";
import { Icon, type IconName } from "../Icon";
import { useChat, type Approval } from "../../store/chat";
import { useEditor } from "../../store/editor";
import { formatBytes } from "../../lib/util";
import type { ToolUi } from "../../lib/types";

const TOOL_ICON: Record<string, IconName> = {
  read_file: "eye",
  list_dir: "list",
  glob: "search",
  grep: "search",
  edit_file: "edit",
  write_file: "file",
  run_command: "terminal",
  multi_edit: "edit",
  find_replace: "refresh",
  create_directory: "folder",
  move_path: "chevronRight",
  delete_path: "trash",
  read_many_files: "files",
  file_info: "file",
  git_status: "branch",
  git_diff: "branch",
  git_log: "branch",
  web_fetch: "globe",
  todo_write: "list",
  skill_load: "book",
  skill_save: "book",
  browser: "globe",
  computer: "monitor",
};

/** Icon for a tool, including generated names (MCP, custom tools). */
export function toolIcon(name: string): IconName {
  return TOOL_ICON[name] ?? (name.startsWith("mcp__") ? "plug" : "wrench");
}

export function DiffView({ diff }: { diff: string }) {
  const lines = diff.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  return (
    <div className="diff">
      {lines.map((l, i) => {
        const cls = l.startsWith("+++") || l.startsWith("---") ? "meta" : l.startsWith("@@") ? "hunk" : l.startsWith("+") ? "add" : l.startsWith("-") ? "del" : "";
        return (
          <div key={i} className={cls}>
            {l || " "}
          </div>
        );
      })}
    </div>
  );
}

function LiveOutput({ text }: { text: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [text]);
  return (
    <div className="tool-body" ref={ref}>
      <pre>{text || " "}</pre>
    </div>
  );
}

const APPROVAL_COPY: Record<Approval["kind"], { question: string; allow: string; session: string; prefix: string }> = {
  edit: { question: "Apply this change?", allow: "Apply", session: "Apply all edits in this chat", prefix: "" },
  command: { question: "Run this command?", allow: "Run", session: "Allow all commands in this chat", prefix: "$ " },
  network: { question: "Open this URL?", allow: "Allow", session: "Allow all requests in this chat", prefix: "GET " },
  tool: { question: "Call this tool?", allow: "Call", session: "Allow all tool calls in this chat", prefix: "" },
  computer: { question: "Let the agent control your computer?", allow: "Allow", session: "Allow computer use in this chat", prefix: "" },
};

function ApprovalBox({ convId, toolId }: { convId: string; toolId: string }) {
  const approval = useChat((s) => s.runs[convId]?.approvals.find((a) => a.toolId === toolId));
  const respond = useChat((s) => s.respond);
  const [feedback, setFeedback] = useState("");
  const [showFeedback, setShowFeedback] = useState(false);
  if (!approval) return null;
  const isEdit = approval.kind === "edit";
  const copy = APPROVAL_COPY[approval.kind];
  return (
    <>
      <div className="tool-body">{isEdit ? <DiffView diff={approval.detail} /> : <pre>{copy.prefix}{approval.detail}</pre>}</div>
      <div className="approval">
        <div className="question">
          <Icon name="shield" size={15} />
          {copy.question}
        </div>
        {showFeedback && (
          <input
            className="input"
            autoFocus
            placeholder="Tell Claude what to do instead (optional)"
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void respond(approval.approvalId, "deny", feedback);
            }}
          />
        )}
        <div className="row">
          <button className="btn primary sm" onClick={() => void respond(approval.approvalId, "allow")}>
            <Icon name="check" size={14} /> {copy.allow}
          </button>
          <button className="btn sm" onClick={() => void respond(approval.approvalId, "allowSession")}>
            {copy.session}
          </button>
          <span style={{ flex: 1 }} />
          {showFeedback ? (
            <button className="btn sm danger" onClick={() => void respond(approval.approvalId, "deny", feedback)}>
              Deny
            </button>
          ) : (
            <button className="btn sm danger" onClick={() => setShowFeedback(true)}>
              <Icon name="x" size={14} /> Deny…
            </button>
          )}
        </div>
      </div>
    </>
  );
}

interface Props {
  convId: string;
  toolId: string;
  name: string;
  input?: Record<string, unknown>;
  /** Final/known UI state (from history or live status events). */
  ui?: ToolUi;
  /** Draft-only info while the tool input is still streaming. */
  streamingBytes?: number;
  summary?: string;
}

export const ToolCard = memo(function ToolCard({ convId, toolId, name, input, ui, streamingBytes, summary }: Props) {
  const pending = useChat((s) => !!s.runs[convId]?.approvals.some((a) => a.toolId === toolId));
  const liveOutput = useChat((s) => s.runs[convId]?.outputs[toolId]);
  const running = ui?.status === "running";
  const [open, setOpen] = useState(false);
  const status = ui?.status;
  const title =
    ui?.summary ??
    summary ??
    (streamingBytes ? `${name.replace("_", " ")} — writing ${formatBytes(streamingBytes)}…` : `${name.replace("_", " ")}…`);
  const detail = ui?.detail;
  const hasBody = !!detail || (running && liveOutput != null) || (!running && liveOutput && !detail);
  const path = ui?.path ?? (typeof input?.path === "string" ? (input.path as string) : undefined);

  let statusEl;
  if (pending) statusEl = <Icon name="shield" size={14} className="status-muted" />;
  else if (!status || status === "running") statusEl = <div className="spinner" />;
  else if (status === "done") statusEl = <Icon name="check" size={14} className="status-ok" />;
  else if (status === "denied" || status === "cancelled") statusEl = <Icon name="x" size={14} className="status-muted" />;
  else statusEl = <Icon name="alert" size={14} className="status-err" />;

  return (
    <div className={`tool${pending ? " pending" : ""}`} data-tool={name}>
      <div className="tool-head" onClick={() => hasBody && setOpen(!open)}>
        {statusEl}
        <Icon name={toolIcon(name)} size={14} />
        <span className="summary ellipsis" title={title}>
          {title}
          {status === "denied" && <span className="muted"> — denied</span>}
          {status === "cancelled" && <span className="muted"> — cancelled</span>}
        </span>
        {path && status === "done" && name !== "list_dir" && (
          <button
            className="icon-btn"
            title="Open in editor"
            onClick={(e) => {
              e.stopPropagation();
              void useEditor.getState().openFile(path);
            }}
          >
            <Icon name="file" size={13} />
          </button>
        )}
        {hasBody && !pending && <Icon name={open || running ? "chevronDown" : "chevronRight"} size={14} />}
      </div>
      {pending ? (
        <ApprovalBox convId={convId} toolId={toolId} />
      ) : running && liveOutput != null ? (
        <LiveOutput text={liveOutput} />
      ) : open && detail ? (
        <div className="tool-body">
          {ui?.detailKind === "diff" ? <DiffView diff={detail} /> : ui?.detailKind === "image" ? <img className="tool-shot" src={detail} alt="Screenshot" /> : <pre>{detail}</pre>}
        </div>
      ) : open && liveOutput ? (
        <LiveOutput text={liveOutput} />
      ) : null}
    </div>
  );
});

/** Anthropic server-side web search, rendered from the server_tool_use + result blocks. */
export function WebSearchRow({ query, results }: { query: string; results?: { url: string; title?: string }[] }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="tool" data-tool="web_search">
      <div className="tool-head" onClick={() => results?.length && setOpen(!open)}>
        {results ? <Icon name="check" size={14} className="status-ok" /> : <div className="spinner" />}
        <Icon name="globe" size={14} />
        <span className="summary ellipsis">
          Searched the web for “{query}”{results ? ` — ${results.length} results` : "…"}
        </span>
        {!!results?.length && <Icon name={open ? "chevronDown" : "chevronRight"} size={14} />}
      </div>
      {open && results && (
        <div className="sources">
          {results.map((r) => (
            <a key={r.url} className="source" href={r.url} title={r.url} onClick={(e) => { e.preventDefault(); void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(r.url)); }}>
              <span className="ellipsis">{r.title || r.url}</span>
              <span className="host">{(() => { try { return new URL(r.url).hostname; } catch { return ""; } })()}</span>
            </a>
          ))}
        </div>
      )}
    </div>
  );
}
