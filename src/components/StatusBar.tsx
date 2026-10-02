import { Icon } from "./Icon";
import { useApp } from "../store/app";
import { useChat } from "../store/chat";
import { useEditor } from "../store/editor";
import { formatTokens } from "../lib/util";

const MODE_LABEL = { ask: "Ask", acceptEdits: "Auto-edit", plan: "Plan", bypass: "Bypass" } as const;

export function StatusBar() {
  const workspace = useApp((s) => s.workspace);
  const git = useApp((s) => s.git);
  const settings = useApp((s) => s.settings);
  const cursor = useEditor((s) => s.cursor);
  const language = useEditor((s) => s.language);
  const hasTabs = useEditor((s) => s.tabs.length > 0);
  const runningCount = useChat((s) => Object.values(s.runs).filter((r) => r.running).length);
  const usage = useChat((s) => (s.activeId ? s.convs[s.activeId]?.usage : undefined));
  const changes = git ? Object.keys(git.files).length : 0;
  const app = useApp.getState();

  return (
    <div className="statusbar" data-testid="statusbar">
      {workspace && (
        <div className="item clickable" title={workspace.root} onClick={() => app.setLayout({ sidebarVisible: true, sidebarView: "explorer" })}>
          <Icon name="folder" size={13} /> {workspace.name}
        </div>
      )}
      {git?.branch && (
        <div className="item clickable" title={`${changes} changed files`} onClick={() => void app.refreshGit()}>
          <Icon name="branch" size={13} /> {git.branch}
          {git.ahead > 0 && ` ↑${git.ahead}`}
          {git.behind > 0 && ` ↓${git.behind}`}
          {changes > 0 && <span className="git-M"> ●{changes}</span>}
        </div>
      )}
      {runningCount > 0 && (
        <div className="item">
          <div className="spinner" style={{ width: 11, height: 11 }} />
          Claude is working{runningCount > 1 ? ` (${runningCount} chats)` : ""}…
        </div>
      )}
      <span className="spacer" />
      {hasTabs && cursor && (
        <div className="item">
          Ln {cursor.line}, Col {cursor.col}
          {cursor.selected > 0 && ` (${cursor.selected} selected)`}
        </div>
      )}
      {hasTabs && language && <div className="item">{language}</div>}
      {usage && usage.outputTokens > 0 && (
        <div
          className="item"
          title={`Input ${usage.inputTokens.toLocaleString()} · cache read ${usage.cacheReadTokens.toLocaleString()} · cache write ${usage.cacheWriteTokens.toLocaleString()} · output ${usage.outputTokens.toLocaleString()}`}
        >
          ↑{formatTokens(usage.inputTokens + usage.cacheReadTokens + usage.cacheWriteTokens)} ↓{formatTokens(usage.outputTokens)}
        </div>
      )}
      {settings && workspace && (
        <div className={`item clickable${settings.permissionMode === "bypass" ? " git-D" : ""}`} title="Permission mode" onClick={() => app.setOverlay("commands")}>
          <Icon name="shield" size={13} /> {MODE_LABEL[settings.permissionMode]}
        </div>
      )}
      {settings && (
        <div className="item clickable" title="Model and effort (Settings)" onClick={() => app.setOverlay("settings")}>
          <Icon name="bulb" size={13} /> {settings.model.replace(/^claude-/, "")} · {settings.effort}
        </div>
      )}
      <div className="item clickable" title="Toggle terminal (Ctrl+J)" onClick={() => app.setLayout({ terminalVisible: !app.layout.terminalVisible })}>
        <Icon name="terminal" size={13} />
      </div>
    </div>
  );
}
