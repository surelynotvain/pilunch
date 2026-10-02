// App commands: shared by keyboard shortcuts and the command palette.
import { useApp } from "./store/app";
import { useChat } from "./store/chat";
import { useEditor } from "./store/editor";
import { pickFolder } from "./components/chat/ChatPanel";
import { addSelectionToChat, getActiveEditor } from "./components/editor/EditorArea";

export interface Command {
  id: string;
  title: string;
  /** Display form, e.g. "Ctrl+Shift+P". Matching is done by `keys`. */
  shortcut?: string;
  /** Normalized key combos, e.g. "ctrl+shift+p". */
  keys?: string[];
  /** Only allowed while a folder is open. */
  needsWorkspace?: boolean;
  run: () => void;
}

const app = () => useApp.getState();

export const COMMANDS: Command[] = [
  { id: "quickOpen", title: "Go to File…", shortcut: "Ctrl+P", keys: ["ctrl+p"], needsWorkspace: true, run: () => app().setOverlay("quickOpen") },
  { id: "palette", title: "Show All Commands", shortcut: "Ctrl+Shift+P", keys: ["ctrl+shift+p", "f1"], run: () => app().setOverlay("commands") },
  { id: "openFolder", title: "Open Folder…", shortcut: "Ctrl+O", keys: ["ctrl+o"], run: () => void pickFolder() },
  { id: "closeFolder", title: "Close Folder", needsWorkspace: true, run: () => void useEditor.getState().closeAll().then(() => app().closeWorkspace()) },
  { id: "newChat", title: "New Chat", shortcut: "Ctrl+N", keys: ["ctrl+n"], run: () => useChat.getState().newChat() },
  { id: "focusChat", title: "Focus Chat Input", shortcut: "Ctrl+K", keys: ["ctrl+k"], run: () => useChat.getState().focusComposer() },
  { id: "addToChat", title: "Add Selection/File to Chat", shortcut: "Ctrl+L", keys: ["ctrl+l"], needsWorkspace: true, run: () => addSelectionToChat(getActiveEditor()) },
  { id: "stop", title: "Stop Claude", run: () => useChat.getState().cancel() },
  { id: "toggleChatFocus", title: "Toggle Chat Focus Mode", shortcut: "Ctrl+Shift+L", keys: ["ctrl+shift+l"], run: () => app().setLayout({ chatFocus: !app().layout.chatFocus }) },
  { id: "save", title: "Save File", shortcut: "Ctrl+S", keys: ["ctrl+s"], run: () => void useEditor.getState().save() },
  { id: "saveAll", title: "Save All Files", shortcut: "Ctrl+Alt+S", keys: ["ctrl+alt+s"], run: () => void useEditor.getState().saveAll() },
  {
    id: "closeTab",
    title: "Close Editor Tab",
    shortcut: "Ctrl+W",
    keys: ["ctrl+w"],
    run: () => {
      const a = useEditor.getState().active;
      if (a) void useEditor.getState().closeTab(a);
    },
  },
  { id: "closeAll", title: "Close All Editor Tabs", run: () => void useEditor.getState().closeAll() },
  { id: "toggleSidebar", title: "Toggle Sidebar", shortcut: "Ctrl+B", keys: ["ctrl+b"], run: () => app().setLayout({ sidebarVisible: !app().layout.sidebarVisible }) },
  { id: "explorer", title: "Show Explorer", shortcut: "Ctrl+Shift+E", keys: ["ctrl+shift+e"], run: () => app().setLayout({ sidebarVisible: true, sidebarView: "explorer" }) },
  {
    id: "search",
    title: "Search in Files",
    shortcut: "Ctrl+Shift+F",
    keys: ["ctrl+shift+f"],
    needsWorkspace: true,
    run: () => {
      app().setLayout({ sidebarVisible: true, sidebarView: "search" });
      requestAnimationFrame(() => window.dispatchEvent(new Event("pilunch-focus-search")));
    },
  },
  { id: "chats", title: "Show Chat History", shortcut: "Ctrl+Shift+H", keys: ["ctrl+shift+h"], run: () => app().setLayout({ sidebarVisible: true, sidebarView: "chats" }) },
  { id: "terminal", title: "Toggle Terminal", shortcut: "Ctrl+J", keys: ["ctrl+j", "ctrl+`"], run: () => app().setLayout({ terminalVisible: !app().layout.terminalVisible }) },
  { id: "settings", title: "Open Settings", shortcut: "Ctrl+,", keys: ["ctrl+,"], run: () => app().setOverlay("settings") },
  { id: "browser", title: "Toggle Browser", shortcut: "Ctrl+Shift+B", keys: ["ctrl+shift+b"], run: () => app().setLayout({ browserVisible: !app().layout.browserVisible }) },
  { id: "customize", title: "Customize: Skills, Tools, MCP, Plugins", run: () => app().openHub("skills") },
  { id: "skills", title: "Manage Skills", run: () => app().openHub("skills") },
  { id: "customTools", title: "Manage Custom Tools", run: () => app().openHub("tools") },
  { id: "mcp", title: "Manage MCP Servers", run: () => app().openHub("mcp") },
  { id: "plugins", title: "Manage Plugins & Rust Extensions", run: () => app().openHub("plugins") },
  { id: "usage", title: "Show Usage", run: () => app().openHub("usage") },
  { id: "traces", title: "Training Traces (Export Dataset)", run: () => app().openHub("traces") },
  {
    id: "theme",
    title: "Toggle Light/Dark Theme",
    run: () => void app().updateSettings({ theme: document.documentElement.dataset.theme === "light" ? "dark" : "light" }),
  },
  { id: "modeAsk", title: "Permission Mode: Ask Before Edits", run: () => void app().updateSettings({ permissionMode: "ask" }) },
  { id: "modeAccept", title: "Permission Mode: Auto-accept Edits", run: () => void app().updateSettings({ permissionMode: "acceptEdits" }) },
  { id: "modePlan", title: "Permission Mode: Plan (read-only)", run: () => void app().updateSettings({ permissionMode: "plan" }) },
  { id: "refreshGit", title: "Refresh Git Status", needsWorkspace: true, run: () => void app().refreshGit() },
];

export function comboFromEvent(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("ctrl");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  let k = e.key.toLowerCase();
  if (e.code === "Backquote") k = "`";
  if (e.code === "Comma") k = ",";
  if (e.code.startsWith("Key")) k = e.code.slice(3).toLowerCase();
  parts.push(k);
  return parts.join("+");
}

export function commandForEvent(e: KeyboardEvent): Command | undefined {
  const combo = comboFromEvent(e);
  return COMMANDS.find((c) => c.keys?.includes(combo));
}
