import { useEffect, useState } from "react";
import { Icon, type IconName } from "./Icon";
import { useApp } from "../store/app";
import { api, errorText } from "../lib/ipc";
import { activeModel, modelPatch, providerReady } from "../lib/models";
import type { ModelInfo, Provider } from "../lib/types";

const PROVIDERS: { id: Provider; title: string; text: string; icon: IconName }[] = [
  { id: "anthropic", title: "Anthropic", text: "Claude with an API key", icon: "key" },
  { id: "openai", title: "OpenAI", text: "GPT models with an API key", icon: "bulb" },
  { id: "google", title: "Google", text: "Gemini via AI Studio", icon: "globe" },
  { id: "xai", title: "xAI", text: "Grok models", icon: "chat" },
  { id: "openrouter", title: "OpenRouter", text: "Hundreds of models, one key", icon: "plug" },
  { id: "local", title: "Local", text: "Ollama, LM Studio, vLLM, llama.cpp", icon: "cpu" },
];

/** API-key providers with a model list: where to get a key, key prefix, env var. */
const KEYED: Partial<Record<Provider, { help: string; placeholder: string; env: string }>> = {
  openai: { help: "Create a key at platform.openai.com/api-keys.", placeholder: "sk-…", env: "OPENAI_API_KEY" },
  google: { help: "Create a key at aistudio.google.com/apikey (Gemini API).", placeholder: "AIza…", env: "GEMINI_API_KEY" },
  xai: { help: "Create a key at console.x.ai.", placeholder: "xai-…", env: "XAI_API_KEY" },
  openrouter: { help: "One key for Claude, GPT, Gemini, Grok, DeepSeek, Qwen and more. Create it at openrouter.ai/keys.", placeholder: "sk-or-…", env: "OPENROUTER_API_KEY" },
};

type Status = { ok: boolean; text: string } | null;

function StatusLine({ msg }: { msg: Status }) {
  return msg ? <div className={msg.ok ? "onb-ok" : "onb-err"}>{msg.text}</div> : null;
}

/** Secret input + Save/Test. Saving verifies by listing the provider's models. */
function KeyRow({ provider, has, placeholder, optional }: { provider: Provider; has: boolean; placeholder: string; optional?: boolean }) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<Status>(null);
  const save = async () => {
    setBusy(true);
    setMsg(null);
    try {
      if (key.trim()) useApp.getState().setSettingsView(await api.setProviderKey(provider, key.trim()));
      const models = await api.listModels(provider);
      setMsg({
        ok: true,
        text: `Connected — ${models.length} models available`,
      });
      setKey("");
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  const canTest = has || optional;
  return (
    <>
      <div className="onb-row">
        <input
          className="input"
          type="password"
          placeholder={has ? "Saved — paste a new key to replace it" : placeholder}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && (key.trim() || canTest) && void save()}
          data-testid={`key-${provider}`}
        />
        <button className="btn primary" disabled={busy || (!key.trim() && !canTest)} onClick={() => void save()}>
          {busy ? <div className="spinner" /> : key.trim() ? "Save" : "Test"}
        </button>
      </div>
      <StatusLine msg={msg} />
    </>
  );
}

/** Model dropdown filled from the provider's /models, with a free-text fallback. */
function ModelSelect({ provider, value, onChange, refresh }: { provider: Provider; value: string; onChange: (v: string) => void; refresh: unknown }) {
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const load = () => {
    setErr(null);
    setBusy(true);
    api
      .listModels(provider)
      .then(setModels, (e) => setErr(errorText(e)))
      .finally(() => setBusy(false));
  };
  useEffect(load, [provider, refresh]);

  const options = models ?? [];
  const known = options.some((m) => m.id === value);
  return (
    <>
      <div className="onb-row">
        {options.length > 0 ? (
          <select className="select" value={known ? value : ""} onChange={(e) => onChange(e.target.value)} data-testid={`models-${provider}`}>
            {!known && <option value="">{value || "Choose a model…"}</option>}
            {options.map((m) => (
              <option key={m.id} value={m.id}>
                {m.displayName}
              </option>
            ))}
          </select>
        ) : (
          <input
            className="input"
            value={value}
            placeholder="model id, e.g. qwen3-coder"
            onChange={(e) => onChange(e.target.value)}
            data-testid={`model-${provider}`}
          />
        )}
        <button className="btn" title="Reload models" disabled={busy} onClick={load}>
          {busy ? <div className="spinner" /> : <Icon name="refresh" size={14} />}
        </button>
      </div>
      {err && <div className="onb-err">{err}</div>}
    </>
  );
}

/** Text input that commits on blur / Enter. */
function UrlInput({ value, onSave }: { value: string; onSave: (v: string) => void }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <input
      className="input"
      value={v}
      placeholder="http://localhost:11434/v1"
      onChange={(e) => setV(e.target.value)}
      onBlur={() => v.trim() !== value && onSave(v.trim())}
      onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
      data-testid="local-url"
    />
  );
}

/** Choose where the agent's model comes from and connect it. Used by onboarding and Settings. */
export function ProviderPicker() {
  const s = useApp((st) => st.settings)!;
  const update = useApp.getState().updateSettings;
  const p = s.provider;

  return (
    <div className="provider-picker">
      <div className="provider-cards">
        {PROVIDERS.map((x) => (
          <button
            key={x.id}
            className={`provider-card${p === x.id ? " on" : ""}`}
            onClick={() => void update({ provider: x.id })}
            data-testid={`provider-${x.id}`}
          >
            <span className="provider-icon">
              <Icon name={x.icon} size={16} />
            </span>
            <b>
              {x.title}
              {providerReady(s, x.id) && <Icon name="check" size={12} />}
            </b>
            <span>{x.text}</span>
          </button>
        ))}
      </div>

      <div className="provider-body" key={p}>
        {p === "anthropic" && (
          <>
            <p className="provider-help">Paste a key from console.anthropic.com. It's stored only on this machine, readable only by you.</p>
            {s.hasApiKey && (
              <div className="onb-ok">
                <Icon name="check" size={14} /> {s.apiKeySource === "env" ? "Using ANTHROPIC_API_KEY from your environment" : "A key is already saved"} (
                {s.apiKeyHint})
              </div>
            )}
            <KeyRow provider="anthropic" has={s.hasApiKey} placeholder="sk-ant-…" />
          </>
        )}

        {KEYED[p] && (
          <>
            <p className="provider-help">
              {KEYED[p]!.help} Or set {KEYED[p]!.env}.
            </p>
            <KeyRow provider={p} has={!!s.providerKeys[p]} placeholder={KEYED[p]!.placeholder} />
            {s.providerKeys[p] && (
              <>
                <div className="onb-label">Model</div>
                <ModelSelect provider={p} value={activeModel(s)} refresh={s.providerKeys[p]} onChange={(v) => void update(modelPatch(p, v))} />
              </>
            )}
          </>
        )}

        {p === "local" && (
          <>
            <p className="provider-help">
              Any OpenAI-compatible server. Nothing leaves your machine. Use a model with tool calling (Qwen3-Coder, GPT-OSS, Llama 3.3…).
            </p>
            <div className="onb-label">Server URL</div>
            <div className="onb-row">
              <UrlInput value={s.localBaseUrl} onSave={(v) => void update({ localBaseUrl: v })} />
            </div>
            <div className="onb-label">Model</div>
            <ModelSelect provider="local" value={s.localModel} refresh={s.localBaseUrl} onChange={(v) => void update({ localModel: v })} />
            <div className="onb-label">API key (optional)</div>
            <KeyRow provider="local" has={s.hasLocalKey} placeholder="Only if your server needs one" optional />
          </>
        )}
      </div>
    </div>
  );
}
