import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Icon } from "./Icon";
import { useApp } from "../store/app";
import { api, errorText } from "../lib/ipc";
import type { BrowserOutcome } from "../lib/types";

const SPECIAL_KEYS = new Set([
  "Enter",
  "Tab",
  "Escape",
  "Backspace",
  "Delete",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "PageUp",
  "PageDown",
  "Home",
  "End",
]);

/**
 * The built-in browser: a live view of the Firefox session the agent drives. Clicks,
 * typing and scrolling on the screenshot are forwarded to Firefox.
 */
export function BrowserPanel() {
  const [state, setState] = useState<BrowserOutcome | null>(null);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const img = useRef<HTMLImageElement>(null);
  const typed = useRef("");
  const typeTimer = useRef<number | undefined>(undefined);
  const inflight = useRef(false);

  const act = useCallback(async (action: Record<string, unknown>, quiet = false) => {
    if (inflight.current && quiet) return;
    inflight.current = true;
    if (!quiet) setBusy(true);
    try {
      const out = await api.browserAction(action);
      setState(out);
      setError(null);
      if (action.action !== "screenshot" || !quiet) setUrl(out.url);
    } catch (e) {
      if (!quiet) setError(errorText(e));
    } finally {
      inflight.current = false;
      if (!quiet) setBusy(false);
    }
  }, []);

  // Live refresh while the session runs, and right after the agent acts.
  useEffect(() => {
    let alive = true;
    void api.browserRunning().then((r) => r && alive && void act({ action: "screenshot" }));
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void api.browserRunning().then((r) => r && void act({ action: "screenshot" }, true));
    }, 2000);
    const un = listen("browser-changed", () => void act({ action: "screenshot" }, true));
    return () => {
      alive = false;
      window.clearInterval(timer);
      void un.then((u) => u());
    };
  }, [act]);

  const navigate = (e?: React.FormEvent) => {
    e?.preventDefault();
    if (url.trim()) void act({ action: "navigate", url: url.trim() });
  };

  const click = (e: React.MouseEvent<HTMLImageElement>) => {
    const el = img.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const sx = el.naturalWidth / r.width;
    const sy = el.naturalHeight / r.height;
    void act({ action: "click", x: (e.clientX - r.left) * sx, y: (e.clientY - r.top) * sy });
  };

  const flushTyping = () => {
    const text = typed.current;
    typed.current = "";
    if (text) void act({ action: "type", text });
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (!state || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key.length === 1) {
      e.preventDefault();
      typed.current += e.key;
      window.clearTimeout(typeTimer.current);
      typeTimer.current = window.setTimeout(flushTyping, 350);
    } else if (SPECIAL_KEYS.has(e.key)) {
      e.preventDefault();
      flushTyping();
      void act({ action: "press", key: e.key });
    }
  };

  const onWheel = (e: React.WheelEvent) => {
    if (!state) return;
    void act({ action: "scroll", direction: e.deltaY < 0 ? "up" : "down", amount: Math.min(1200, Math.abs(e.deltaY) * 3) }, true);
  };

  return (
    <div className="browser-panel" data-testid="browser-panel">
      <form className="browser-bar" onSubmit={navigate}>
        <button type="button" className="icon-btn" title="Back" disabled={!state} onClick={() => void act({ action: "back" })}>
          <Icon name="arrowLeft" size={15} />
        </button>
        <button type="button" className="icon-btn" title="Forward" disabled={!state} onClick={() => void act({ action: "forward" })}>
          <Icon name="arrowRight" size={15} />
        </button>
        <button type="button" className="icon-btn" title="Reload" disabled={!state} onClick={() => void act({ action: "reload" })}>
          <Icon name="refresh" size={14} />
        </button>
        <input
          className="input"
          value={url}
          placeholder="Search or enter an address"
          onChange={(e) => setUrl(e.target.value)}
          data-testid="browser-url"
          spellCheck={false}
        />
        {busy && <div className="spinner" />}
        <button
          type="button"
          className="icon-btn"
          title="Close Firefox"
          disabled={!state}
          onClick={() => void api.browserAction({ action: "close" }).then(() => setState(null))}
        >
          <Icon name="stop" size={13} />
        </button>
        <button type="button" className="icon-btn" title="Hide browser" onClick={() => useApp.getState().setLayout({ browserVisible: false })}>
          <Icon name="x" size={15} />
        </button>
      </form>
      <div className="browser-view" tabIndex={0} onKeyDown={onKey} onWheel={onWheel}>
        {state?.screenshot ? (
          <img ref={img} src={state.screenshot} alt={state.title || "Browser"} onClick={click} draggable={false} />
        ) : (
          <div className="browser-empty">
            <Icon name="globe" size={30} />
            <b>Built-in browser</b>
            <span>
              Enter an address to start Firefox, or ask the agent to research something on the web. You'll see every page it visits here; click and type to take
              over.
            </span>
            {error && (
              <div className="onb-err" style={{ textAlign: "left" }}>
                {error}
              </div>
            )}
          </div>
        )}
      </div>
      {state && (
        <div className="browser-status">
          <span className="ellipsis">{state.title}</span>
          {error && <span style={{ color: "var(--err)" }}>{error}</span>}
          <span style={{ flex: 1 }} />
          <span>Firefox · click, type and scroll on the page</span>
        </div>
      )}
    </div>
  );
}
