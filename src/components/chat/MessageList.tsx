import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon, Logo } from "../Icon";
import { Markdown } from "./Markdown";
import { ToolCard, WebSearchRow } from "./ToolCard";
import { useChat, type DraftBlock, type LiveRun } from "../../store/chat";
import { useApp } from "../../store/app";
import { useEditor } from "../../store/editor";
import { basename } from "../../lib/util";
import type { ContentBlock, StoredMessage, ToolUi } from "../../lib/types";

type Item = { kind: "user"; msg: StoredMessage; key: string } | { kind: "assistant"; msgs: StoredMessage[]; key: string };

function isToolResultsOnly(m: StoredMessage) {
  return m.role === "user" && m.content.length > 0 && m.content.every((b) => b.type === "tool_result");
}

function group(messages: StoredMessage[]): Item[] {
  const items: Item[] = [];
  messages.forEach((m, i) => {
    if (m.role === "user" && !isToolResultsOnly(m)) {
      items.push({ kind: "user", msg: m, key: `u${i}` });
      return;
    }
    if (isToolResultsOnly(m)) return; // results are shown inside the tool cards
    const last = items[items.length - 1];
    if (last?.kind === "assistant") last.msgs.push(m);
    else items.push({ kind: "assistant", msgs: [m], key: `a${i}` });
  });
  return items;
}

function userText(m: StoredMessage): string {
  if (m.display) return m.display.text;
  return m.content.map((b) => (b.type === "text" ? (b.text ?? "") : "")).join("\n");
}

const UserMessage = memo(function UserMessage({ msg }: { msg: StoredMessage }) {
  const attachments = msg.display?.attachments ?? [];
  return (
    <div className="msg-user">
      {attachments.length > 0 && (
        <div className="attach-chips">
          {attachments.map((a) => (
            <span key={a} className="chip" title={a} onClick={() => void useEditor.getState().openFile(a)}>
              <Icon name="file" size={12} />
              <span className="ellipsis">{basename(a)}</span>
            </span>
          ))}
        </div>
      )}
      <div className="bubble">{userText(msg)}</div>
    </div>
  );
});

function Thinking({ text, live }: { text: string; live?: boolean }) {
  const [open, setOpen] = useState(false);
  if (!live && !text.trim()) return null;
  return (
    <details className="thinking" open={open} onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary>
        <Icon name="bulb" size={13} />
        {live ? <span className="shimmer">Thinking…</span> : <span>Thought process</span>}
        <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
      </summary>
      {text.trim() && <div className="thinking-text">{text}</div>}
    </details>
  );
}

const AssistantBlocks = memo(function AssistantBlocks({ convId, msg, toolUi }: { convId: string; msg: StoredMessage; toolUi: Record<string, ToolUi> }) {
  return (
    <>
      {msg.content.map((b: ContentBlock, i) => {
        switch (b.type) {
          case "text":
            return b.text?.trim() ? <Markdown key={i} text={b.text} /> : null;
          case "thinking":
            return <Thinking key={i} text={b.thinking ?? ""} />;
          case "tool_use":
            return <ToolCard key={b.id} convId={convId} toolId={b.id!} name={b.name!} input={b.input} ui={toolUi[b.id!]} />;
          case "server_tool_use": {
            const result = msg.content.find((r) => r.type === "web_search_tool_result" && r.tool_use_id === b.id);
            const items = Array.isArray(result?.content) ? (result!.content as { url: string; title?: string }[]) : result ? [] : undefined;
            return <WebSearchRow key={b.id ?? i} query={String((b.input as { query?: string } | undefined)?.query ?? "")} results={items} />;
          }
          default:
            return null;
        }
      })}
    </>
  );
});

function DraftBlocks({ convId, draft, toolUi }: { convId: string; draft: DraftBlock[]; toolUi: Record<string, ToolUi> }) {
  return (
    <>
      {draft.map((b, i) => {
        if (b.kind === "text")
          return b.text ? (
            <div key={b.index}>
              <Markdown text={b.text} />
              {i === draft.length - 1 && <span className="streaming-caret" />}
            </div>
          ) : null;
        if (b.kind === "thinking") return <Thinking key={b.index} text={b.text} live />;
        if (b.kind === "server_tool_use") return <WebSearchRow key={b.index} query="…" />;
        if (b.kind === "tool_use" && b.toolId)
          return (
            <ToolCard
              key={b.toolId}
              convId={convId}
              toolId={b.toolId}
              name={b.toolName ?? "tool"}
              input={b.input}
              ui={toolUi[b.toolId]}
              streamingBytes={b.bytes}
              summary={b.summary}
            />
          );
        return null;
      })}
    </>
  );
}

function Working({ run }: { run: LiveRun }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => tick((x) => x + 1), 1000);
    return () => window.clearInterval(t);
  }, []);
  const secs = Math.floor((Date.now() - run.startedAt) / 1000);
  if (run.approvals.length) {
    return (
      <div className="working">
        <Icon name="shield" size={14} /> Waiting for your approval
      </div>
    );
  }
  return (
    <div className="working">
      <div className="spinner" />
      <span className="shimmer">{run.retrying ?? (run.draft.length ? "Working…" : "Thinking…")}</span>
      <span>{secs}s</span>
      <span className="muted">· Esc to stop</span>
    </div>
  );
}

function ModelNote({ msgs }: { msgs: StoredMessage[] }) {
  const configured = useApp((s) => s.settings?.model);
  const served = msgs.find((m) => m.model)?.model;
  if (!served || !configured || served === configured) return null;
  return <div className="model-note">Answered by {served}</div>;
}

export function MessageList({ convId }: { convId: string }) {
  const conv = useChat((s) => s.convs[convId]);
  const run = useChat((s) => s.runs[convId]);
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const items = useMemo(() => group(conv?.messages ?? []), [conv?.messages]);
  const toolUi = conv?.toolUi ?? {};
  const live = !!run?.running;
  const last = items[items.length - 1];
  const draftInLastGroup = live && last?.kind === "assistant";

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  });
  useEffect(() => {
    stick.current = true;
  }, [convId]);

  const onScroll = () => {
    const el = scroller.current;
    if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  };

  return (
    <div className="messages" ref={scroller} onScroll={onScroll}>
      <div className="messages-inner">
        {items.map((it, idx) =>
          it.kind === "user" ? (
            <UserMessage key={it.key} msg={it.msg} />
          ) : (
            <div key={it.key} className="msg-assistant">
              <div className="avatar">
                <Logo size={24} />
              </div>
              <div className="body">
                {it.msgs.map((m, i) => (
                  <AssistantBlocks key={i} convId={convId} msg={m} toolUi={toolUi} />
                ))}
                {draftInLastGroup && idx === items.length - 1 && run && <DraftBlocks convId={convId} draft={run.draft} toolUi={toolUi} />}
                {draftInLastGroup && idx === items.length - 1 && run && <Working run={run} />}
                <ModelNote msgs={it.msgs} />
              </div>
            </div>
          ),
        )}
        {live && !draftInLastGroup && run && (
          <div className="msg-assistant">
            <div className="avatar">
              <Logo size={24} />
            </div>
            <div className="body">
              <DraftBlocks convId={convId} draft={run.draft} toolUi={toolUi} />
              <Working run={run} />
            </div>
          </div>
        )}
        {run?.notices.map((n, i) => (
          <div key={i} className={`notice ${n.kind}`}>
            <Icon name={n.kind === "notice" ? "alert" : "alert"} size={15} />
            <div>
              {n.kind === "refusal" && <b>Claude declined: </b>}
              {n.text}
              {n.kind === "error" && /API key/i.test(n.text) && (
                <div style={{ marginTop: 8 }}>
                  <button className="btn sm" onClick={() => useApp.getState().setOverlay("settings")}>
                    <Icon name="key" size={13} /> Open settings
                  </button>
                </div>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
