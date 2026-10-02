import type { Provider, Settings, SettingsView, ThinkingLevel } from "./types";

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
  openai: "OpenAI",
  xai: "xAI",
  google: "Google",
  openrouter: "OpenRouter",
  local: "Local",
};

/** Settings field holding each provider's model id. */
const MODEL_FIELD = {
  anthropic: "model",
  openai: "openaiModel",
  xai: "xaiModel",
  google: "googleModel",
  openrouter: "openrouterModel",
  local: "localModel",
} as const satisfies Record<Provider, keyof Settings>;

type ProviderFields = Pick<SettingsView, "provider" | "hasApiKey" | "providerKeys" | "localModel">;

/** Can the selected provider take a message right now? */
export function providerReady(s: ProviderFields, p: Provider = s.provider): boolean {
  if (p === "local") return s.localModel.trim() !== "";
  if (p === "anthropic") return s.hasApiKey;
  return !!s.providerKeys?.[p];
}

/** The model id the selected provider will use. */
export function activeModel(s: Settings): string {
  return s[MODEL_FIELD[s.provider] ?? "model"];
}

/** Settings patch that selects `id` as the model for provider `p`. */
export function modelPatch(p: Provider, id: string): Partial<Settings> {
  return { [MODEL_FIELD[p] ?? "model"]: id };
}

export const THINKING_LEVELS: { id: ThinkingLevel; label: string; hint: string }[] = [
  { id: "off", label: "Off", hint: "No thinking" },
  { id: "low", label: "Low", hint: "Brief reasoning" },
  { id: "normal", label: "Normal", hint: "The model's default" },
  { id: "medium", label: "Medium", hint: "reasoning_effort: medium" },
  { id: "high", label: "High", hint: "reasoning_effort: high" },
  { id: "xhigh", label: "XHigh", hint: "At least ~2k thinking tokens" },
  { id: "ultra", label: "Ultra", hint: "At least ~6k thinking tokens" },
  { id: "max", label: "Max", hint: "Forces ~16k+ thinking tokens" },
];
