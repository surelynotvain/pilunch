import { create } from "zustand";
import { api, errorText } from "../lib/ipc";
import type { AgentEvent, Conversation, ConversationMeta, Decision, ToolUi } from "../lib/types";
import { useApp } from "./app";
import { useEditor } from "./editor";

export interface DraftBlock {
  index: number;
  kind: "text" | "thinking" | "tool_use" | "server_tool_use" | "other";
  text: string;
  toolId?: string;
  toolName?: string;
  bytes?: number;
  input?: Record<string, unknown>;
  summary?: string;
}

export interface Approval {
  approvalId: string;
  toolId: string;
  kind: "edit" | "command" | "network";
  title: string;
  detail: string;
}

export interface RunNotice {
  kind: "notice" | "error" | "refusal";
  text: string;
}

export interface LiveRun {
  running: boolean;
  startedAt: number;
  draft: DraftBlock[];
  approvals: Approval[];
  outputs: Record<string, string>;
  model?: string;
  retrying?: string;
  notices: RunNotice[];
  contextTokens?: number;
}

interface Composer {
  text: string;
  attachments: string[];
  /** Bumped to ask the composer to focus itself. */
  focusSeq: number;
}

interface ChatState {
  list: ConversationMeta[];
  activeId: string | null;
  convs: Record<string, Conversation>;
  runs: Record<string, LiveRun>;
  composer: Composer;

  loadList(): Promise<void>;
  open(id: string): Promise<void>;
  newChat(): void;
  send(): Promise<void>;
  cancel(id?: string): void;
  respond(approvalId: string, decision: Decision, feedback?: string): Promise<void>;
  rename(id: string, title: string): Promise<void>;
  remove(id: string): Promise<void>;
  setComposer(patch: Partial<Composer>): void;
  attach(path: string): void;
  insertText(text: string): void;
  focusComposer(): void;
}

const MAX_OUTPUT = 200_000;

function emptyRun(): LiveRun {
  return { running: true, startedAt: Date.now(), draft: [], approvals: [], outputs: {}, notices: [] };
}

export const useChat = create<ChatState>((set, get) => {
  /** Apply an update to one conversation's run. */
  const patchRun = (id: string, f: (r: LiveRun) => Partial<LiveRun>) => {
    const run = get().runs[id];
    if (!run) return;
    set({ runs: { ...get().runs, [id]: { ...run, ...f(run) } } });
  };
  const patchConv = (id: string, f: (c: Conversation) => Partial<Conversation>) => {
    const conv = get().convs[id];
    if (!conv) return;
    set({ convs: { ...get().convs, [id]: { ...conv, ...f(conv) } } });
  };
  const patchDraft = (id: string, index: number, f: (b: DraftBlock) => Partial<DraftBlock>) =>
    patchRun(id, (r) => ({ draft: r.draft.map((b) => (b.index === index ? { ...b, ...f(b) } : b)) }));

  function handle(id: string, ev: AgentEvent) {
    switch (ev.type) {
      case "messageAppended":
        patchConv(id, (c) => ({ messages: [...c.messages, ev.message], messageCount: c.messages.length + 1 }));
        if (ev.message.role === "assistant") patchRun(id, () => ({ draft: [] }));
        break;
      case "title":
        patchConv(id, () => ({ title: ev.title }));
        set({ list: get().list.map((m) => (m.id === id ? { ...m, title: ev.title } : m)) });
        break;
      case "requestStarted":
        patchRun(id, () => ({ model: ev.model, retrying: undefined }));
        break;
      case "blockStart":
        patchRun(id, (r) => ({
          draft: [...r.draft, { index: ev.index, kind: ev.kind, text: "", toolId: ev.toolId ?? undefined, toolName: ev.toolName ?? undefined }],
        }));
        break;
      case "delta":
        patchDraft(id, ev.index, (b) => ({ text: b.text + ev.text }));
        break;
      case "toolInputProgress":
        patchDraft(id, ev.index, () => ({ bytes: ev.bytes }));
        break;
      case "toolInput":
        patchDraft(id, ev.index, () => ({ input: ev.input, summary: ev.summary }));
        break;
      case "toolStatus":
        patchConv(id, (c) => ({ toolUi: { ...c.toolUi, [ev.toolId]: ev.ui } }));
        onToolStatus(ev.ui);
        break;
      case "toolOutput":
        patchRun(id, (r) => {
          let out = (r.outputs[ev.toolId] ?? "") + ev.text;
          if (out.length > MAX_OUTPUT) out = out.slice(out.length - MAX_OUTPUT);
          return { outputs: { ...r.outputs, [ev.toolId]: out } };
        });
        break;
      case "approvalRequest":
        patchRun(id, (r) => ({ approvals: [...r.approvals, ev] }));
        break;
      case "approvalResolved":
        patchRun(id, (r) => ({ approvals: r.approvals.filter((a) => a.approvalId !== ev.approvalId) }));
        break;
      case "usage":
        patchConv(id, () => ({ usage: ev.totals }));
        patchRun(id, () => ({ contextTokens: ev.contextTokens }));
        break;
      case "retrying":
        patchRun(id, () => ({ retrying: `${ev.message} — retrying in ${Math.ceil(ev.delayMs / 1000)}s (attempt ${ev.attempt})` }));
        break;
      case "reset":
        patchRun(id, () => ({ draft: [] }));
        break;
      case "notice":
      case "error":
      case "refusal":
        patchRun(id, (r) => ({ notices: [...r.notices, { kind: ev.type, text: ev.message }] }));
        break;
      case "done":
        patchRun(id, () => ({ running: false, draft: [], approvals: [], retrying: undefined }));
        void get().loadList();
        break;
    }
  }

  return {
    list: [],
    activeId: null,
    convs: {},
    runs: {},
    composer: { text: "", attachments: [], focusSeq: 0 },

    async loadList() {
      try {
        set({ list: await api.listConversations() });
      } catch (e) {
        useApp.getState().toast(errorText(e), "error");
      }
    },

    async open(id) {
      set({ activeId: id });
      if (get().convs[id] && get().runs[id]?.running) return; // live state is authoritative
      try {
        const conv = await api.getConversation(id);
        set({ convs: { ...get().convs, [id]: conv } });
      } catch (e) {
        useApp.getState().toast(errorText(e), "error");
      }
    },

    newChat() {
      set({ activeId: null });
      get().focusComposer();
    },

    async send() {
      const { composer } = get();
      const text = composer.text.trim();
      if (!text) return;
      let id = get().activeId;
      if (id && get().runs[id]?.running) return;
      if (id) {
        // A chat belongs to the folder it was started in; switch to a new chat if the folder changed.
        const conv = get().convs[id];
        const ws = useApp.getState().workspace?.root ?? null;
        if (conv && conv.workspace !== ws && conv.messages.length > 0) id = null;
      }
      try {
        if (!id) {
          const meta = await api.createConversation();
          const conv: Conversation = {
            ...meta,
            messages: [],
            toolUi: {},
            usage: { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 },
          };
          set({ convs: { ...get().convs, [meta.id]: conv }, list: [meta, ...get().list], activeId: meta.id });
          id = meta.id;
        }
        const convId = id;
        set({
          runs: { ...get().runs, [convId]: emptyRun() },
          composer: { ...composer, text: "", attachments: [] },
        });
        await api.agentSend(convId, text, composer.attachments, (ev) => handle(convId, ev));
      } catch (e) {
        if (id) patchRun(id, (r) => ({ running: false, notices: [...r.notices, { kind: "error", text: errorText(e) }] }));
        else useApp.getState().toast(errorText(e), "error");
      }
    },

    cancel(id) {
      const target = id ?? get().activeId;
      if (target) void api.agentCancel(target);
    },

    async respond(approvalId, decision, feedback) {
      try {
        await api.agentRespond(approvalId, decision, feedback);
      } catch (e) {
        useApp.getState().toast(errorText(e), "error");
      }
    },

    async rename(id, title) {
      try {
        const meta = await api.renameConversation(id, title);
        set({ list: get().list.map((m) => (m.id === id ? meta : m)) });
        patchConv(id, () => ({ title: meta.title }));
      } catch (e) {
        useApp.getState().toast(errorText(e), "error");
      }
    },

    async remove(id) {
      try {
        await api.deleteConversation(id);
        const { [id]: _c, ...convs } = get().convs;
        const { [id]: _r, ...runs } = get().runs;
        set({ list: get().list.filter((m) => m.id !== id), convs, runs, activeId: get().activeId === id ? null : get().activeId });
      } catch (e) {
        useApp.getState().toast(errorText(e), "error");
      }
    },

    setComposer(patch) {
      set({ composer: { ...get().composer, ...patch } });
    },

    attach(path) {
      const c = get().composer;
      if (!c.attachments.includes(path)) set({ composer: { ...c, attachments: [...c.attachments, path] } });
      get().focusComposer();
    },

    insertText(text) {
      const c = get().composer;
      const sep = c.text && !c.text.endsWith("\n") ? "\n\n" : "";
      set({ composer: { ...c, text: c.text + sep + text } });
      get().focusComposer();
    },

    focusComposer() {
      const c = get().composer;
      set({ composer: { ...c, focusSeq: c.focusSeq + 1 } });
    },
  };
});

/** When the agent finishes editing a file, refresh it in the editor right away. */
function onToolStatus(ui: ToolUi) {
  if (ui.status === "done" && ui.detailKind === "diff" && ui.path) {
    void useEditor.getState().onFsChanged([ui.path]);
    void useApp.getState().refreshGit();
  }
}
