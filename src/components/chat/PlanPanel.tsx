import { useMemo, useState } from "react";
import { Icon } from "../Icon";
import { useChat } from "../../store/chat";

interface Todo {
  content: string;
  status: "pending" | "in_progress" | "completed";
}

/** The agent's live checklist: the latest todo_write call in this chat. */
export function PlanPanel({ convId }: { convId: string }) {
  const messages = useChat((s) => s.convs[convId]?.messages);
  const draft = useChat((s) => s.runs[convId]?.draft);
  const [open, setOpen] = useState(true);
  const todos = useMemo(() => {
    let latest: Todo[] | null = null;
    for (const m of messages ?? []) {
      for (const b of m.content) {
        if (b.type === "tool_use" && b.name === "todo_write" && Array.isArray((b.input as { todos?: unknown })?.todos)) {
          latest = (b.input as { todos: Todo[] }).todos;
        }
      }
    }
    for (const b of draft ?? []) {
      if (b.toolName === "todo_write" && Array.isArray((b.input as { todos?: unknown } | undefined)?.todos)) latest = (b.input as { todos: Todo[] }).todos;
    }
    return latest;
  }, [messages, draft]);
  if (!todos?.length) return null;
  const done = todos.filter((t) => t.status === "completed").length;
  return (
    <div className="plan-panel" data-testid="plan-panel">
      <button className="plan-head" onClick={() => setOpen(!open)}>
        <Icon name="list" size={14} />
        <span>Plan</span>
        <span className="plan-progress">
          <span style={{ width: `${(done / todos.length) * 100}%` }} />
        </span>
        <span className="muted">
          {done}/{todos.length}
        </span>
        <Icon name={open ? "chevronDown" : "chevronRight"} size={13} />
      </button>
      {open && (
        <ul>
          {todos.map((t, i) => (
            <li key={i} className={t.status}>
              <span className="box">{t.status === "completed" ? <Icon name="check" size={11} /> : t.status === "in_progress" ? <span className="dot" /> : null}</span>
              {t.content}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
