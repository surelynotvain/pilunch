import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Icon } from "../Icon";
import { useChat } from "../../store/chat";
import { useApp } from "../../store/app";
import { api } from "../../lib/ipc";
import { basename, dirname } from "../../lib/util";
import type { Effort, FileMatch, PermissionMode } from "../../lib/types";
import { EFFORTS, MODELS, activeModel, modelPatch, providerReady } from "../../lib/models";

const MODES: { id: PermissionMode; label: string; title: string }[] = [
  { id: "ask", label: "Ask before edits", title: "Claude asks before every file edit and command" },
  { id: "acceptEdits", label: "Auto-accept edits", title: "File edits apply immediately; commands still ask" },
  { id: "plan", label: "Plan mode", title: "Read-only: Claude explores and proposes a plan" },
  { id: "bypass", label: "Bypass approvals", title: "Edits and commands run without asking" },
];

function Highlighted({ text, indices, offset }: { text: string; indices: number[]; offset: number }) {
  const set = new Set(indices.map((i) => i - offset));
  return (
    <>
      {[...text].map((ch, i) => (set.has(i) ? <span key={i} className="hl">{ch}</span> : ch))}
    </>
  );
}

export function Composer({ convId }: { convId: string | null }) {
  const composer = useChat((s) => s.composer);
  const running = useChat((s) => (convId ? !!s.runs[convId]?.running : false));
  const { setComposer, send, cancel } = useChat.getState();
  const workspace = useApp((s) => s.workspace);
  const mode = useApp((s) => s.settings?.permissionMode ?? "ask");
  const hasKey = useApp((s) => (s.settings ? providerReady(s.settings) : false));
  const provider = useApp((s) => s.settings?.provider ?? "anthropic");
  const model = useApp((s) => (s.settings ? activeModel(s.settings) : ""));
  const [remote, setRemote] = useState<{ id: string; label: string }[] | null>(null);
  useEffect(() => setRemote(null), [provider]);
  const loadRemote = () => {
    if (provider === "anthropic" || remote) return;
    api.listModels(provider).then(
      (ms) => setRemote(ms.map((m) => ({ id: m.id, label: m.displayName }))),
      () => setRemote([]),
    );
  };
  const choices = provider === "anthropic" ? MODELS : (remote ?? []);
  const effort = useApp((s) => s.settings?.effort ?? "high");
  const ta = useRef<HTMLTextAreaElement>(null);
  const [mention, setMention] = useState<{ start: number; query: string } | null>(null);
  const [matches, setMatches] = useState<FileMatch[]>([]);
  const [sel, setSel] = useState(0);

  // autosize
  useLayoutEffect(() => {
    const el = ta.current;
    if (!el) return;
    el.style.height = "0px";
    el.style.height = `${Math.min(el.scrollHeight, window.innerHeight * 0.4)}px`;
  }, [composer.text]);

  useEffect(() => {
    if (composer.focusSeq) ta.current?.focus();
  }, [composer.focusSeq]);

  useEffect(() => {
    if (!mention || !workspace) {
      setMatches([]);
      return;
    }
    let alive = true;
    void api.quickOpen(mention.query, 12).then((m) => {
      if (alive) {
        setMatches(m);
        setSel(0);
      }
    });
    return () => {
      alive = false;
    };
  }, [mention, workspace]);

  const detectMention = (value: string, caret: number) => {
    const before = value.slice(0, caret);
    const m = /(^|\s)@([^\s@]*)$/.exec(before);
    setMention(m && workspace ? { start: caret - m[2]!.length - 1, query: m[2]! } : null);
  };

  const pickMention = (path: string) => {
    if (!mention) return;
    const el = ta.current!;
    const caret = el.selectionStart;
    const text = composer.text.slice(0, mention.start) + composer.text.slice(caret);
    setComposer({ text, attachments: composer.attachments.includes(path) ? composer.attachments : [...composer.attachments, path] });
    setMention(null);
    requestAnimationFrame(() => {
      el.focus();
      el.setSelectionRange(mention.start, mention.start);
    });
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (mention && matches.length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSel((sel + 1) % matches.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSel((sel - 1 + matches.length) % matches.length);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        pickMention(matches[sel]!.path);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setMention(null);
        return;
      }
    }
    if (e.key === "Escape" && running) {
      e.preventDefault();
      cancel(convId ?? undefined);
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (!running) void send();
    }
  };

  const canSend = !running && composer.text.trim().length > 0;
  const placeholder = !hasKey
    ? "Connect a model provider in Settings to start…"
    : workspace
      ? `Ask PiLunch to build, fix or explain anything in ${workspace.name}…  (@ to mention files)`
      : "Ask anything… (open a folder to let PiLunch work on code)";

  return (
    <div className="composer-wrap">
      <div className="composer">
        {mention && matches.length > 0 && (
          <div className="mention-popup">
            {matches.map((m, i) => {
              const name = basename(m.path);
              const dir = dirname(m.path);
              return (
                <div
                  key={m.path}
                  className={`palette-item${i === sel ? " active" : ""}`}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    pickMention(m.path);
                  }}
                  onMouseEnter={() => setSel(i)}
                >
                  <Icon name="file" size={14} />
                  <span>
                    <Highlighted text={name} indices={m.indices} offset={m.path.length - name.length} />
                  </span>
                  <span className="dir">{dir}</span>
                </div>
              );
            })}
          </div>
        )}
        {composer.attachments.length > 0 && (
          <div className="attach-chips">
            {composer.attachments.map((a) => (
              <span key={a} className="chip" title={a}>
                <Icon name="file" size={12} />
                <span className="ellipsis">{basename(a)}</span>
                <span className="x" onClick={() => setComposer({ attachments: composer.attachments.filter((x) => x !== a) })}>
                  <Icon name="x" size={12} />
                </span>
              </span>
            ))}
          </div>
        )}
        <textarea
          ref={ta}
          rows={1}
          value={composer.text}
          placeholder={placeholder}
          spellCheck
          onChange={(e) => {
            setComposer({ text: e.target.value });
            detectMention(e.target.value, e.target.selectionStart);
          }}
          onKeyDown={onKeyDown}
          onBlur={() => setTimeout(() => setMention(null), 120)}
          data-testid="composer-input"
        />
        <div className="composer-bar">
          {workspace && (
            <button
              className="icon-btn"
              title="Attach files (@)"
              onClick={() => {
                const el = ta.current!;
                const pos = el.selectionStart;
                const t = composer.text;
                const ins = (pos > 0 && !/\s/.test(t[pos - 1]!) ? " " : "") + "@";
                setComposer({ text: t.slice(0, pos) + ins + t.slice(pos) });
                requestAnimationFrame(() => {
                  el.focus();
                  el.setSelectionRange(pos + ins.length, pos + ins.length);
                  detectMention(el.value, pos + ins.length);
                });
              }}
            >
              <Icon name="paperclip" size={16} />
            </button>
          )}
          {workspace && (
            <select
              className={`mode-select ${mode}`}
              value={mode}
              title={MODES.find((m) => m.id === mode)?.title}
              onChange={(e) => void useApp.getState().updateSettings({ permissionMode: e.target.value as PermissionMode })}
              data-testid="mode-select"
            >
              {MODES.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </select>
          )}
          <span className="spacer" />
          <select
            className="pill-select"
            value={model}
            title="Model"
            onChange={(e) => void useApp.getState().updateSettings(modelPatch(provider, e.target.value))}
            onMouseDown={loadRemote}
            onFocus={loadRemote}
            data-testid="model-select"
          >
            {(choices.some((m) => m.id === model) ? choices : [{ id: model, label: model || "No model" }, ...choices]).map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>
          <select
            className="pill-select"
            value={effort}
            title="Thinking effort"
            onChange={(e) => void useApp.getState().updateSettings({ effort: e.target.value as Effort })}
          >
            {EFFORTS.map((x) => (
              <option key={x.id} value={x.id}>
                {x.label}
              </option>
            ))}
          </select>
          {running ? (
            <button className="send-btn stop" title="Stop (Esc)" onClick={() => cancel(convId ?? undefined)} data-testid="stop-button">
              <Icon name="stop" size={14} />
            </button>
          ) : (
            <button className="send-btn" title="Send (Enter)" disabled={!canSend} onClick={() => void send()} data-testid="send-button">
              <Icon name="send" size={16} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
