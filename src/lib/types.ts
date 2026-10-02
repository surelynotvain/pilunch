// Types mirrored from the Rust core (src-tauri/src). Field names are camelCase on the wire.

export type PermissionMode = "ask" | "acceptEdits" | "plan" | "bypass";
export type Effort = "low" | "medium" | "high" | "xhigh" | "max";
export type Theme = "dark" | "light" | "system";

export interface Settings {
  model: string;
  effort: Effort;
  maxTokens: number;
  baseUrl: string;
  permissionMode: PermissionMode;
  showThinking: boolean;
  customInstructions: string;
  theme: Theme;
  editorFontSize: number;
  editorWordWrap: boolean;
  editorMinimap: boolean;
  terminalShell: string;
  recentWorkspaces: string[];
  webSearch: boolean;
  onboarded: boolean;
}

export interface SettingsView extends Settings {
  hasApiKey: boolean;
  apiKeySource: "settings" | "env" | null;
  apiKeyHint: string | null;
  configDir: string;
}

export interface ModelInfo {
  id: string;
  displayName: string;
}

export interface WorkspaceInfo {
  root: string;
  name: string;
}

export interface DirEntryInfo {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
}

export interface FileContent {
  path: string;
  content: string | null;
  size: number;
  binary: boolean;
  tooLarge: boolean;
  readonly: boolean;
}

export interface FileMatch {
  path: string;
  score: number;
  indices: number[];
}

export interface GrepOptions {
  regex: boolean;
  caseInsensitive: boolean;
  wholeWord: boolean;
  include: string | null;
  maxResults: number;
}

export interface GrepMatch {
  path: string;
  line: number;
  text: string;
  ranges: [number, number][];
}

export interface GrepResult {
  matches: GrepMatch[];
  truncated: boolean;
}

/** M modified, A added, D deleted, R renamed, U untracked, C conflict */
export type GitLetter = "M" | "A" | "D" | "R" | "U" | "C";

export interface GitStatus {
  branch: string | null;
  ahead: number;
  behind: number;
  files: Record<string, GitLetter>;
}

export interface FsChanged {
  paths: string[];
  structural: boolean;
  git: boolean;
  overflow: boolean;
}

// ---- conversations ----------------------------------------------------------------

export interface ConversationMeta {
  id: string;
  title: string;
  workspace: string | null;
  createdAt: number;
  updatedAt: number;
  messageCount: number;
}

/** A Messages-API content block (kept loosely typed: we only read what we render). */
export interface ContentBlock {
  type: string;
  text?: string;
  thinking?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  tool_use_id?: string;
  content?: unknown;
  is_error?: boolean;
  [k: string]: unknown;
}

export interface StoredMessage {
  role: "user" | "assistant";
  content: ContentBlock[];
  display?: { text: string; attachments: string[] };
  model?: string;
  ts: number;
}

export type ToolStatus = "running" | "done" | "error" | "denied" | "cancelled";

export interface ToolUi {
  status: ToolStatus;
  summary: string;
  detail?: string;
  detailKind?: "diff" | "output" | "text";
  path?: string;
}

export interface UsageTotals {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
}

export interface Conversation extends ConversationMeta {
  messages: StoredMessage[];
  toolUi: Record<string, ToolUi>;
  usage: UsageTotals;
}

export type Decision = "allow" | "allowSession" | "deny";

export type AgentEvent =
  | { type: "messageAppended"; message: StoredMessage }
  | { type: "title"; title: string }
  | { type: "requestStarted"; model: string }
  | { type: "blockStart"; index: number; kind: "text" | "thinking" | "tool_use" | "server_tool_use" | "other"; toolId: string | null; toolName: string | null }
  | { type: "delta"; index: number; text: string }
  | { type: "toolInputProgress"; index: number; bytes: number }
  | { type: "toolInput"; index: number; toolId: string; name: string; input: Record<string, unknown>; summary: string }
  | { type: "toolStatus"; toolId: string; ui: ToolUi }
  | { type: "toolOutput"; toolId: string; text: string }
  | { type: "approvalRequest"; approvalId: string; toolId: string; kind: "edit" | "command" | "network"; title: string; detail: string }
  | { type: "approvalResolved"; approvalId: string; toolId: string }
  | { type: "usage"; totals: UsageTotals; contextTokens: number }
  | { type: "retrying"; attempt: number; delayMs: number; message: string }
  | { type: "reset" }
  | { type: "notice"; message: string }
  | { type: "refusal"; message: string }
  | { type: "error"; message: string }
  | { type: "done"; stopReason: string };
