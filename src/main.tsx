import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

// Keep the native webview menu (Reload, Inspect…) out of the way, but allow it where
// people expect to copy/paste text. Monaco and xterm handle their own menus.
document.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest("input, textarea, .selectable, .md, .bubble, pre, .monaco-editor, .xterm")) e.preventDefault();
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
