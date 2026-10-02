// Typed wrappers around the Rust commands (src-tauri/src/commands.rs).
import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AgentEvent,
  Conversation,
  ConversationMeta,
  Decision,
  DirEntryInfo,
  FileContent,
  FileMatch,
  GitStatus,
  GrepOptions,
  GrepResult,
  ModelInfo,
  Settings,
  SettingsView,
  WorkspaceInfo,
} from "./types";

export const api = {
  // settings
  getSettings: () => invoke<SettingsView>("get_settings"),
  updateSettings: (patch: Partial<Settings>) => invoke<SettingsView>("update_settings", { patch }),
  setApiKey: (key: string) => invoke<SettingsView>("set_api_key", { key }),
  listModels: () => invoke<ModelInfo[]>("list_models"),

  // workspace
  openWorkspace: (path: string) => invoke<WorkspaceInfo>("open_workspace", { path }),
  closeWorkspace: () => invoke<void>("close_workspace"),
  currentWorkspace: () => invoke<WorkspaceInfo | null>("current_workspace"),

  // files
  listDir: (path: string) => invoke<DirEntryInfo[]>("list_dir", { path }),
  readFile: (path: string) => invoke<FileContent>("read_file", { path }),
  writeFile: (path: string, content: string) => invoke<void>("write_file", { path, content }),
  createFile: (path: string) => invoke<string>("create_file", { path }),
  createDir: (path: string) => invoke<string>("create_dir", { path }),
  renamePath: (from: string, to: string) => invoke<string>("rename_path", { from, to }),
  deletePath: (path: string, permanent: boolean) => invoke<void>("delete_path", { path, permanent }),

  // search
  quickOpen: (query: string, limit = 50) => invoke<FileMatch[]>("quick_open", { query, limit }),
  searchText: (query: string, options: GrepOptions) => invoke<GrepResult>("search_text", { query, options }),
  gitStatus: () => invoke<GitStatus | null>("git_status"),

  // terminal
  terminalSpawn: (cols: number, rows: number, onData: (data: Uint8Array) => void) => {
    const ch = new Channel<ArrayBuffer>();
    ch.onmessage = (buf) => onData(new Uint8Array(buf));
    return invoke<number>("terminal_spawn", { cols, rows, onData: ch });
  },
  terminalWrite: (id: number, data: string) => invoke<void>("terminal_write", { id, data }),
  terminalResize: (id: number, cols: number, rows: number) => invoke<void>("terminal_resize", { id, cols, rows }),
  terminalKill: (id: number) => invoke<void>("terminal_kill", { id }),

  // conversations
  listConversations: () => invoke<ConversationMeta[]>("list_conversations"),
  createConversation: () => invoke<ConversationMeta>("create_conversation"),
  getConversation: (id: string) => invoke<Conversation>("get_conversation", { id }),
  renameConversation: (id: string, title: string) => invoke<ConversationMeta>("rename_conversation", { id, title }),
  deleteConversation: (id: string) => invoke<void>("delete_conversation", { id }),

  // agent
  agentSend: (conversationId: string, text: string, attachments: string[], onEvent: (e: AgentEvent) => void) => {
    const ch = new Channel<AgentEvent>();
    ch.onmessage = onEvent;
    return invoke<void>("agent_send", { request: { conversationId, text, attachments }, onEvent: ch });
  },
  agentCancel: (conversationId: string) => invoke<void>("agent_cancel", { conversationId }),
  agentCancelAll: () => invoke<void>("agent_cancel_all"),
  agentRespond: (approvalId: string, decision: Decision, feedback?: string) =>
    invoke<void>("agent_respond", { approvalId, decision, feedback: feedback ?? null }),
  agentRunning: () => invoke<string[]>("agent_running"),
};

/** Error text from a rejected invoke (Rust errors serialize as strings). */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
