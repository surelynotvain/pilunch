import { useEffect, useState } from "react";
import { Icon } from "./Icon";
import { MODELS } from "../lib/models";
import { useApp } from "../store/app";
import { ProviderPicker } from "./ProviderPicker";
import type { Effort, PermissionMode, Settings, Theme } from "../lib/types";

function Switch({ on, onChange, testId }: { on: boolean; onChange: (v: boolean) => void; testId?: string }) {
  return <button className={`switch${on ? " on" : ""}`} onClick={() => onChange(!on)} role="switch" aria-checked={on} data-testid={testId} />;
}

/** Text input that saves on blur / Enter. */
function Lazy({ value, onSave, ...rest }: { value: string; onSave: (v: string) => void } & Omit<React.InputHTMLAttributes<HTMLInputElement>, "value" | "onChange">) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      {...rest}
      className="input"
      value={v}
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v !== value && onSave(v)}
      onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
    />
  );
}

export function SettingsModal() {
  const s = useApp((st) => st.settings);
  const close = () => useApp.getState().setOverlay(null);
  const update = (patch: Partial<Settings>) => void useApp.getState().updateSettings(patch);
  const [instructions, setInstructions] = useState(s?.customInstructions ?? "");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!s) return null;

  const modelOptions = [...MODELS];
  if (!modelOptions.some((m) => m.id === s.model)) modelOptions.push({ id: s.model, label: s.model });

  return (
    <div className="overlay" onMouseDown={close}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()} data-testid="settings">
        <div className="modal-head">
          <Icon name="settings" size={18} />
          <h2>Settings</h2>
          <button className="icon-btn" onClick={close} title="Close (Esc)">
            <Icon name="x" size={16} />
          </button>
        </div>
        <div className="modal-body">
          <div className="settings-section">
            <h3>Model provider</h3>
            <ProviderPicker />
            <span className="help">Keys are stored in {s.configDir}/secrets.json (owner-only permissions) and only sent to their provider.</span>
          </div>

          <div className="settings-section">
            <h3>Model</h3>
            {s.provider === "anthropic" && (
              <div className="field">
                <label>
                  Model
                  <span className="help">Opus for the hardest work, Sonnet for speed, Haiku for quick answers.</span>
                </label>
                <div className="row">
                  <select className="select" value={s.model} onChange={(e) => update({ model: e.target.value })}>
                    {modelOptions.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.label}
                      </option>
                    ))}
                  </select>
                  <Lazy value={s.model} onSave={(v) => v.trim() && update({ model: v.trim() })} placeholder="custom model id" style={{ maxWidth: 200 }} />
                </div>
              </div>
            )}
            <div className="field">
              <label>
                Effort
                <span className="help">How hard the model thinks. Higher is smarter but slower and costs more.</span>
              </label>
              <select className="select" value={s.effort} onChange={(e) => update({ effort: e.target.value as Effort })}>
                <option value="low">Low — fastest</option>
                <option value="medium">Medium</option>
                <option value="high">High (recommended)</option>
                <option value="xhigh">Extra high</option>
                <option value="max">Max</option>
              </select>
            </div>
            {s.provider === "anthropic" && (
              <div className="field">
                <label>
                  Web search
                  <span className="help">Let Claude search the web for docs and answers (Anthropic bills per search).</span>
                </label>
                <Switch on={s.webSearch} onChange={(v) => update({ webSearch: v })} />
              </div>
            )}
            <div className="field">
              <label>Show thinking</label>
              <Switch on={s.showThinking} onChange={(v) => update({ showThinking: v })} />
            </div>
            <div className="field">
              <label>
                Max output tokens
                <span className="help">Per response. Raise it if long files get cut off.</span>
              </label>
              <Lazy value={String(s.maxTokens)} onSave={(v) => Number(v) > 0 && update({ maxTokens: Number(v) })} inputMode="numeric" />
            </div>
            {s.provider === "anthropic" && (
              <div className="field">
                <label>
                  API endpoint
                  <span className="help">Only change this for a proxy or gateway.</span>
                </label>
                <Lazy value={s.baseUrl} onSave={(v) => update({ baseUrl: v.trim() })} placeholder="https://api.anthropic.com" />
              </div>
            )}
          </div>

          <div className="settings-section">
            <h3>Agent</h3>
            <div className="field">
              <label>
                Permissions
                <span className="help">What Claude may do without asking.</span>
              </label>
              <select className="select" value={s.permissionMode} onChange={(e) => update({ permissionMode: e.target.value as PermissionMode })}>
                <option value="ask">Ask before edits and commands</option>
                <option value="acceptEdits">Auto-accept edits, ask before commands</option>
                <option value="plan">Plan mode (read-only)</option>
                <option value="bypass">Bypass approvals (not recommended)</option>
              </select>
            </div>
            <div className="field">
              <label>
                Custom instructions
                <span className="help">Added to every conversation. Project-specific rules can also go in AGENTS.md or PILUNCH.md at the project root.</span>
              </label>
              <textarea
                className="textarea"
                rows={4}
                value={instructions}
                placeholder="e.g. Prefer small functions. Use pnpm, not npm."
                onChange={(e) => setInstructions(e.target.value)}
                onBlur={() => instructions !== s.customInstructions && update({ customInstructions: instructions })}
              />
            </div>
          </div>

          <div className="settings-section">
            <h3>Appearance & editor</h3>
            <div className="field">
              <label>Theme</label>
              <select className="select" value={s.theme} onChange={(e) => update({ theme: e.target.value as Theme })}>
                <option value="dark">Dark</option>
                <option value="light">Light</option>
                <option value="system">Follow system</option>
              </select>
            </div>
            <div className="field">
              <label>Editor font size</label>
              <Lazy value={String(s.editorFontSize)} onSave={(v) => Number(v) > 0 && update({ editorFontSize: Number(v) })} inputMode="numeric" />
            </div>
            <div className="field">
              <label>Word wrap</label>
              <Switch on={s.editorWordWrap} onChange={(v) => update({ editorWordWrap: v })} />
            </div>
            <div className="field">
              <label>Minimap</label>
              <Switch on={s.editorMinimap} onChange={(v) => update({ editorMinimap: v })} />
            </div>
            <div className="field">
              <label>
                Terminal shell
                <span className="help">Empty uses $SHELL.</span>
              </label>
              <Lazy value={s.terminalShell} onSave={(v) => update({ terminalShell: v.trim() })} placeholder="/bin/bash" />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
