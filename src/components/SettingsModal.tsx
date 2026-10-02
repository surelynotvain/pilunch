import { useEffect, useState } from "react";
import { Icon } from "./Icon";
import { MODELS } from "../lib/models";
import { useApp } from "../store/app";
import { api, errorText } from "../lib/ipc";
import type { Effort, ModelInfo, PermissionMode, Settings, Theme } from "../lib/types";

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
  const [key, setKey] = useState("");
  const [testing, setTesting] = useState(false);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [testMsg, setTestMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [instructions, setInstructions] = useState(s?.customInstructions ?? "");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!s) return null;

  const saveKey = async () => {
    try {
      useApp.getState().setSettingsView(await api.setApiKey(key));
      setKey("");
      await test();
    } catch (e) {
      setTestMsg({ ok: false, text: errorText(e) });
    }
  };

  const test = async () => {
    setTesting(true);
    setTestMsg(null);
    try {
      const list = await api.listModels();
      setModels(list);
      setTestMsg({ ok: true, text: `Connected — ${list.length} models available` });
    } catch (e) {
      setTestMsg({ ok: false, text: errorText(e) });
    } finally {
      setTesting(false);
    }
  };

  const modelOptions = [...MODELS];
  for (const m of models ?? []) if (!modelOptions.some((x) => x.id === m.id)) modelOptions.push({ id: m.id, label: m.displayName });
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
            <h3>Claude</h3>
            <div className="field">
              <label>
                API key
                <span className="help">
                  {s.hasApiKey ? (
                    <span className="key-status ok">
                      <Icon name="check" size={12} /> {s.apiKeySource === "env" ? "From ANTHROPIC_API_KEY" : "Saved"} {s.apiKeyHint}
                    </span>
                  ) : (
                    "Not configured"
                  )}
                </span>
              </label>
              <div>
                <div className="row">
                  <input
                    className="input"
                    type="password"
                    placeholder={s.hasApiKey ? "Enter a new key to replace it" : "sk-ant-…"}
                    value={key}
                    onChange={(e) => setKey(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && key.trim() && void saveKey()}
                    data-testid="settings-api-key"
                  />
                  <button className="btn primary" disabled={!key.trim()} onClick={() => void saveKey()}>
                    Save
                  </button>
                  <button className="btn" disabled={!s.hasApiKey || testing} onClick={() => void test()}>
                    {testing ? <div className="spinner" /> : "Test"}
                  </button>
                </div>
                {testMsg && (
                  <span className="help" style={{ color: testMsg.ok ? "var(--ok)" : "var(--err)" }}>
                    {testMsg.text}
                  </span>
                )}
                <span className="help">Stored in {s.configDir}/secrets.json (owner-only permissions). It never leaves this machine except to the API.</span>
              </div>
            </div>
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
            <div className="field">
              <label>
                Effort
                <span className="help">How hard Claude thinks. Higher is smarter but slower and costs more.</span>
              </label>
              <select className="select" value={s.effort} onChange={(e) => update({ effort: e.target.value as Effort })}>
                <option value="low">Low — fastest</option>
                <option value="medium">Medium</option>
                <option value="high">High (recommended)</option>
                <option value="xhigh">Extra high</option>
                <option value="max">Max</option>
              </select>
            </div>
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
            <div className="field">
              <label>
                API endpoint
                <span className="help">Only change this for a proxy or gateway.</span>
              </label>
              <Lazy value={s.baseUrl} onSave={(v) => update({ baseUrl: v.trim() })} placeholder="https://api.anthropic.com" />
            </div>
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
