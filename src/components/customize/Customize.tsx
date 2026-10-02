import { useEffect } from "react";
import { Icon, type IconName } from "../Icon";
import { useApp, type HubTab } from "../../store/app";
import { SkillsTab } from "./SkillsTab";
import { ToolsTab } from "./ToolsTab";
import { McpTab } from "./McpTab";
import { PluginsTab } from "./PluginsTab";
import { UsageTab } from "./UsageTab";
import { TracesTab } from "./TracesTab";

const TABS: { id: HubTab; label: string; icon: IconName }[] = [
  { id: "skills", label: "Skills", icon: "book" },
  { id: "tools", label: "Custom tools", icon: "wrench" },
  { id: "mcp", label: "MCP servers", icon: "plug" },
  { id: "plugins", label: "Plugins & extensions", icon: "puzzle" },
  { id: "usage", label: "Usage", icon: "chart" },
  { id: "traces", label: "Training traces", icon: "database" },
];

/** Everything that extends or measures the agent, in one place. */
export function Customize() {
  const tab = useApp((s) => s.hubTab);
  const close = () => useApp.getState().setOverlay(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <div className="overlay" onMouseDown={close}>
      <div className="modal hub" onMouseDown={(e) => e.stopPropagation()} data-testid="customize">
        <div className="modal-head">
          <Icon name="puzzle" size={18} />
          <h2>Customize</h2>
          <button className="icon-btn" onClick={close} title="Close (Esc)">
            <Icon name="x" size={16} />
          </button>
        </div>
        <div className="hub-main">
          <nav className="hub-nav">
            {TABS.map((t) => (
              <button key={t.id} className={tab === t.id ? "on" : ""} onClick={() => useApp.setState({ hubTab: t.id })} data-testid={`hub-${t.id}`}>
                <Icon name={t.icon} size={15} />
                {t.label}
              </button>
            ))}
          </nav>
          <div className="hub-pane" key={tab}>
            {tab === "skills" && <SkillsTab />}
            {tab === "tools" && <ToolsTab />}
            {tab === "mcp" && <McpTab />}
            {tab === "plugins" && <PluginsTab />}
            {tab === "usage" && <UsageTab />}
            {tab === "traces" && <TracesTab />}
          </div>
        </div>
      </div>
    </div>
  );
}
