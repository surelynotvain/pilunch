import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Icon } from "../Icon";
import { useApp } from "../../store/app";
import { api, errorText } from "../../lib/ipc";
import type { BuildResult, Plugin } from "../../lib/types";

const DOCS = "https://github.com/surelynotvain/pilunch/blob/main/docs/extensions.md";

export function PluginsTab() {
  const configDir = useApp((s) => s.settings?.configDir ?? "");
  const [plugins, setPlugins] = useState<Plugin[] | null>(null);
  const [git, setGit] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [build, setBuild] = useState<BuildResult | null>(null);
  const toast = useApp.getState().toast;
  const load = () => api.listPlugins().then(setPlugins, (e) => toast(errorText(e), "error"));
  useEffect(() => void load(), []);

  const run = async (label: string, f: () => Promise<unknown>, done?: string) => {
    setBusy(label);
    try {
      await f();
      if (done) toast(done);
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(null);
    }
  };

  const fromFolder = async () => {
    const dir = await open({ directory: true, title: "Plugin folder" });
    if (typeof dir === "string") await run("folder", () => api.installPluginFolder(dir), "Plugin installed");
  };

  const buildRust = async () => {
    const dir = await open({ directory: true, title: "Rust extension crate (folder with Cargo.toml)" });
    if (typeof dir !== "string") return;
    setBuild(null);
    await run("rust", async () => {
      const r = await api.buildRustExtension(dir);
      setBuild(r);
      if (r.ok) toast(`Built and installed ${r.id}`);
    });
  };

  const toggle = async (p: Plugin) => {
    try {
      useApp.getState().setSettingsView(await api.setPluginEnabled(p.id, !p.enabled));
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <>
      <h3>Plugins & extensions</h3>
      <p className="lead">
        A plugin is a folder that bundles skills, custom tools and MCP servers. A Rust extension is a plugin whose tools are a Rust program built with the{" "}
        <code>pilunch-extension</code> crate. PiLunch compiles it with cargo and runs it for you. Plugins live in {configDir}/plugins.{" "}
        <a href="#" onClick={(e) => (e.preventDefault(), void openUrl(DOCS))}>
          Extension guide
        </a>
      </p>
      <div className="hub-actions">
        <button className="btn primary sm" disabled={!!busy} onClick={() => void buildRust()} data-testid="plugin-rust">
          {busy === "rust" ? <div className="spinner" /> : <Icon name="cpu" size={13} />} Build Rust extension…
        </button>
        <button className="btn sm" disabled={!!busy} onClick={() => void fromFolder()}>
          <Icon name="folderOpen" size={13} /> Install from folder…
        </button>
        <div className="onb-row" style={{ flex: 1, minWidth: 260 }}>
          <input className="input" placeholder="https://github.com/user/pilunch-plugin.git" value={git} onChange={(e) => setGit(e.target.value)} />
          <button
            className="btn sm"
            disabled={!!busy || !git.trim()}
            onClick={() => void run("git", () => api.installPluginGit(git), "Plugin installed").then(() => setGit(""))}
          >
            {busy === "git" ? <div className="spinner" /> : "Install from git"}
          </button>
        </div>
      </div>
      {build && (
        <div style={{ marginBottom: 14 }}>
          <div className={build.ok ? "onb-ok" : "onb-err"} style={{ marginBottom: 6 }}>
            {build.ok ? `Installed ${build.id}` : build.error}
          </div>
          <pre className="hub-log">{build.log}</pre>
        </div>
      )}
      <div className="hub-list">
        {plugins?.length === 0 && <div className="hub-empty">No plugins installed.</div>}
        {plugins?.map((p) => (
          <div key={p.id} className="hub-item" style={{ cursor: "default" }} data-testid="plugin-item">
            <span className="title">
              <Icon name={p.kind === "rust-extension" ? "cpu" : "puzzle"} size={14} />
              {p.name}
              {p.version && <span className="tag">v{p.version}</span>}
              {p.kind === "rust-extension" && <span className="tag accent">Rust extension</span>}
              <span style={{ flex: 1 }} />
              <button
                className={`switch${p.enabled ? " on" : ""}`}
                role="switch"
                aria-checked={p.enabled}
                title={p.enabled ? "Disable" : "Enable"}
                onClick={() => void toggle(p)}
              />
              <button className="btn ghost sm" title="Open folder" onClick={() => void api.openFolder(p.path)}>
                <Icon name="folderOpen" size={13} />
              </button>
              <button className="btn ghost sm" title="Remove" onClick={() => confirm(`Remove ${p.name}?`) && void run("rm", () => api.removePlugin(p.id))}>
                <Icon name="trash" size={13} />
              </button>
            </span>
            {p.description && <span className="sub">{p.description}</span>}
            <span className="sub">
              {[p.skills && `${p.skills} skills`, p.tools && `${p.tools} tools`, p.servers.length && `MCP: ${p.servers.join(", ")}`]
                .filter(Boolean)
                .join(" · ") || "Empty"}
            </span>
            {p.error && (
              <span className="sub" style={{ color: "var(--err)" }}>
                {p.error}
              </span>
            )}
          </div>
        ))}
      </div>
    </>
  );
}
