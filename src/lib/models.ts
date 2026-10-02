import type { Provider, Settings, SettingsView } from "./types";

export const MODELS = [
  { id: "claude-opus-5-5", label: "Opus 5.5" },
  { id: "claude-sonnet-5-5", label: "Sonnet 5.5" },
  { id: "claude-fable-5-1", label: "Fable 5.1" },
  { id: "claude-haiku-4-5", label: "Haiku 4.5" },
];

export const EFFORTS = [
  { id: "low", label: "Low" },
  { id: "medium", label: "Medium" },
  { id: "high", label: "High" },
  { id: "xhigh", label: "Extra high" },
  { id: "max", label: "Max" },
] as const;

export function modelLabel(id: string): string {
  return MODELS.find((m) => m.id === id)?.label ?? id.replace(/^claude-/, "");
}

export const PROVIDER_LABEL: Record<Provider, string> = {
  anthropic: "Anthropic",
  openrouter: "OpenRouter",
  local: "Local",
};

type ProviderFields = Pick<SettingsView, "provider" | "hasApiKey" | "hasOpenrouterKey" | "localModel">;

/** Can the selected provider take a message right now? */
export function providerReady(s: ProviderFields, p: Provider = s.provider): boolean {
  if (p === "openrouter") return s.hasOpenrouterKey;
  if (p === "local") return s.localModel.trim() !== "";
  return s.hasApiKey;
}

/** The model id the selected provider will use. */
export function activeModel(s: Pick<SettingsView, "provider" | "model" | "openrouterModel" | "localModel">): string {
  if (s.provider === "openrouter") return s.openrouterModel;
  if (s.provider === "local") return s.localModel;
  return s.model;
}

/** Settings patch that selects `id` as the model for provider `p`. */
export function modelPatch(p: Provider, id: string): Partial<Settings> {
  if (p === "openrouter") return { openrouterModel: id };
  if (p === "local") return { localModel: id };
  return { model: id };
}
