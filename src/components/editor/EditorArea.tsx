import { useEffect, useRef, useState } from "react";
import type * as Monaco from "monaco-editor";
import { Icon } from "../Icon";
import { FileBadge } from "../sidebar/Explorer";
import { useEditor, loadMonaco, modelFor } from "../../store/editor";
import { useApp } from "../../store/app";
import { useChat } from "../../store/chat";
import { formatBytes } from "../../lib/util";

/** Send the editor selection (or the whole file) to the chat composer. */
export function addSelectionToChat(editor: Monaco.editor.ICodeEditor | null) {
  const { active } = useEditor.getState();
  if (!active) return;
  const sel = editor?.getSelection();
  const model = editor?.getModel();
  if (editor && sel && model && !sel.isEmpty()) {
    const text = model.getValueInRange(sel);
    const lang = model.getLanguageId();
    const range = sel.startLineNumber === sel.endLineNumber ? `${sel.startLineNumber}` : `${sel.startLineNumber}-${sel.endLineNumber}`;
    useChat.getState().insertText(`\`${active}:${range}\`\n\`\`\`${lang}\n${text}\n\`\`\`\n`);
  } else {
    useChat.getState().attach(active);
  }
}

let currentEditor: Monaco.editor.IStandaloneCodeEditor | null = null;
export function getActiveEditor() {
  return currentEditor;
}

function Tabs() {
  const tabs = useEditor((s) => s.tabs);
  const active = useEditor((s) => s.active);
  const { setActive, closeTab } = useEditor.getState();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector(".tab.active")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active]);
  return (
    <div className="tabs" ref={ref} data-testid="editor-tabs">
      {tabs.map((t) => (
        <div
          key={t.path}
          className={`tab${t.path === active ? " active" : ""}`}
          onClick={() => setActive(t.path)}
          onMouseDown={(e) => {
            if (e.button === 1) {
              e.preventDefault();
              void closeTab(t.path);
            }
          }}
          title={t.path}
          data-path={t.path}
        >
          <FileBadge name={t.name} />
          <span className="ellipsis">{t.name}</span>
          {t.dirty && <span className="dirty-dot" />}
          <span
            className="close"
            onClick={(e) => {
              e.stopPropagation();
              void closeTab(t.path);
            }}
          >
            <Icon name="x" size={12} />
          </span>
        </div>
      ))}
      <div className="tab-actions">
        <button className="icon-btn" title="Add selection to chat (Ctrl+L)" onClick={() => addSelectionToChat(currentEditor)}>
          <Icon name="at" size={15} />
        </button>
      </div>
    </div>
  );
}

export function EditorArea() {
  const tabs = useEditor((s) => s.tabs);
  const active = useEditor((s) => s.active);
  const reveal = useEditor((s) => s.reveal);
  const settings = useApp((s) => s.settings);
  const host = useRef<HTMLDivElement>(null);
  const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
  const shown = useRef<string | null>(null);
  const [ready, setReady] = useState(false);
  const tab = tabs.find((t) => t.path === active);

  // Create the editor once.
  useEffect(() => {
    let disposed = false;
    const disposables: Monaco.IDisposable[] = [];
    void loadMonaco().then(({ monaco, monacoTheme }) => {
      if (disposed || !host.current) return;
      const s = useApp.getState().settings;
      const editor = monaco.editor.create(host.current, {
        model: null,
        theme: monacoTheme(),
        automaticLayout: true,
        fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--mono"),
        fontSize: s?.editorFontSize ?? 14,
        fontLigatures: true,
        minimap: { enabled: s?.editorMinimap ?? true, renderCharacters: false },
        wordWrap: s?.editorWordWrap ? "on" : "off",
        smoothScrolling: true,
        cursorSmoothCaretAnimation: "on",
        cursorBlinking: "smooth",
        scrollBeyondLastLine: false,
        padding: { top: 10 },
        bracketPairColorization: { enabled: true },
        guides: { bracketPairs: "active", indentation: true },
        stickyScroll: { enabled: true },
        renderWhitespace: "selection",
        fixedOverflowWidgets: true,
      });
      editorRef.current = editor;
      currentEditor = editor;
      let raf = 0;
      disposables.push(
        editor.onDidChangeCursorSelection(() => {
          cancelAnimationFrame(raf);
          raf = requestAnimationFrame(() => {
            const pos = editor.getPosition();
            const model = editor.getModel();
            const sel = editor.getSelection();
            if (!pos || !model) return;
            const selected = sel && !sel.isEmpty() ? model.getValueLengthInRange(sel) : 0;
            useEditor.getState().setCursor({ line: pos.lineNumber, col: pos.column, selected }, model.getLanguageId());
          });
        }),
      );
      editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => void useEditor.getState().save());
      editor.addAction({
        id: "pilunch.addToChat",
        label: "Add Selection to Chat",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyL],
        contextMenuGroupId: "1_pilunch",
        run: (ed) => addSelectionToChat(ed),
      });
      const onTheme = () => monaco.editor.setTheme(monacoTheme());
      window.addEventListener("pilunch-theme", onTheme);
      disposables.push({ dispose: () => window.removeEventListener("pilunch-theme", onTheme) });
      setReady(true);
    });
    return () => {
      disposed = true;
      disposables.forEach((d) => d.dispose());
      if (shown.current) {
        const e = modelFor(shown.current);
        if (e && editorRef.current) e.viewState = editorRef.current.saveViewState();
      }
      editorRef.current?.dispose();
      editorRef.current = null;
      currentEditor = null;
      shown.current = null;
    };
  }, []);

  // Switch models when the active tab changes.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !ready) return;
    if (shown.current && shown.current !== active) {
      const prev = modelFor(shown.current);
      if (prev) prev.viewState = editor.saveViewState();
    }
    const entry = active && tab?.kind === "text" ? modelFor(active) : undefined;
    if (!entry) {
      editor.setModel(null);
      shown.current = null;
      return;
    }
    if (editor.getModel() !== entry.model) {
      editor.setModel(entry.model);
      if (entry.viewState) editor.restoreViewState(entry.viewState);
    }
    editor.updateOptions({ readOnly: tab?.readonly ?? false });
    shown.current = active;
    editor.focus();
  }, [active, ready, tab?.kind, tab?.readonly]);

  // Reveal a requested line.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !ready || !reveal || reveal.path !== active || editor.getModel() == null) return;
    editor.revealLineInCenter(reveal.line);
    editor.setPosition({ lineNumber: reveal.line, column: 1 });
    editor.focus();
  }, [reveal, active, ready]);

  // Live settings.
  useEffect(() => {
    editorRef.current?.updateOptions({
      fontSize: settings?.editorFontSize ?? 14,
      minimap: { enabled: settings?.editorMinimap ?? true, renderCharacters: false },
      wordWrap: settings?.editorWordWrap ? "on" : "off",
    });
  }, [settings?.editorFontSize, settings?.editorMinimap, settings?.editorWordWrap, ready]);

  const parts = active?.split("/") ?? [];

  return (
    <>
      <Tabs />
      {tab && (
        <div className="breadcrumbs">
          {parts.map((p, i) => (
            <span key={i} style={{ display: "flex", alignItems: "center", gap: 4 }}>
              {i > 0 && <Icon name="chevronRight" size={11} />}
              <span style={i === parts.length - 1 ? { color: "var(--text-2)" } : undefined}>{p}</span>
            </span>
          ))}
          {tab.readonly && tab.kind === "text" && <span className="muted"> — read-only (not valid UTF-8)</span>}
        </div>
      )}
      {tab?.externalChange && (
        <div className="editor-banner">
          <Icon name="alert" size={15} />
          <span style={{ flex: 1 }}>This file changed on disk while you had unsaved edits.</span>
          <button className="btn sm" onClick={() => void useEditor.getState().reload(tab.path)}>
            Reload from disk
          </button>
          <button className="btn sm" onClick={() => void useEditor.getState().save(tab.path)}>
            Keep mine & save
          </button>
        </div>
      )}
      <div className="editor-host">
        <div ref={host} style={{ position: "absolute", inset: 0, visibility: tab?.kind === "text" ? "visible" : "hidden" }} data-testid="monaco-host" />
        {tab && tab.kind !== "text" && (
          <div className="editor-placeholder">
            <div>
              <Icon name="file" size={36} />
              <p>{tab.kind === "binary" ? "Binary file" : "File is too large to open"}</p>
              <p className="muted">{formatBytes(tab.size)}</p>
            </div>
          </div>
        )}
      </div>
    </>
  );
}
