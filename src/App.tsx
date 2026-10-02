import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Icon, Logo } from "./components/Icon";
import { Splitter } from "./components/Splitter";
import { Explorer } from "./components/sidebar/Explorer";
import { SearchPanel } from "./components/sidebar/SearchPanel";
import { ChatList } from "./components/sidebar/ChatList";
import { EditorArea } from "./components/editor/EditorArea";
import { ChatPanel } from "./components/chat/ChatPanel";
import { Palette } from "./components/Palette";
import { SettingsModal } from "./components/SettingsModal";
import { StatusBar } from "./components/StatusBar";
import { ContextMenuHost } from "./components/ContextMenu";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { commandForEvent } from "./commands";
import { useApp, type SidebarView } from "./store/app";
import { useChat } from "./store/chat";
import { useEditor } from "./store/editor";
import { api } from "./lib/ipc";
import type { FsChanged } from "./lib/types";

function ActivityBar() {
  const layout = useApp((s) => s.layout);
  const running = useChat((s) => Object.values(s.runs).some((r) => r.running));
  const item = (view: SidebarView, icon: "files" | "search" | "chat", title: string) => (
    <button
      className={`icon-btn${layout.sidebarVisible && layout.sidebarView === view ? " active" : ""}`}
      title={title}
      onClick={() => useApp.getState().showSidebar(view)}
      data-testid={`activity-${view}`}
    >
      <Icon name={icon} size={20} />
      {view === "chats" && running && <span className="badge-dot" />}
    </button>
  );
  return (
    <div className="activity">
      <div className="logo" title="PiLunch">
        <Logo size={26} />
      </div>
      {item("explorer", "files", "Explorer (Ctrl+Shift+E)")}
      {item("search", "search", "Search (Ctrl+Shift+F)")}
      {item("chats", "chat", "Chats (Ctrl+Shift+H)")}
      <span className="spacer" />
      <button
        className={`icon-btn${layout.terminalVisible ? " active" : ""}`}
        title="Terminal (Ctrl+J)"
        onClick={() => useApp.getState().setLayout({ terminalVisible: !layout.terminalVisible })}
        data-testid="activity-terminal"
      >
        <Icon name="terminal" size={20} />
      </button>
      <button className="icon-btn" title="Settings (Ctrl+,)" onClick={() => useApp.getState().setOverlay("settings")} data-testid="activity-settings">
        <Icon name="settings" size={20} />
      </button>
    </div>
  );
}

function Toasts() {
  const toasts = useApp((s) => s.toasts);
  return (
    <div className="toasts">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          <Icon name={t.kind === "error" ? "alert" : "check"} size={15} style={{ color: t.kind === "error" ? "var(--err)" : "var(--ok)", flex: "none", marginTop: 1 }} />
          <span className="text">{t.text}</span>
          <button className="icon-btn" style={{ width: 20, height: 20 }} onClick={() => useApp.getState().dismissToast(t.id)}>
            <Icon name="x" size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}

// xterm is only loaded the first time the terminal is opened.
const TerminalPanel = lazy(() => import("./components/TerminalPanel").then((m) => ({ default: m.TerminalPanel })));

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

export function App() {
  const [ready, setReady] = useState(false);
  const layout = useApp((s) => s.layout);
  const overlay = useApp((s) => s.overlay);
  const workspace = useApp((s) => s.workspace);
  const hasTabs = useEditor((s) => s.tabs.length > 0);
  const [terminalMounted, setTerminalMounted] = useState(layout.terminalVisible);
  const base = useRef(0);

  useEffect(() => {
    void (async () => {
      await useApp.getState().init();
      // Runs left over from a reloaded webview have no listener anymore.
      if ((await api.agentRunning()).length) await api.agentCancelAll();
      await useChat.getState().loadList();
      setReady(true);
    })();
  }, []);

  useEffect(() => {
    if (layout.terminalVisible) setTerminalMounted(true);
  }, [layout.terminalVisible]);

  // Backend events.
  useEffect(() => {
    let gitTimer: number | undefined;
    const subs = [
      listen<FsChanged>("fs-changed", (e) => {
        const ev = e.payload;
        window.dispatchEvent(new CustomEvent("pilunch-fs", { detail: ev }));
        void useEditor.getState().onFsChanged(ev.paths);
        window.clearTimeout(gitTimer);
        gitTimer = window.setTimeout(() => void useApp.getState().refreshGit(), 400);
      }),
      listen<number>("index-ready", (e) => useApp.setState({ indexedFiles: e.payload })),
    ];
    return () => subs.forEach((p) => void p.then((u) => u()));
  }, []);

  // Global shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const cmd = commandForEvent(e);
      if (!cmd) return;
      if (cmd.needsWorkspace && !useApp.getState().workspace) return;
      if (useApp.getState().overlay && cmd.id !== "palette" && cmd.id !== "quickOpen") return;
      e.preventDefault();
      cmd.run();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Window title.
  useEffect(() => {
    document.title = workspace ? `${workspace.name} — PiLunch` : "PiLunch";
  }, [workspace]);

  if (!ready) return <div className="app" />;

  const showEditor = hasTabs && !layout.chatFocus;
  const setLayout = useApp.getState().setLayout;

  return (
    <div className="app">
      <div className="main">
        <ActivityBar />
        {layout.sidebarVisible && (
          <>
            <div className="sidebar" style={{ width: layout.sidebarWidth }}>
              <ErrorBoundary label="Sidebar">
                {layout.sidebarView === "explorer" && <Explorer />}
                {layout.sidebarView === "search" && <SearchPanel />}
                {layout.sidebarView === "chats" && <ChatList />}
              </ErrorBoundary>
            </div>
            <Splitter dir="v" onStart={() => (base.current = layout.sidebarWidth)} onDrag={(d) => setLayout({ sidebarWidth: clamp(base.current + d, 180, 560) })} />
          </>
        )}
        <div className="center">
          <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}><ErrorBoundary label={showEditor ? "Editor" : "Chat"}>{showEditor ? <EditorArea /> : <ChatPanel docked={false} />}</ErrorBoundary></div>
          {terminalMounted && (
            <div style={{ display: layout.terminalVisible ? "flex" : "none", flexDirection: "column" }}>
              <Splitter
                dir="h"
                onStart={() => (base.current = layout.terminalHeight)}
                onDrag={(d) => setLayout({ terminalHeight: clamp(base.current - d, 120, window.innerHeight - 200) })}
              />
              <Suspense fallback={<div className="terminal-panel" style={{ height: layout.terminalHeight }} />}>
                <TerminalPanel height={layout.terminalHeight} />
              </Suspense>
            </div>
          )}
        </div>
        {showEditor && (
          <>
            <Splitter
              dir="v"
              onStart={() => (base.current = layout.chatWidth)}
              onDrag={(d) => setLayout({ chatWidth: clamp(base.current - d, 320, window.innerWidth - 500) })}
            />
            <div className="chat-dock" style={{ width: layout.chatWidth }}>
              <ErrorBoundary label="Chat">
                <ChatPanel docked />
              </ErrorBoundary>
            </div>
          </>
        )}
      </div>
      <StatusBar />
      {overlay === "settings" && <SettingsModal />}
      {(overlay === "quickOpen" || overlay === "commands") && <Palette key={overlay} mode={overlay} />}
      <ContextMenuHost />
      <Toasts />
    </div>
  );
}
