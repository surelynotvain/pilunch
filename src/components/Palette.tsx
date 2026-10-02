import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "./Icon";
import { FileBadge } from "./sidebar/Explorer";
import { COMMANDS } from "../commands";
import { useApp } from "../store/app";
import { useEditor } from "../store/editor";
import { api } from "../lib/ipc";
import { basename, dirname } from "../lib/util";
import type { FileMatch } from "../lib/types";

function Highlight({ text, indices, offset }: { text: string; indices: number[]; offset: number }) {
  if (!indices.length) return <>{text}</>;
  const set = new Set(indices);
  const out: React.ReactNode[] = [];
  for (let i = 0; i < text.length; i++) {
    out.push(set.has(i + offset) ? <span key={i} className="hl">{text[i]}</span> : text[i]);
  }
  return <>{out}</>;
}

/** Ctrl+P (files) and Ctrl+Shift+P (commands; also via a leading ">"). */
export function Palette({ mode }: { mode: "quickOpen" | "commands" }) {
  const workspace = useApp((s) => s.workspace);
  const close = () => useApp.getState().setOverlay(null);
  const [query, setQuery] = useState(mode === "commands" ? ">" : "");
  const [files, setFiles] = useState<FileMatch[]>([]);
  const [sel, setSel] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const isCommands = query.startsWith(">");

  useEffect(() => {
    if (isCommands || !workspace) return;
    let alive = true;
    void api.quickOpen(query, 60).then((r) => {
      if (alive) {
        setFiles(r);
        setSel(0);
      }
    });
    return () => {
      alive = false;
    };
  }, [query, isCommands, workspace]);

  const commands = useMemo(() => {
    const q = query.slice(1).trim().toLowerCase();
    return COMMANDS.filter((c) => (!c.needsWorkspace || workspace) && (!q || q.split(/\s+/).every((w) => c.title.toLowerCase().includes(w))));
  }, [query, workspace]);

  useEffect(() => setSel(0), [isCommands]);
  useEffect(() => {
    list.current?.querySelector(".palette-item.active")?.scrollIntoView({ block: "nearest" });
  }, [sel]);

  const count = isCommands ? commands.length : files.length;
  const choose = (i: number) => {
    close();
    if (isCommands) commands[i]?.run();
    else if (files[i]) void useEditor.getState().openFile(files[i]!.path);
  };

  return (
    <div className="overlay" onMouseDown={close}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()} data-testid="palette">
        <input
          autoFocus
          value={query}
          placeholder={isCommands ? "Type a command" : workspace ? "Search files by name (type > for commands)" : "Open a folder to search files (type > for commands)"}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") close();
            else if (e.key === "ArrowDown") {
              e.preventDefault();
              setSel((s) => Math.min(s + 1, count - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setSel((s) => Math.max(s - 1, 0));
            } else if (e.key === "Enter") {
              e.preventDefault();
              choose(sel);
            }
          }}
        />
        <div className="palette-list" ref={list}>
          {count === 0 && <div className="palette-empty">{isCommands ? "No matching commands" : workspace ? "No matching files" : "No folder open"}</div>}
          {isCommands
            ? commands.map((c, i) => (
                <div key={c.id} className={`palette-item${i === sel ? " active" : ""}`} onMouseEnter={() => setSel(i)} onClick={() => choose(i)}>
                  <Icon name="chevronRight" size={13} />
                  <span>{c.title}</span>
                  {c.shortcut && <span className="hint kbd">{c.shortcut}</span>}
                </div>
              ))
            : files.map((f, i) => {
                const name = basename(f.path);
                return (
                  <div key={f.path} className={`palette-item${i === sel ? " active" : ""}`} onMouseEnter={() => setSel(i)} onClick={() => choose(i)}>
                    <FileBadge name={name} />
                    <span>
                      <Highlight text={name} indices={f.indices} offset={f.path.length - name.length} />
                    </span>
                    <span className="dir">{dirname(f.path)}</span>
                  </div>
                );
              })}
        </div>
      </div>
    </div>
  );
}
