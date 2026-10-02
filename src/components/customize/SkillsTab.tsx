import { useEffect, useState } from "react";
import { Icon } from "../Icon";
import { useApp } from "../../store/app";
import { api, errorText } from "../../lib/ipc";
import type { Skill } from "../../lib/types";

interface Draft {
  path: string | null;
  scope: string;
  name: string;
  description: string;
  body: string;
}

const NEW: Draft = { path: null, scope: "user", name: "", description: "", body: "" };

export function scopeLabel(scope: string) {
  if (scope === "user") return "You";
  if (scope === "project") return "Project";
  return scope.replace(/^plugin:/, "Plugin: ");
}

export function SkillsTab() {
  const workspace = useApp((s) => s.workspace);
  const [skills, setSkills] = useState<Skill[] | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [busy, setBusy] = useState(false);
  const toast = useApp.getState().toast;

  const load = () => api.listSkills().then(setSkills, (e) => toast(errorText(e), "error"));
  useEffect(() => void load(), []);

  const open = async (s: Skill) => {
    try {
      const t = await api.readSkill(s.path);
      setDraft({ path: s.path, scope: s.scope, name: t.name, description: t.description, body: t.body });
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const save = async () => {
    if (!draft) return;
    setBusy(true);
    try {
      const path = await api.saveSkill(draft.scope, draft.name.trim(), draft.description, draft.body);
      setDraft({ ...draft, path });
      toast(`Saved skill “${draft.name}”`);
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!draft?.path || !confirm(`Delete the skill “${draft.name}”?`)) return;
    try {
      await api.deleteSkill(draft.path);
      setDraft(null);
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const readOnly = !!draft?.scope.startsWith("plugin:");

  return (
    <>
      <h3>Skills</h3>
      <p className="lead">
        Reusable instructions in Markdown (SKILL.md). The agent sees each skill's name and description, loads the full text when a task matches, and saves new
        skills itself after it researches something or works out a procedure. Yours live in ~/.config/pilunch/skills; project skills in .pilunch/skills.
      </p>
      <div className="hub-actions">
        <button className="btn primary sm" onClick={() => setDraft({ ...NEW })} data-testid="skill-new">
          <Icon name="plus" size={14} /> New skill
        </button>
      </div>
      <div className="hub-split">
        <div className="hub-list">
          {skills === null && <div className="spinner" />}
          {skills?.length === 0 && <div className="hub-empty">No skills yet. Create one, or ask the agent to “save what you learned as a skill”.</div>}
          {skills?.map((s) => (
            <button key={s.path} className={`hub-item${draft?.path === s.path ? " on" : ""}`} onClick={() => void open(s)}>
              <span className="title">
                <Icon name="book" size={13} /> {s.name} <span className="tag">{scopeLabel(s.scope)}</span>
              </span>
              <span className="sub">{s.description || "No description"}</span>
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
                disabled={!!draft.path}
                placeholder="release-checklist"
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                data-testid="skill-name"
              />
            </label>
            <label>
              Description — when should the agent use it?
              <input
                className="input"
                value={draft.description}
                disabled={readOnly}
                placeholder="How to cut a release of this project"
                onChange={(e) => setDraft({ ...draft, description: e.target.value })}
              />
            </label>
            {!draft.path && (
              <label>
                Save for
                <select className="select" value={draft.scope} onChange={(e) => setDraft({ ...draft, scope: e.target.value })}>
                  <option value="user">All my projects</option>
                  <option value="project" disabled={!workspace}>
                    This project (.pilunch/skills)
                  </option>
                </select>
              </label>
            )}
            <label>
              Instructions (Markdown)
              <textarea
                className="textarea code"
                rows={16}
                value={draft.body}
                disabled={readOnly}
                onChange={(e) => setDraft({ ...draft, body: e.target.value })}
                data-testid="skill-body"
              />
            </label>
            {readOnly ? (
              <span className="muted">This skill comes from a plugin and is read-only.</span>
            ) : (
              <div className="hub-actions">
                <button
                  className="btn primary sm"
                  disabled={busy || !draft.name.trim() || !draft.body.trim()}
                  onClick={() => void save()}
                  data-testid="skill-save"
                >
                  Save
                </button>
                {draft.path && (
                  <button className="btn sm danger" onClick={() => void remove()}>
                    <Icon name="trash" size={13} /> Delete
                  </button>
                )}
              </div>
            )}
          </div>
        ) : (
          <div className="hub-empty">Select a skill to view or edit it.</div>
        )}
      </div>
    </>
  );
}
