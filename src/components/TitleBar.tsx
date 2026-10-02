import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon, Logo } from "./Icon";
import { useApp } from "../store/app";
import { useChat } from "../store/chat";

const win = (() => {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
})();

/** Frameless-window title bar: brand, context, command search, window controls. */
export function TitleBar() {
  const workspace = useApp((s) => s.workspace);
  const sidebarVisible = useApp((s) => s.layout.sidebarVisible);
  const title = useChat((s) => (s.activeId ? s.convs[s.activeId]?.title : undefined));
  const running = useChat((s) => Object.values(s.runs).some((r) => r.running));
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!win) return;
    const sync = () => void win.isMaximized().then(setMaximized).catch(() => {});
    sync();
    const un = win.onResized(sync);
    return () => void un.then((u) => u());
  }, []);

  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="tb-left" data-tauri-drag-region>
        <span className={`tb-logo${running ? " busy" : ""}`}>
          <Logo size={18} />
        </span>
        <button
          className={`icon-btn tb-btn${sidebarVisible ? " active" : ""}`}
          title="Toggle sidebar (Ctrl+B)"
          onClick={() => useApp.getState().setLayout({ sidebarVisible: !sidebarVisible })}
        >
          <Icon name="sidebar" size={15} />
        </button>
        <span className="tb-crumbs" data-tauri-drag-region>
          <span className="tb-app">PiLunch</span>
          {workspace && (
            <>
              <span className="tb-sep">/</span>
              <span className="tb-ws">{workspace.name}</span>
            </>
          )}
          {title && title !== "New chat" && (
            <>
              <span className="tb-sep">/</span>
              <span className="tb-title ellipsis">{title}</span>
            </>
          )}
        </span>
      </div>
      <button className="tb-search" onClick={() => useApp.getState().setOverlay(workspace ? "quickOpen" : "commands")} data-testid="titlebar-search">
        <Icon name="search" size={13} />
        <span>{workspace ? "Search files and commands" : "Search commands"}</span>
        <span className="kbd">Ctrl P</span>
      </button>
      <div className="tb-right" data-tauri-drag-region>
        <div className="win-controls">
          <button title="Minimize" onClick={() => void win?.minimize()}>
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path d="M1 5h8" stroke="currentColor" strokeWidth="1.2" />
            </svg>
          </button>
          <button title={maximized ? "Restore" : "Maximize"} onClick={() => void win?.toggleMaximize()}>
            {maximized ? (
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.1">
                <rect x="1.5" y="3" width="5.5" height="5.5" rx="1" />
                <path d="M3.5 3V2.2a.7.7 0 0 1 .7-.7h4.1a.7.7 0 0 1 .7.7v4.1a.7.7 0 0 1-.7.7H7" />
              </svg>
            ) : (
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.1">
                <rect x="1.5" y="1.5" width="7" height="7" rx="1.2" />
              </svg>
            )}
          </button>
          <button className="close" title="Close" onClick={() => void win?.close()}>
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" strokeWidth="1.2" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}
