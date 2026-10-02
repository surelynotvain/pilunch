import { create } from "zustand";
import { ask } from "@tauri-apps/plugin-dialog";
import type * as Monaco from "monaco-editor";
import { api, errorText } from "../lib/ipc";
import { useApp } from "./app";

type MonacoModule = typeof import("../editor/monaco");
let monacoPromise: Promise<MonacoModule> | null = null;
/** Lazily load Monaco (first file open). */
export function loadMonaco(): Promise<MonacoModule> {
  return (monacoPromise ??= import("../editor/monaco"));
}

export interface Tab {
  path: string;
  name: string;
  kind: "text" | "binary" | "tooLarge";
  dirty: boolean;
  readonly: boolean;
  /** The file changed on disk while it had unsaved edits. */
  externalChange: boolean;
  size: number;
}

interface Entry {
  model: Monaco.editor.ITextModel;
  savedVersion: number;
  viewState: Monaco.editor.ICodeEditorViewState | null;
  sub: Monaco.IDisposable;
}

/** Monaco models live outside React state: one per open text file. */
const entries = new Map<string, Entry>();

export function modelFor(path: string): Entry | undefined {
  return entries.get(path);
}

interface EditorState {
  tabs: Tab[];
  active: string | null;
  cursor: { line: number; col: number; selected: number } | null;
  language: string | null;
  /** Line to reveal once the editor shows `path`. */
  reveal: { path: string; line: number; seq: number } | null;

  openFile(path: string, line?: number): Promise<void>;
  setActive(path: string): void;
  closeTab(path: string, force?: boolean): Promise<void>;
  closeAll(): Promise<void>;
  save(path?: string): Promise<void>;
  saveAll(): Promise<void>;
  reload(path: string): Promise<void>;
  onFsChanged(paths: string[]): Promise<void>;
  renamed(from: string, to: string): Promise<void>;
  removed(path: string): void;
  setCursor(c: EditorState["cursor"], language: string | null): void;
}

let revealSeq = 0;

function baseName(p: string) {
  return p.split("/").pop() ?? p;
}

function setDirty(path: string, dirty: boolean) {
  const st = useEditor.getState();
  const tab = st.tabs.find((t) => t.path === path);
  if (tab && tab.dirty !== dirty) {
    useEditor.setState({ tabs: st.tabs.map((t) => (t.path === path ? { ...t, dirty } : t)) });
  }
}

function patchTab(path: string, patch: Partial<Tab>) {
  useEditor.setState({ tabs: useEditor.getState().tabs.map((t) => (t.path === path ? { ...t, ...patch } : t)) });
}

async function createEntry(path: string, content: string, readonly: boolean): Promise<Entry> {
  const { monaco, languageFor } = await loadMonaco();
  const uri = monaco.Uri.from({ scheme: "file", path: "/" + path });
  monaco.editor.getModel(uri)?.dispose();
  const model = monaco.editor.createModel(content, languageFor(path), uri);
  const entry: Entry = {
    model,
    savedVersion: model.getAlternativeVersionId(),
    viewState: null,
    sub: model.onDidChangeContent(() => {
      // Alternative version ids return to the saved value when edits are undone.
      setDirty(path, !readonly && model.getAlternativeVersionId() !== entry.savedVersion);
    }),
  };
  entries.set(path, entry);
  return entry;
}

function disposeEntry(path: string) {
  const e = entries.get(path);
  if (e) {
    e.sub.dispose();
    e.model.dispose();
    entries.delete(path);
  }
}

/** Replace a model's text as an undoable edit (keeps cursor/undo history). */
function replaceContent(entry: Entry, content: string) {
  const { model } = entry;
  model.pushStackElement();
  model.pushEditOperations([], [{ range: model.getFullModelRange(), text: content }], () => null);
  model.pushStackElement();
  entry.savedVersion = model.getAlternativeVersionId();
}

export const useEditor = create<EditorState>((set, get) => ({
  tabs: [],
  active: null,
  cursor: null,
  language: null,
  reveal: null,

  async openFile(path, line) {
    const existing = get().tabs.find((t) => t.path === path);
    const reveal = line ? { path, line, seq: ++revealSeq } : get().reveal;
    if (existing) {
      set({ active: path, reveal });
      return;
    }
    try {
      const file = await api.readFile(path);
      const kind: Tab["kind"] = file.binary ? "binary" : file.tooLarge ? "tooLarge" : "text";
      if (kind === "text") await createEntry(path, file.content ?? "", file.readonly);
      // Another call may have opened it meanwhile.
      if (get().tabs.some((t) => t.path === path)) {
        set({ active: path, reveal });
        return;
      }
      const tab: Tab = { path, name: baseName(path), kind, dirty: false, readonly: file.readonly, externalChange: false, size: file.size };
      const tabs = [...get().tabs];
      const activeIdx = tabs.findIndex((t) => t.path === get().active);
      tabs.splice(activeIdx >= 0 ? activeIdx + 1 : tabs.length, 0, tab);
      set({ tabs, active: path, reveal });
    } catch (e) {
      useApp.getState().toast(`Can't open ${path}: ${errorText(e)}`, "error");
    }
  },

  setActive(path) {
    set({ active: path });
  },

  async closeTab(path, force = false) {
    const tab = get().tabs.find((t) => t.path === path);
    if (!tab) return;
    if (tab.dirty && !force) {
      const discard = await ask(`${tab.name} has unsaved changes. Close it and discard them?`, {
        title: "Unsaved changes",
        kind: "warning",
        okLabel: "Discard",
        cancelLabel: "Cancel",
      });
      if (!discard) return;
    }
    disposeEntry(path);
    const tabs = get().tabs;
    const idx = tabs.findIndex((t) => t.path === path);
    const next = tabs.filter((t) => t.path !== path);
    let active = get().active;
    if (active === path) active = next[Math.min(idx, next.length - 1)]?.path ?? null;
    set({ tabs: next, active, ...(next.length === 0 ? { cursor: null, language: null } : {}) });
  },

  async closeAll() {
    for (const t of [...get().tabs]) await get().closeTab(t.path);
  },

  async save(path) {
    const p = path ?? get().active;
    if (!p) return;
    const entry = entries.get(p);
    const tab = get().tabs.find((t) => t.path === p);
    if (!entry || !tab || tab.readonly) return;
    try {
      const version = entry.model.getAlternativeVersionId();
      await api.writeFile(p, entry.model.getValue());
      entry.savedVersion = version;
      patchTab(p, { dirty: entry.model.getAlternativeVersionId() !== version, externalChange: false });
    } catch (e) {
      useApp.getState().toast(`Save failed: ${errorText(e)}`, "error");
    }
  },

  async saveAll() {
    for (const t of get().tabs) if (t.dirty) await get().save(t.path);
  },

  async reload(path) {
    const entry = entries.get(path);
    if (!entry) return;
    try {
      const file = await api.readFile(path);
      if (file.content != null && file.content !== entry.model.getValue()) replaceContent(entry, file.content);
      else entry.savedVersion = entry.model.getAlternativeVersionId();
      patchTab(path, { dirty: false, externalChange: false });
    } catch (e) {
      useApp.getState().toast(`Reload failed: ${errorText(e)}`, "error");
    }
  },

  async onFsChanged(paths) {
    const open = new Set(get().tabs.filter((t) => t.kind === "text").map((t) => t.path));
    for (const p of paths) {
      if (!open.has(p)) continue;
      const entry = entries.get(p);
      if (!entry) continue;
      let content: string | null;
      try {
        content = (await api.readFile(p)).content;
      } catch {
        continue; // deleted: keep the buffer so nothing is lost
      }
      if (content == null || content === entry.model.getValue()) {
        // Our own save, or no real change.
        if (content != null) {
          entry.savedVersion = entry.model.getAlternativeVersionId();
          setDirty(p, false);
        }
        continue;
      }
      const tab = get().tabs.find((t) => t.path === p);
      if (tab?.dirty) patchTab(p, { externalChange: true });
      else {
        replaceContent(entry, content);
        setDirty(p, false);
      }
    }
  },

  async renamed(from, to) {
    const affected = get().tabs.filter((t) => t.path === from || t.path.startsWith(from + "/"));
    for (const t of affected) {
      const newPath = to + t.path.slice(from.length);
      const old = entries.get(t.path);
      if (old) {
        const content = old.model.getValue();
        const wasDirty = t.dirty;
        disposeEntry(t.path);
        const e = await createEntry(newPath, content, t.readonly);
        if (wasDirty) e.savedVersion = -1;
      }
      set({
        tabs: get().tabs.map((x) => (x.path === t.path ? { ...x, path: newPath, name: baseName(newPath) } : x)),
        active: get().active === t.path ? newPath : get().active,
      });
    }
  },

  removed(path) {
    for (const t of get().tabs) {
      if (t.path === path || t.path.startsWith(path + "/")) void get().closeTab(t.path, true);
    }
  },

  setCursor(cursor, language) {
    set({ cursor, language });
  },
}));
