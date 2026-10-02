import { useEffect, useMemo, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { Icon } from "../Icon";
import { useChat } from "../../store/chat";
import { useApp } from "../../store/app";
import { dayGroup } from "../../lib/util";
import type { ConversationMeta } from "../../lib/types";

export function ChatList() {
  const list = useChat((s) => s.list);
  const activeId = useChat((s) => s.activeId);
  // A primitive: subscribing to `runs` itself would re-render on every streamed token.
  const runningIds = useChat((s) =>
    Object.keys(s.runs)
      .filter((id) => s.runs[id]!.running)
      .join(","),
  );
  const running = useMemo(() => new Set(runningIds.split(",").filter(Boolean)), [runningIds]);
  const workspace = useApp((s) => s.workspace);
  const [all, setAll] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);

  useEffect(() => {
    void useChat.getState().loadList();
  }, []);

  const groups = useMemo(() => {
    const filtered = all || !workspace ? list : list.filter((c) => c.workspace === workspace.root);
    const out: [string, ConversationMeta[]][] = [];
    for (const c of filtered) {
      if (c.messageCount === 0 && !running.has(c.id)) continue;
      const g = dayGroup(c.updatedAt);
      const last = out[out.length - 1];
      if (last && last[0] === g) last[1].push(c);
      else out.push([g, [c]]);
    }
    return out;
  }, [list, all, workspace, running]);

  return (
    <>
      <div className="panel-header">
        <span className="title">Chats</span>
        {workspace && (
          <button className={`icon-btn${all ? " active" : ""}`} title={all ? "Showing all folders" : "Showing this folder only"} onClick={() => setAll(!all)}>
            <Icon name="globe" size={15} />
          </button>
        )}
        <button className="icon-btn" title="New chat" onClick={() => useChat.getState().newChat()}>
          <Icon name="plus" size={16} />
        </button>
      </div>
      <div className="chat-list" data-testid="chat-list">
        {groups.length === 0 && <div className="empty-state">No chats yet</div>}
        {groups.map(([g, items]) => (
          <div key={g}>
            <div className="chat-group">{g}</div>
            {items.map((c) => (
              <div
                key={c.id}
                className={`chat-item${c.id === activeId ? " active" : ""}`}
                onClick={() => void useChat.getState().open(c.id)}
                onDoubleClick={() => setRenaming(c.id)}
                title={c.workspace ?? "No folder"}
              >
                {running.has(c.id) ? <span className="live-dot" /> : <Icon name="chat" size={14} />}
                {renaming === c.id ? (
                  <input
                    autoFocus
                    defaultValue={c.title}
                    onClick={(e) => e.stopPropagation()}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        void useChat.getState().rename(c.id, e.currentTarget.value);
                        setRenaming(null);
                      }
                      if (e.key === "Escape") setRenaming(null);
                    }}
                    onBlur={(e) => {
                      void useChat.getState().rename(c.id, e.currentTarget.value);
                      setRenaming(null);
                    }}
                  />
                ) : (
                  <span className="title ellipsis">{c.title}</span>
                )}
                <span className="actions">
                  <button
                    className="icon-btn"
                    title="Rename"
                    onClick={(e) => {
                      e.stopPropagation();
                      setRenaming(c.id);
                    }}
                  >
                    <Icon name="edit" size={13} />
                  </button>
                  <button
                    className="icon-btn"
                    title="Delete"
                    onClick={async (e) => {
                      e.stopPropagation();
                      if (await ask(`Delete "${c.title}"?`, { title: "Delete chat", kind: "warning", okLabel: "Delete" })) {
                        void useChat.getState().remove(c.id);
                      }
                    }}
                  >
                    <Icon name="trash" size={13} />
                  </button>
                </span>
              </div>
            ))}
          </div>
        ))}
      </div>
    </>
  );
}
