import { memo, useEffect, useRef, useState } from "react";
import { Icon, type IconName } from "../Icon";
import { useChat } from "../../store/chat";
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
};

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

function ApprovalBox({ convId, toolId }: { convId: string; toolId: string }) {
  const approval = useChat((s) => s.runs[convId]?.approvals.find((a) => a.toolId === toolId));
  const respond = useChat((s) => s.respond);
  const [feedback, setFeedback] = useState("");
  const [showFeedback, setShowFeedback] = useState(false);
  if (!approval) return null;
  const isEdit = approval.kind === "edit";
  return (
    <>
      <div className="tool-body">{isEdit ? <DiffView diff={approval.detail} /> : <pre>$ {approval.detail}</pre>}</div>
      <div className="approval">
        <div className="question">
          <Icon name="shield" size={15} />
          {isEdit ? "Apply this change?" : "Run this command?"}
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
            <Icon name="check" size={14} /> {isEdit ? "Apply" : "Run"}
          </button>
          <button className="btn sm" onClick={() => void respond(approval.approvalId, "allowSession")}>
            {isEdit ? "Apply all edits in this chat" : "Allow all commands in this chat"}
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
        <Icon name={TOOL_ICON[name] ?? "wrench"} size={14} />
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
        <div className="tool-body">{ui?.detailKind === "diff" ? <DiffView diff={detail} /> : <pre>{detail}</pre>}</div>
      ) : open && liveOutput ? (
        <LiveOutput text={liveOutput} />
      ) : null}
    </div>
  );
});
