import { open } from "@tauri-apps/plugin-dialog";
import { Icon, Logo } from "../Icon";
import { MessageList } from "./MessageList";
import { Composer } from "./Composer";
import { PlanPanel } from "./PlanPanel";
import { useChat } from "../../store/chat";
import { useApp } from "../../store/app";
import { useEditor } from "../../store/editor";
import { ProviderPicker } from "../ProviderPicker";
import { providerReady } from "../../lib/models";
import { basename, formatTokens } from "../../lib/util";

export { MODELS } from "../../lib/models";

export async function pickFolder() {
  const dir = await open({ directory: true, multiple: false, title: "Open folder" });
  if (typeof dir === "string") {
    const ok = await useApp.getState().openWorkspace(dir);
    if (ok) {
      useChat.getState().newChat();
      void useEditor.getState().closeAll();
    }
  }
}

function ConnectCard() {
  return (
    <div className="connect-card">
      <b>
        <Icon name="key" size={13} /> Connect a model
      </b>
      <ProviderPicker />
    </div>
  );
}

const NO_RECENTS: string[] = [];

const SUGGESTIONS = [
  { title: "Explain this codebase", prompt: "Give me a tour of this codebase: what it does, how it's structured, and the key entry points." },
  { title: "Find and fix a bug", prompt: "Look for a likely bug in this project, explain it, and fix it." },
  { title: "Write tests", prompt: "Find an important function that has no tests and add tests for it, then run them." },
  { title: "Review my changes", prompt: "Review my uncommitted changes (git diff) for bugs and suggest improvements." },
];

function EmptyChat({ docked }: { docked: boolean }) {
  const workspace = useApp((s) => s.workspace);
  const hasKey = useApp((s) => (s.settings ? providerReady(s.settings) : false));
  const recents = useApp((s) => s.settings?.recentWorkspaces ?? NO_RECENTS);
  const setComposer = useChat((s) => s.setComposer);
  return (
    <div className="welcome">
      <Logo size={docked ? 40 : 56} />
      <h1>
        {workspace ? (
          <>
            What should we build in <span className="grad">{workspace.name}</span>?
          </>
        ) : (
          <>
            Welcome to <span className="grad">PiLunch</span>
          </>
        )}
      </h1>
      {!workspace && <p>An AI code editor for Claude, OpenRouter and local models. Open a project folder and PiLunch can read, edit and run your code — always asking first.</p>}
      {!hasKey && <ConnectCard />}
      {!workspace && (
        <>
          <div className="actions">
            <button className="btn primary lg" onClick={() => void pickFolder()} data-testid="open-folder">
              <Icon name="folderOpen" size={17} /> Open Folder
            </button>
          </div>
          {recents.length > 0 && (
            <div className="recents">
              <div className="chat-group">Recent</div>
              {recents.slice(0, 6).map((r) => (
                <div
                  key={r}
                  className="item"
                  onClick={() => void useApp.getState().openWorkspace(r)}
                  title={r}
                  data-testid="recent-workspace"
                >
                  <Icon name="folder" size={15} />
                  <span>{basename(r)}</span>
                  <span className="muted ellipsis" style={{ fontSize: 12 }}>
                    {r}
                  </span>
                </div>
              ))}
            </div>
          )}
        </>
      )}
      {workspace && hasKey && (
        <div className="suggestions">
          {SUGGESTIONS.map((s) => (
            <button key={s.title} className="suggestion" onClick={() => setComposer({ text: s.prompt, focusSeq: Date.now() })}>
              <b>{s.title}</b>
              {s.prompt}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function ChatPanel({ docked }: { docked: boolean }) {
  const activeId = useChat((s) => s.activeId);
  const conv = useChat((s) => (s.activeId ? s.convs[s.activeId] : undefined));
  const contextTokens = useChat((s) => (s.activeId ? s.runs[s.activeId]?.contextTokens : undefined));
  const running = useChat((s) => (s.activeId ? !!s.runs[s.activeId]?.running : false));
  const chatFocus = useApp((s) => s.layout.chatFocus);
  const hasTabs = useEditor((s) => s.tabs.length > 0);
  const empty = (!conv || conv.messages.length === 0) && !running;

  return (
    <div className={`chat${docked ? " docked" : ""}`} data-testid="chat-panel">
      <div className="chat-header">
        <span className="title ellipsis">{conv && !empty ? conv.title : "New chat"}</span>
        {contextTokens != null && (
          <span className="muted" style={{ fontSize: 11.5 }} title="Tokens in the conversation context">
            {formatTokens(contextTokens)} ctx
          </span>
        )}
        {hasTabs && (
          <button
            className="icon-btn"
            title={chatFocus ? "Show editor (dock chat)" : "Focus chat"}
            onClick={() => useApp.getState().setLayout({ chatFocus: !chatFocus })}
          >
            <Icon name={chatFocus ? "shrink" : "expand"} size={15} />
          </button>
        )}
        <button className="icon-btn" title="New chat (Ctrl+N)" onClick={() => useChat.getState().newChat()} data-testid="new-chat">
          <Icon name="plus" size={16} />
        </button>
      </div>
      {empty || !activeId ? <EmptyChat docked={docked} /> : <MessageList convId={activeId} />}
      {activeId && <PlanPanel convId={activeId} />}
      <Composer convId={activeId} />
    </div>
  );
}
