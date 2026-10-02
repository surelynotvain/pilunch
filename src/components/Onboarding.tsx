import { useState } from "react";
import { Icon, Logo } from "./Icon";
import { pickFolder } from "./chat/ChatPanel";
import { MODELS, providerReady } from "../lib/models";
import { useApp } from "../store/app";
import { ProviderPicker } from "./ProviderPicker";
import { basename } from "../lib/util";
import type { Effort, PermissionMode, Theme } from "../lib/types";

const MODEL_INFO: Record<string, string> = {
  "claude-opus-5-5": "Best for hard, multi-step coding work",
  "claude-sonnet-5-5": "Fast and capable for everyday tasks",
  "claude-fable-5-1": "Anthropic's most capable model",
  "claude-haiku-4-5": "Quickest answers, lowest cost",
};

const MODES: { id: PermissionMode; title: string; text: string; icon: "shield" | "edit" | "eye" | "alert" }[] = [
  { id: "ask", title: "Ask first", text: "Review every edit and command before it runs.", icon: "shield" },
  { id: "acceptEdits", title: "Auto-accept edits", text: "Edits apply instantly; commands still ask.", icon: "edit" },
  { id: "plan", title: "Plan only", text: "Read-only. Claude explores and proposes.", icon: "eye" },
  { id: "bypass", title: "Full autonomy", text: "No approvals. Use in sandboxes only.", icon: "alert" },
];

const STEPS = ["Welcome", "Connect", "Model", "Permissions", "Project"];

/** First-run setup: API key, model & effort, theme, permissions, first project. */
export function Onboarding() {
  const s = useApp((st) => st.settings)!;
  const update = useApp.getState().updateSettings;
  const [step, setStep] = useState(0);
  const [dir, setDir] = useState<1 | -1>(1);

  const go = (n: number) => {
    setDir(n > step ? 1 : -1);
    setStep(n);
  };
  const finish = () => void update({ onboarded: true });

  return (
    <div className="onboarding" data-testid="onboarding">
      <div className="onb-card">
        <div className="onb-steps">
          {STEPS.map((name, i) => (
            <button key={name} className={i === step ? "on" : i < step ? "done" : ""} onClick={() => i < step && go(i)}>
              <span className="n">{i < step ? <Icon name="check" size={11} /> : i + 1}</span>
              {name}
            </button>
          ))}
        </div>

        <div className={`onb-body step-anim-${dir > 0 ? "fwd" : "back"}`} key={step}>
          {step === 0 && (
            <div className="onb-hero">
              <div className="onb-logo">
                <Logo size={84} />
              </div>
              <h1>Welcome to PiLunch</h1>
              <p>An AI code editor that reads, edits and runs your code with Claude — fast, native, and always under your control.</p>
              <ul className="onb-points">
                <li>
                  <Icon name="chat" size={15} /> Chat-first agent with 20 built-in tools
                </li>
                <li>
                  <Icon name="shield" size={15} /> Every change shown as a diff before it lands
                </li>
                <li>
                  <Icon name="terminal" size={15} /> Editor, terminal, git and search built in
                </li>
              </ul>
            </div>
          )}

          {step === 1 && (
            <div>
              <h2>Connect a model</h2>
              <p className="lead">Use Claude with an API key, any model through OpenRouter, or a model running on this machine.</p>
              <ProviderPicker />
            </div>
          )}

          {step === 2 && (
            <div>
              <h2>Choose your defaults</h2>
              <p className="lead">You can switch model and effort any time from the composer.</p>
              {s.provider === "anthropic" && (
                <div className="onb-grid two">
                  {MODELS.map((m) => (
                    <button key={m.id} className={`onb-option${s.model === m.id ? " on" : ""}`} onClick={() => void update({ model: m.id })}>
                      <b>Claude {m.label}</b>
                      <span>{MODEL_INFO[m.id]}</span>
                    </button>
                  ))}
                </div>
              )}
              <div className="onb-label">Thinking effort</div>
              <div className="segmented wide">
                {(["low", "medium", "high", "xhigh", "max"] as Effort[]).map((e) => (
                  <button key={e} className={s.effort === e ? "on" : ""} onClick={() => void update({ effort: e })}>
                    {e === "xhigh" ? "Extra" : e[0]!.toUpperCase() + e.slice(1)}
                  </button>
                ))}
              </div>
              <div className="onb-label">Theme</div>
              <div className="segmented wide">
                {(["dark", "light", "system"] as Theme[]).map((t) => (
                  <button key={t} className={s.theme === t ? "on" : ""} onClick={() => void update({ theme: t })}>
                    {t[0]!.toUpperCase() + t.slice(1)}
                  </button>
                ))}
              </div>
            </div>
          )}

          {step === 3 && (
            <div>
              <h2>How much should Claude do on its own?</h2>
              <p className="lead">Change it per chat from the composer.</p>
              <div className="onb-grid two">
                {MODES.map((m) => (
                  <button key={m.id} className={`onb-option${s.permissionMode === m.id ? " on" : ""}${m.id === "bypass" ? " danger" : ""}`} onClick={() => void update({ permissionMode: m.id })}>
                    <b>
                      <Icon name={m.icon} size={14} /> {m.title}
                    </b>
                    <span>{m.text}</span>
                  </button>
                ))}
              </div>
              <label className="onb-toggle">
                <button className={`switch${s.webSearch ? " on" : ""}`} onClick={() => void update({ webSearch: !s.webSearch })} role="switch" aria-checked={s.webSearch} />
                <span>
                  <b>Web search</b> — let Claude look up docs and answers (billed per search)
                </span>
              </label>
            </div>
          )}

          {step === 4 && (
            <div>
              <h2>Open a project</h2>
              <p className="lead">PiLunch works inside one folder at a time and never touches files outside it.</p>
              <button
                className="onb-option big"
                onClick={async () => {
                  await pickFolder();
                  if (useApp.getState().workspace) finish();
                }}
              >
                <b>
                  <Icon name="folderOpen" size={16} /> Open a folder…
                </b>
                <span>Choose any project directory</span>
              </button>
              {s.recentWorkspaces.slice(0, 4).map((r) => (
                <button
                  key={r}
                  className="onb-option row"
                  onClick={async () => {
                    if (await useApp.getState().openWorkspace(r)) finish();
                  }}
                  data-testid="onb-recent"
                >
                  <Icon name="folder" size={15} />
                  <b>{basename(r)}</b>
                  <span className="ellipsis">{r}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="onb-foot">
          {step > 0 ? (
            <button className="btn ghost" onClick={() => go(step - 1)}>
              Back
            </button>
          ) : (
            <span />
          )}
          <span className="spacer" />
          {step === 4 ? (
            <button className="btn ghost" onClick={finish} data-testid="onb-finish">
              Skip — just chat
            </button>
          ) : (
            <button
              className="btn primary"
              onClick={() => go(step + 1)}
              disabled={step === 1 && !providerReady(s)}
              title={step === 1 && !providerReady(s) ? "Connect a provider first (or skip setup)" : undefined}
              data-testid="onb-next"
            >
              {step === 0 ? "Get started" : "Continue"} <Icon name="chevronRight" size={14} />
            </button>
          )}
        </div>
      </div>
      <button className="onb-skip" onClick={finish}>
        Skip setup
      </button>
    </div>
  );
}
