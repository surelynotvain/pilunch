import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Icon } from "./Icon";
import { api, errorText } from "../lib/ipc";
import { useApp } from "../store/app";

function xtermTheme() {
  const css = getComputedStyle(document.documentElement);
  const v = (n: string) => css.getPropertyValue(n).trim();
  const light = document.documentElement.dataset.theme === "light";
  return {
    background: v("--bg-1"),
    foreground: v("--text"),
    cursor: v("--accent"),
    selectionBackground: light ? "#c9c1ff" : "#3b3f6b",
    black: light ? "#1b1e25" : "#1b1e25",
    brightBlack: "#6c7385",
    red: "#f0565c",
    green: "#3fbf86",
    yellow: "#e5b440",
    blue: "#4aa3ff",
    magenta: "#a78bfa",
    cyan: "#2ac3de",
    white: light ? "#4c5466" : "#c8ccd6",
    brightWhite: light ? "#1b1e25" : "#ffffff",
  };
}

interface Session {
  key: number;
  title: string;
}

/** One xterm instance bound to one PTY. Stays mounted (hidden) when not the active tab. */
function TermView({ visible, onExit }: { visible: boolean; onExit: () => void }) {
  const host = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const termRef = useRef<Terminal | null>(null);

  useEffect(() => {
    const el = host.current!;
    const s = useApp.getState().settings;
    const term = new Terminal({
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--mono"),
      fontSize: Math.max(11, (s?.editorFontSize ?? 14) - 1),
      cursorBlink: true,
      allowProposedApi: true,
      scrollback: 10000,
      theme: xtermTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon((_e, uri) => void openUrl(uri)));
    term.open(el);
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => gl.dispose());
      term.loadAddon(gl);
    } catch {
      /* WebGL unavailable: the DOM renderer is used */
    }
    fitRef.current = fit;
    termRef.current = term;
    fit.fit();

    let id: number | null = null;
    let disposed = false;
    const unlisten = listen<number>("terminal-exit", (e) => {
      if (e.payload === id) onExit();
    });
    api
      .terminalSpawn(term.cols, term.rows, (data) => term.write(data))
      .then((tid) => {
        if (disposed) {
          void api.terminalKill(tid);
          return;
        }
        id = tid;
      })
      .catch((e) => term.write(`\r\n\x1b[31mCould not start a shell: ${errorText(e)}\x1b[0m\r\n`));
    const onData = term.onData((d) => {
      if (id != null) void api.terminalWrite(id, d);
    });
    const onResize = term.onResize(({ cols, rows }) => {
      if (id != null) void api.terminalResize(id, cols, rows);
    });
    const ro = new ResizeObserver(() => {
      if (el.offsetParent !== null) fit.fit();
    });
    ro.observe(el);
    const onTheme = () => (term.options.theme = xtermTheme());
    window.addEventListener("pilunch-theme", onTheme);
    return () => {
      disposed = true;
      ro.disconnect();
      onData.dispose();
      onResize.dispose();
      window.removeEventListener("pilunch-theme", onTheme);
      void unlisten.then((u) => u());
      if (id != null) void api.terminalKill(id);
      term.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (visible) {
      requestAnimationFrame(() => {
        fitRef.current?.fit();
        termRef.current?.focus();
      });
    }
  }, [visible]);

  return <div ref={host} className="terminal-instance" style={{ visibility: visible ? "visible" : "hidden" }} />;
}

let nextKey = 1;

export function TerminalPanel({ height }: { height: number }) {
  const [sessions, setSessions] = useState<Session[]>(() => [{ key: nextKey, title: `Terminal ${nextKey++}` }]);
  const [active, setActive] = useState(sessions[0]!.key);

  const add = () => {
    const s = { key: nextKey, title: `Terminal ${nextKey++}` };
    setSessions((x) => [...x, s]);
    setActive(s.key);
  };
  const close = (key: number) => {
    setSessions((x) => {
      const rest = x.filter((s) => s.key !== key);
      if (rest.length === 0) useApp.getState().setLayout({ terminalVisible: false });
      if (key === active && rest.length) setActive(rest[rest.length - 1]!.key);
      return rest;
    });
  };

  useEffect(() => {
    if (sessions.length === 0) add();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions.length]);

  return (
    <div className="terminal-panel" style={{ height }} data-testid="terminal-panel">
      <div className="terminal-tabs">
        {sessions.map((s) => (
          <div key={s.key} className={`terminal-tab${s.key === active ? " active" : ""}`} onClick={() => setActive(s.key)}>
            <Icon name="terminal" size={13} />
            {s.title}
            <span
              className="icon-btn"
              style={{ width: 18, height: 18 }}
              onClick={(e) => {
                e.stopPropagation();
                close(s.key);
              }}
            >
              <Icon name="x" size={11} />
            </span>
          </div>
        ))}
        <button className="icon-btn" title="New terminal" onClick={add}>
          <Icon name="plus" size={15} />
        </button>
        <span style={{ flex: 1 }} />
        <button className="icon-btn" title="Hide panel (Ctrl+J)" onClick={() => useApp.getState().setLayout({ terminalVisible: false })}>
          <Icon name="chevronDown" size={15} />
        </button>
      </div>
      <div className="terminal-body">
        {sessions.map((s) => (
          <TermView key={s.key} visible={s.key === active} onExit={() => close(s.key)} />
        ))}
      </div>
    </div>
  );
}
