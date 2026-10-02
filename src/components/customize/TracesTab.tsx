import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { Icon } from "../Icon";
import { useApp } from "../../store/app";
import { api, errorText } from "../../lib/ipc";
import { formatBytes } from "../../lib/util";
import type { TraceStats } from "../../lib/types";

const SAMPLE = `{"id": "…", "provider": "local", "model": "qwen3-coder", "thinking_level": "max",
 "tools": [{"type": "function", "function": {"name": "read_file", …}}],
 "messages": [
   {"role": "system", "content": "You are PiLunch…"},
   {"role": "user", "content": "Fix the failing test"},
   {"role": "assistant", "reasoning_content": "Let me look…", "content": null,
    "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "grep", "arguments": "{…}"}}]},
   {"role": "tool", "tool_call_id": "call_1", "content": "src/lib.rs:12: …"},
   {"role": "assistant", "content": "Fixed: …"}]}`;

export function TracesTab() {
  const s = useApp((st) => st.settings)!;
  const update = useApp.getState().updateSettings;
  const toast = useApp.getState().toast;
  const [stats, setStats] = useState<TraceStats | null>(null);
  const [localOnly, setLocalOnly] = useState(false);
  const load = () => api.traceStats().then(setStats, (e) => toast(errorText(e), "error"));
  useEffect(() => void load(), []);

  const exportAll = async () => {
    const path = await save({
      title: "Export traces",
      defaultPath: `pilunch-traces${localOnly ? "-local" : ""}.jsonl`,
      filters: [{ name: "JSON Lines", extensions: ["jsonl"] }],
    });
    if (!path) return;
    try {
      const n = await api.exportTraces(path, localOnly);
      toast(`Exported ${n} traces to ${path}`);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <>
      <h3>Training traces</h3>
      <p className="lead">
        Save everything the agent does, including its thinking, tool calls and tool results, as a dataset for fine-tuning models on agentic coding. Each chat is
        one trace in chat-completions format (<code>reasoning_content</code> + <code>tool_calls</code>), updated after every run. Screenshots are replaced with
        a marker. Traces never leave this machine.
      </p>
      <div className="settings-section" style={{ padding: 0, border: 0 }}>
        <div className="field">
          <label>
            Save traces
            <span className="help">Record new runs from now on.</span>
          </label>
          <button
            className={`switch${s.saveTraces ? " on" : ""}`}
            role="switch"
            aria-checked={s.saveTraces}
            onClick={() => void update({ saveTraces: !s.saveTraces })}
            data-testid="traces-toggle"
          />
        </div>
        <div className="field">
          <label>
            Which runs
            <span className="help">Only local models (for distilling your own agent), or every provider.</span>
          </label>
          <select className="select" value={s.tracesScope} onChange={(e) => void update({ tracesScope: e.target.value as "all" | "local" })}>
            <option value="all">All providers</option>
            <option value="local">Local models only</option>
          </select>
        </div>
      </div>
      {stats && (
        <div className="stat-cards" style={{ gridTemplateColumns: "repeat(2, 1fr)", marginTop: 14 }}>
          <div className="stat-card">
            <div className="k">Traces</div>
            <div className="v" data-testid="trace-count">
              {stats.count}
            </div>
          </div>
          <div className="stat-card">
            <div className="k">Size</div>
            <div className="v">{formatBytes(stats.bytes)}</div>
          </div>
        </div>
      )}
      <div className="hub-actions">
        <button className="btn primary sm" disabled={!stats?.count} onClick={() => void exportAll()}>
          <Icon name="database" size={13} /> Export JSONL…
        </button>
        <label className="onb-toggle" style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12.5 }}>
          <input type="checkbox" checked={localOnly} onChange={(e) => setLocalOnly(e.target.checked)} /> Local-model traces only
        </label>
        <span style={{ flex: 1 }} />
        {stats && (
          <button className="btn ghost sm" onClick={() => void api.openFolder(stats.dir)}>
            <Icon name="folderOpen" size={13} /> Open folder
          </button>
        )}
        <button className="btn ghost sm" disabled={!stats?.count} onClick={() => confirm("Delete all saved traces?") && void api.clearTraces().then(load)}>
          <Icon name="trash" size={13} /> Delete all
        </button>
      </div>
      <div className="onb-label">Format (one JSON object per line)</div>
      <pre className="hub-log">{SAMPLE}</pre>
    </>
  );
}
