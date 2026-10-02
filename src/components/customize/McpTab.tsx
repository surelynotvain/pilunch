import { useEffect, useState } from "react";
import { Icon } from "../Icon";
import { useApp } from "../../store/app";
import { api, errorText } from "../../lib/ipc";
import type { McpServerStatus } from "../../lib/types";

const EXAMPLES: { label: string; name: string; config: Record<string, unknown> }[] = [
  { label: "Filesystem", name: "filesystem", config: { command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "/home"] } },
  {
    label: "GitHub",
    name: "github",
    config: { command: "npx", args: ["-y", "@modelcontextprotocol/server-github"], env: { GITHUB_PERSONAL_ACCESS_TOKEN: "<token>" } },
  },
  { label: "Fetch (Python)", name: "fetch", config: { command: "uvx", args: ["mcp-server-fetch"] } },
  { label: "Remote (HTTP)", name: "remote", config: { url: "https://example.com/mcp", headers: { Authorization: "Bearer <token>" } } },
];

export function McpTab() {
  const [raw, setRaw] = useState("");
  const [saved, setSaved] = useState("");
  const [status, setStatus] = useState<McpServerStatus[] | null>(null);
  const [checking, setChecking] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const toast = useApp.getState().toast;

  const check = async (reconnect?: string) => {
    setChecking(true);
    try {
      setStatus(await api.mcpStatus(reconnect));
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setChecking(false);
    }
  };

  useEffect(() => {
    void api.mcpConfig().then((r) => {
      setRaw(r);
      setSaved(r);
    });
    void check();
  }, []);

  const save = async () => {
    try {
      await api.saveMcpConfig(raw);
      setSaved(raw);
      toast("Saved mcp.json");
      await check();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const addExample = (ex: (typeof EXAMPLES)[number]) => {
    try {
      const v = JSON.parse(raw || "{}");
      v.mcpServers = { ...(v.mcpServers ?? {}), [ex.name]: ex.config };
      setRaw(JSON.stringify(v, null, 2));
    } catch {
      toast("Fix the JSON first", "error");
    }
  };

  return (
    <>
      <h3>MCP servers</h3>
      <p className="lead">
        Connect Model Context Protocol servers to give the agent their tools (named <code>mcp__server__tool</code>). Local servers run as a command over stdio;
        remote ones use a URL (Streamable HTTP). The format matches Claude Desktop's <code>mcpServers</code>, so you can paste existing configs. Tools marked
        read-only run without asking; others ask first.
      </p>
      <div className="hub-actions">
        <button className="btn sm" disabled={checking} onClick={() => void check()}>
          {checking ? <div className="spinner" /> : <Icon name="refresh" size={13} />} Check connections
        </button>
      </div>
      <div className="hub-list" style={{ marginBottom: 18 }}>
        {status?.length === 0 && <div className="hub-empty">No servers configured yet.</div>}
        {status?.map((s) => (
          <div key={s.name} className="hub-item" style={{ cursor: "default" }}>
            <span className="title">
              <span className={`status-dot ${s.state}`} />
              {s.name}
              <span className="tag">{s.transport}</span>
              {s.source !== "mcp.json" && <span className="tag">{s.source.replace("plugin:", "Plugin: ")}</span>}
              <span style={{ flex: 1 }} />
              {s.state === "connected" && (
                <button className="btn ghost sm" onClick={() => setOpen(open === s.name ? null : s.name)}>
                  {s.tools.length} tools
                </button>
              )}
              {s.state !== "disabled" && (
                <button className="btn ghost sm" title="Reconnect" onClick={() => void check(s.name)}>
                  <Icon name="refresh" size={13} />
                </button>
              )}
            </span>
            {s.state === "error" && (
              <span className="sub" style={{ whiteSpace: "pre-wrap", color: "var(--err)" }}>
                {s.error}
              </span>
            )}
            {s.state === "disabled" && <span className="sub">Disabled</span>}
            {open === s.name && (
              <span className="sub" style={{ whiteSpace: "normal" }}>
                {s.tools.join(", ")}
              </span>
            )}
          </div>
        ))}
      </div>
      <div className="hub-actions">
        <b style={{ fontSize: 13 }}>mcp.json</b>
        <span className="muted">Add an example:</span>
        {EXAMPLES.map((ex) => (
          <button key={ex.name} className="btn ghost sm" onClick={() => addExample(ex)}>
            + {ex.label}
          </button>
        ))}
      </div>
      <textarea
        className="textarea code-area"
        rows={14}
        value={raw}
        spellCheck={false}
        onChange={(e) => setRaw(e.target.value)}
        style={{ width: "100%" }}
        data-testid="mcp-json"
      />
      <div className="hub-actions" style={{ marginTop: 10 }}>
        <button className="btn primary sm" disabled={raw === saved} onClick={() => void save()} data-testid="mcp-save">
          Save & connect
        </button>
        <span className="muted">Stored with owner-only permissions (it may contain tokens). Set "disabled": true to turn a server off.</span>
      </div>
    </>
  );
}
