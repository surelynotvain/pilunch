import { create } from "zustand";
import { api, errorText } from "../lib/ipc";
import type { GitStatus, Settings, SettingsView, WorkspaceInfo } from "../lib/types";

export type SidebarView = "explorer" | "search" | "chats";

interface Layout {
  sidebarVisible: boolean;
  sidebarView: SidebarView;
  sidebarWidth: number;
  chatWidth: number;
  terminalVisible: boolean;
  terminalHeight: number;
  /** Chat takes the whole center area (ChatGPT-style) even when files are open. */
  chatFocus: boolean;
}

interface Toast {
  id: number;
  kind: "info" | "error";
  text: string;
}

type Overlay = null | "settings" | "quickOpen" | "commands";

interface AppState {
  settings: SettingsView | null;
  workspace: WorkspaceInfo | null;
  git: GitStatus | null;
  layout: Layout;
  overlay: Overlay;
  toasts: Toast[];
  indexedFiles: number | null;

  init(): Promise<void>;
  updateSettings(patch: Partial<Settings>): Promise<void>;
  setSettingsView(v: SettingsView): void;
  openWorkspace(path: string): Promise<boolean>;
  closeWorkspace(): Promise<void>;
  refreshGit(): Promise<void>;
  setLayout(patch: Partial<Layout>): void;
  showSidebar(view: SidebarView): void;
  setOverlay(o: Overlay): void;
  toast(text: string, kind?: Toast["kind"]): void;
  dismissToast(id: number): void;
}

const LAYOUT_KEY = "pilunch.layout";

function loadLayout(): Layout {
  const def: Layout = {
    sidebarVisible: true,
    sidebarView: "chats",
    sidebarWidth: 272,
    chatWidth: 440,
    terminalVisible: false,
    terminalHeight: 260,
    chatFocus: false,
  };
  try {
    const raw = localStorage.getItem(LAYOUT_KEY);
    return raw ? { ...def, ...JSON.parse(raw) } : def;
  } catch {
    return def;
  }
}

let toastSeq = 0;
let saveLayoutTimer: number | undefined;

export const useApp = create<AppState>((set, get) => ({
  settings: null,
  workspace: null,
  git: null,
  layout: loadLayout(),
  overlay: null,
  toasts: [],
  indexedFiles: null,

  async init() {
    const [settings, workspace] = await Promise.all([api.getSettings(), api.currentWorkspace()]);
    set({ settings, workspace });
    applyTheme(settings.theme);
    if (workspace) void get().refreshGit();
  },

  async updateSettings(patch) {
    try {
      const settings = await api.updateSettings(patch);
      set({ settings });
      if (patch.theme) applyTheme(settings.theme);
    } catch (e) {
      get().toast(errorText(e), "error");
    }
  },

  setSettingsView(v) {
    set({ settings: v });
  },

  async openWorkspace(path) {
    try {
      const workspace = await api.openWorkspace(path);
      set({ workspace, git: null, indexedFiles: null });
      void get().refreshGit();
      // refresh recents
      set({ settings: await api.getSettings() });
      return true;
    } catch (e) {
      get().toast(errorText(e), "error");
      return false;
    }
  },

  async closeWorkspace() {
    await api.closeWorkspace();
    set({ workspace: null, git: null, indexedFiles: null });
  },

  async refreshGit() {
    if (!get().workspace) return;
    try {
      set({ git: await api.gitStatus() });
    } catch {
      set({ git: null });
    }
  },

  setLayout(patch) {
    const layout = { ...get().layout, ...patch };
    set({ layout });
    window.clearTimeout(saveLayoutTimer);
    saveLayoutTimer = window.setTimeout(() => {
      try {
        localStorage.setItem(LAYOUT_KEY, JSON.stringify(layout));
      } catch {
        /* storage unavailable */
      }
    }, 300);
  },

  showSidebar(view) {
    const { layout } = get();
    if (layout.sidebarVisible && layout.sidebarView === view) {
      get().setLayout({ sidebarVisible: false });
    } else {
      get().setLayout({ sidebarVisible: true, sidebarView: view });
    }
  },

  setOverlay(overlay) {
    set({ overlay });
  },

  toast(text, kind = "info") {
    const id = ++toastSeq;
    set({ toasts: [...get().toasts, { id, kind, text }] });
    window.setTimeout(() => get().dismissToast(id), kind === "error" ? 7000 : 3500);
  },

  dismissToast(id) {
    set({ toasts: get().toasts.filter((t) => t.id !== id) });
  },
}));

const media = window.matchMedia?.("(prefers-color-scheme: light)");

export function resolvedTheme(theme: string | undefined): "dark" | "light" {
  if (theme === "light") return "light";
  if (theme === "system") return media?.matches ? "light" : "dark";
  return "dark";
}

export function applyTheme(theme: string) {
  document.documentElement.dataset.theme = resolvedTheme(theme);
  window.dispatchEvent(new CustomEvent("pilunch-theme"));
}

media?.addEventListener?.("change", () => {
  const t = useApp.getState().settings?.theme;
  if (t === "system") applyTheme(t);
});
