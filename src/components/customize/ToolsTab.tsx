import { useEffect, useState } from "react";
import { Icon } from "../Icon";
import { useApp } from "../../store/app";
import { api, errorText } from "../../lib/ipc";
import type { ToolList } from "../../lib/types";
import { scopeLabel } from "./SkillsTab";

interface Draft {
  existing: boolean;
  scope: string;
  name: string;
  description: string;
  parameters: string;
  command: string;
  timeout: string;
}

const EXAMPLE: Draft = {
  existing: false,
  scope: "user",
  name: "count_todos",
  description: "Count TODO comments in the project, optionally only in one folder.",
  parameters: JSON.stringify({ type: "object", properties: { dir: { type: "string", description: "Folder to search (default: whole project)" } } }, null, 2),
  command: 'grep -rn "TODO" {{dir}} --include="*.*" | wc -l',
  timeout: "",
};

export function ToolsTab() {
  const workspace = useApp((s) => s.workspace);
  const [list, setList] = useState<ToolList | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const toast = useApp.getState().toast;
  const load = () => api.listCustomTools().then(setList, (e) => toast(errorText(e), "error"));
  useEffect(() => void load(), []);

  const save = async () => {
    if (!draft) return;
    let parameters: Record<string, unknown>;
    try {
      parameters = draft.parameters.trim() ? JSON.parse(draft.parameters) : { type: "object", properties: {} };
    } catch (e) {
      toast(`Parameters must be JSON Schema: ${errorText(e)}`, "error");
      return;
    }
    try {
      await api.saveCustomTool(draft.scope, {
        name: draft.name.trim(),
        description: draft.description.trim(),
        parameters,
        command: draft.command,
        timeout: draft.timeout.trim() ? Number(draft.timeout) : null,
      });
      toast(`Saved tool “${draft.name}”`);
      setDraft({ ...draft, existing: true });
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const remove = async () => {
    if (!draft || !confirm(`Delete the tool “${draft.name}”?`)) return;
    try {
      await api.deleteCustomTool(draft.name);
      setDraft(null);
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const readOnly = !!draft?.scope.startsWith("plugin:");

  return (
    <>
      <h3>Custom tools</h3>
      <p className="lead">
        Give the agent your own tools: a shell command plus a JSON Schema for its inputs. <code>{"{{param}}"}</code> is replaced with the shell-quoted value,
        and every input is also in <code>$PILUNCH_ARG_NAME</code> and <code>$PILUNCH_INPUT</code> (JSON). Tools run in the project folder and ask before
        running, like commands.
      </p>
      <div className="hub-actions">
        <button className="btn primary sm" onClick={() => setDraft({ ...EXAMPLE })} data-testid="tool-new">
          <Icon name="plus" size={14} /> New tool
        </button>
        {list && <span className="muted">Saved as JSON in {list.userDir} or .pilunch/tools</span>}
      </div>
      {list?.errors.map((e) => (
        <div key={e} className="onb-err" style={{ marginBottom: 8 }}>
          {e}
        </div>
      ))}
      <div className="hub-split">
        <div className="hub-list">
          {list?.tools.length === 0 && <div className="hub-empty">No custom tools yet.</div>}
          {list?.tools.map((t) => (
            <button
              key={t.path}
              className={`hub-item${draft?.existing && draft.name === t.name ? " on" : ""}`}
              onClick={() =>
                setDraft({
                  existing: true,
                  scope: t.scope,
                  name: t.name,
                  description: t.description,
                  parameters: JSON.stringify(t.parameters, null, 2),
                  command: t.command,
                  timeout: t.timeout ? String(t.timeout) : "",
                })
              }
            >
              <span className="title">
                <Icon name="wrench" size={13} /> {t.name} <span className="tag">{scopeLabel(t.scope)}</span>
              </span>
              <span className="sub">{t.description}</span>
            </button>
          ))}
        </div>
        {draft ? (
          <div className="hub-form">
            <label>
              Name
              <input
                className="input"
                value={draft.name}
                disabled={draft.existing}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                data-testid="tool-name"
              />
            </label>
            <label>
              Description — what it does and when to use it
              <input className="input" value={draft.description} disabled={readOnly} onChange={(e) => setDraft({ ...draft, description: e.target.value })} />
            </label>
            <label>
              Parameters (JSON Schema)
              <textarea
                className="textarea code"
                rows={7}
                value={draft.parameters}
                disabled={readOnly}
                onChange={(e) => setDraft({ ...draft, parameters: e.target.value })}
              />
            </label>
            <label>
              Command
              <textarea
                className="textarea code"
                rows={3}
                value={draft.command}
                disabled={readOnly}
                onChange={(e) => setDraft({ ...draft, command: e.target.value })}
                data-testid="tool-command"
              />
            </label>
            <div className="onb-row">
              <label style={{ flex: 1 }}>
                Timeout (seconds, default 120)
                <input
                  className="input"
                  inputMode="numeric"
                  value={draft.timeout}
                  disabled={readOnly}
                  onChange={(e) => setDraft({ ...draft, timeout: e.target.value })}
                />
              </label>
              {!draft.existing && (
                <label style={{ flex: 1 }}>
                  Save for
                  <select className="select" value={draft.scope} onChange={(e) => setDraft({ ...draft, scope: e.target.value })}>
                    <option value="user">All my projects</option>
                    <option value="project" disabled={!workspace}>
                      This project
                    </option>
                  </select>
                </label>
              )}
            </div>
            {readOnly ? (
              <span className="muted">This tool comes from a plugin and is read-only.</span>
            ) : (
              <div className="hub-actions">
                <button className="btn primary sm" disabled={!draft.name.trim() || !draft.command.trim()} onClick={() => void save()} data-testid="tool-save">
                  Save
                </button>
                {draft.existing && (
                  <button className="btn sm danger" onClick={() => void remove()}>
                    <Icon name="trash" size={13} /> Delete
                  </button>
                )}
              </div>
            )}
          </div>
        ) : (
          <div className="hub-empty">Select a tool, or create one from the example.</div>
        )}
      </div>
    </>
  );
}
