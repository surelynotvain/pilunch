import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { Icon } from "../Icon";
import { useMenu, type MenuItem } from "../ContextMenu";
import { useApp } from "../../store/app";
import { useEditor } from "../../store/editor";
import { useChat } from "../../store/chat";
import { api, errorText } from "../../lib/ipc";
import { dirname, fileBadge } from "../../lib/util";
import type { DirEntryInfo, FsChanged, GitLetter } from "../../lib/types";
import { pickFolder } from "../chat/ChatPanel";

const ROW = 24;

interface Row {
  entry: DirEntryInfo;
  depth: number;
}

type Editing = { mode: "newFile" | "newDir"; parent: string } | { mode: "rename"; path: string };

export function FileBadge({ name }: { name: string }) {
  const [label, color] = fileBadge(name);
  if (!label) return <Icon name="file" size={14} style={{ color: "var(--text-3)", flex: "none" }} />;
  return (
    <span className="file-icon" style={{ color }}>
      {label}
    </span>
  );
}

export function Explorer() {
  const workspace = useApp((s) => s.workspace);
  const git = useApp((s) => s.git);
  const active = useEditor((s) => s.active);
  const [children, setChildren] = useState<Record<string, DirEntryInfo[]>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set(["."]));
  const [selected, setSelected] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [scroll, setScroll] = useState({ top: 0, height: 600 });
  const box = useRef<HTMLDivElement>(null);
  const loadedRef = useRef(children);
  loadedRef.current = children;

  const load = useCallback(async (dir: string) => {
    try {
      const entries = await api.listDir(dir);
      setChildren((c) => ({ ...c, [dir]: entries }));
    } catch {
      setChildren((c) => {
        const { [dir]: _gone, ...rest } = c;
        return rest;
      });
    }
  }, []);

  // Reset when the workspace changes.
  useEffect(() => {
    setChildren({});
    setExpanded(new Set(["."]));
    setSelected(null);
    if (workspace) void load(".");
  }, [workspace, load]);

  // Refresh directories touched by filesystem changes.
  useEffect(() => {
    const onFs = (e: Event) => {
      const ev = (e as CustomEvent<FsChanged>).detail;
      if (!ev.structural && !ev.overflow) return;
      const loaded = Object.keys(loadedRef.current);
      const dirs = ev.overflow ? loaded : [...new Set(ev.paths.map((p) => dirname(p) || "."))].filter((d) => loaded.includes(d));
      for (const d of dirs) void load(d);
    };
    window.addEventListener("pilunch-fs", onFs);
    return () => window.removeEventListener("pilunch-fs", onFs);
  }, [load]);

  // Reveal the active editor file.
  useEffect(() => {
    if (!active) return;
    setSelected(active);
    const parts = active.split("/");
    const dirs: string[] = [];
    for (let i = 1; i < parts.length; i++) dirs.push(parts.slice(0, i).join("/"));
    if (dirs.length === 0) return;
    setExpanded((ex) => {
      const next = new Set(ex);
      for (const d of dirs) {
        if (!next.has(d)) {
          next.add(d);
          if (!loadedRef.current[d]) void load(d);
        }
      }
      return next;
    });
  }, [active, load]);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setScroll((s) => ({ ...s, height: el.clientHeight })));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const rows = useMemo(() => {
    const out: Row[] = [];
    const walk = (dir: string, depth: number) => {
      for (const e of children[dir] ?? []) {
        out.push({ entry: e, depth });
        if (e.isDir && expanded.has(e.path)) walk(e.path, depth + 1);
      }
    };
    walk(".", 0);
    return out;
  }, [children, expanded]);

  // Folders containing git changes get a dot.
  const dirtyDirs = useMemo(() => {
    const s = new Map<string, GitLetter>();
    for (const [p, l] of Object.entries(git?.files ?? {})) {
      let d = dirname(p);
      while (d) {
        if (!s.has(d)) s.set(d, l);
        d = dirname(d);
      }
    }
    return s;
  }, [git]);

  const toggle = (path: string) => {
    setExpanded((ex) => {
      const next = new Set(ex);
      if (next.has(path)) next.delete(path);
      else {
        next.add(path);
        if (!children[path]) void load(path);
      }
      return next;
    });
  };

  const startCreate = (mode: "newFile" | "newDir", base?: string) => {
    let parent = base ?? ".";
    if (!base && selected) {
      const sel = rows.find((r) => r.entry.path === selected)?.entry;
      parent = sel?.isDir ? sel.path : dirname(selected) || ".";
    }
    if (parent !== ".") setExpanded((ex) => new Set(ex).add(parent));
    if (!children[parent]) void load(parent);
    setEditing({ mode, parent });
  };

  const commitEdit = async (value: string) => {
    const ed = editing;
    setEditing(null);
    const name = value.trim();
    if (!ed || !name) return;
    try {
      if (ed.mode === "rename") {
        const to = (dirname(ed.path) ? dirname(ed.path) + "/" : "") + name;
        if (to === ed.path) return;
        const newPath = await api.renamePath(ed.path, to);
        await useEditor.getState().renamed(ed.path, newPath);
        void load(dirname(ed.path) || ".");
      } else {
        const path = ed.parent === "." ? name : `${ed.parent}/${name}`;
        if (ed.mode === "newDir") {
          await api.createDir(path);
        } else {
          const created = await api.createFile(path);
          void useEditor.getState().openFile(created);
        }
        void load(ed.parent);
      }
    } catch (e) {
      useApp.getState().toast(errorText(e), "error");
    }
  };

  const remove = async (entry: DirEntryInfo) => {
    const ok = await ask(`Move "${entry.name}" to the trash?`, { title: "Delete", kind: "warning", okLabel: "Move to Trash" });
    if (!ok) return;
    try {
      await api.deletePath(entry.path, false);
    } catch (e) {
      const perm = await ask(`${errorText(e)}\n\nDelete "${entry.name}" permanently instead?`, { title: "Delete", kind: "warning", okLabel: "Delete Permanently" });
      if (!perm) return;
      try {
        await api.deletePath(entry.path, true);
      } catch (e2) {
        useApp.getState().toast(errorText(e2), "error");
        return;
      }
    }
    useEditor.getState().removed(entry.path);
    void load(dirname(entry.path) || ".");
  };

  const openMenu = (e: React.MouseEvent, entry: DirEntryInfo | null) => {
    e.preventDefault();
    if (entry) setSelected(entry.path);
    const items: MenuItem[] = [];
    const base = entry ? (entry.isDir ? entry.path : dirname(entry.path) || ".") : ".";
    items.push({ label: "New File…", icon: "file", onClick: () => startCreate("newFile", base) });
    items.push({ label: "New Folder…", icon: "folder", onClick: () => startCreate("newDir", base) });
    if (entry) {
      items.push("separator");
      if (!entry.isDir) items.push({ label: "Add to Chat", icon: "at", onClick: () => useChat.getState().attach(entry.path) });
      items.push({ label: "Copy Relative Path", icon: "copy", onClick: () => void navigator.clipboard.writeText(entry.path) });
      items.push({ label: "Rename…", icon: "edit", onClick: () => setEditing({ mode: "rename", path: entry.path }) });
      items.push({ label: "Delete", icon: "trash", danger: true, onClick: () => void remove(entry) });
    }
    useMenu.getState().show(e, items);
  };

  if (!workspace) {
    return (
      <div className="empty-state">
        <div>No folder open</div>
        <button className="btn primary" onClick={() => void pickFolder()}>
          <Icon name="folderOpen" size={15} /> Open Folder
        </button>
      </div>
    );
  }

  // Insert the inline "new item" input row.
  const display: (Row | { input: true; depth: number; mode: "newFile" | "newDir" })[] = [...rows];
  if (editing && editing.mode !== "rename") {
    const parentIdx = editing.parent === "." ? -1 : display.findIndex((r) => "entry" in r && r.entry.path === editing.parent);
    const depth = parentIdx === -1 ? 0 : ((display[parentIdx] as Row).depth ?? 0) + 1;
    display.splice(parentIdx + 1, 0, { input: true, depth, mode: editing.mode });
  }

  const first = Math.max(0, Math.floor(scroll.top / ROW) - 8);
  const last = Math.min(display.length, Math.ceil((scroll.top + scroll.height) / ROW) + 8);

  return (
    <>
      <div className="panel-header">
        <span className="title ellipsis" title={workspace.root}>
          {workspace.name}
        </span>
        <button className="icon-btn" title="New File" onClick={() => startCreate("newFile")}>
          <Icon name="file" size={15} />
        </button>
        <button className="icon-btn" title="New Folder" onClick={() => startCreate("newDir")}>
          <Icon name="folder" size={15} />
        </button>
        <button
          className="icon-btn"
          title="Refresh"
          onClick={() => {
            for (const d of Object.keys(children)) void load(d);
            void useApp.getState().refreshGit();
          }}
        >
          <Icon name="refresh" size={15} />
        </button>
        <button className="icon-btn" title="Collapse All" onClick={() => setExpanded(new Set(["."]))}>
          <Icon name="collapse" size={15} />
        </button>
      </div>
      <div
        className="tree"
        ref={box}
        tabIndex={0}
        onScroll={(e) => setScroll({ top: e.currentTarget.scrollTop, height: e.currentTarget.clientHeight })}
        onContextMenu={(e) => openMenu(e, null)}
        data-testid="explorer"
      >
        <div style={{ height: display.length * ROW + 8 }} />
        {display.slice(first, last).map((row, i) => {
          const top = (first + i) * ROW;
          const pad = 10 + row.depth * 14;
          if ("input" in row) {
            return (
              <div key="__input" className="tree-row" style={{ top, paddingLeft: pad + 14 }}>
                <Icon name={row.mode === "newDir" ? "folder" : "file"} size={14} />
                <EditInput onDone={(v) => void commitEdit(v)} />
              </div>
            );
          }
          const { entry } = row;
          const letter = git?.files[entry.path] ?? (entry.isDir ? dirtyDirs.get(entry.path) : undefined);
          const renaming = editing?.mode === "rename" && editing.path === entry.path;
          return (
            <div
              key={entry.path}
              className={`tree-row${selected === entry.path ? " selected" : ""}`}
              style={{ top, paddingLeft: pad }}
              onClick={() => {
                setSelected(entry.path);
                if (entry.isDir) toggle(entry.path);
                else void useEditor.getState().openFile(entry.path);
              }}
              onContextMenu={(e) => {
                e.stopPropagation();
                openMenu(e, entry);
              }}
              title={entry.path}
              data-path={entry.path}
            >
              <span className="chev">{entry.isDir && <Icon name={expanded.has(entry.path) ? "chevronDown" : "chevronRight"} size={12} />}</span>
              {entry.isDir ? (
                <Icon name={expanded.has(entry.path) ? "folderOpen" : "folder"} size={14} style={{ color: "var(--text-3)", flex: "none" }} />
              ) : (
                <FileBadge name={entry.name} />
              )}
              {renaming ? (
                <EditInput initial={entry.name} onDone={(v) => void commitEdit(v)} />
              ) : (
                <span className={`name${letter ? ` git-${letter}` : ""}`}>{entry.name}</span>
              )}
              {letter && !renaming && (entry.isDir ? <span className={`git git-${letter}`}>•</span> : <span className={`git git-${letter}`}>{letter}</span>)}
            </div>
          );
        })}
      </div>
    </>
  );
}

function EditInput({ initial = "", onDone }: { initial?: string; onDone: (v: string) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    const el = ref.current!;
    el.focus();
    const dot = initial.lastIndexOf(".");
    el.setSelectionRange(0, dot > 0 ? dot : initial.length);
  }, [initial]);
  const finish = (v: string) => {
    if (done.current) return;
    done.current = true;
    onDone(v);
  };
  return (
    <input
      ref={ref}
      defaultValue={initial}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === "Enter") finish(e.currentTarget.value);
        if (e.key === "Escape") finish("");
      }}
      onBlur={(e) => finish(e.currentTarget.value)}
    />
  );
}
