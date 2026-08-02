// Typed wrappers over the Tauri command surface. In a plain browser (vite dev
// without the shell) the invoke import fails and we fall back to defaults so the
// Hub still renders for iteration.

export type CleanupMode = "raw" | "local" | "ollama" | "open_ai" | "anthropic";
export type CleanupLevel = "none" | "light" | "medium" | "high";

export interface Settings {
  cleanup_mode: CleanupMode;
  cleanup_level: CleanupLevel;
  openai_model: string;
  // API root for "OpenAI" mode — leave blank for OpenAI itself, or point at
  // an OpenAI-compatible endpoint like OpenRouter (https://openrouter.ai/api/v1).
  openai_base_url: string;
  anthropic_model: string;
  ollama_base_url: string;
  ollama_model: string;
  /** GGUF filename in the models folder; blank = auto-detect. */
  local_model: string;
  launch_at_login: boolean;
  /** Push-to-talk hotkey, e.g. `option+w` or `fn`. */
  ptt_hotkey: string;
  writing_style: WritingStyle;
  sound_on_start: boolean;
}

export type WritingStyle = "default" | "formal" | "casual" | "very_casual" | "excited";

export interface ServicesStatus {
  ollama_running: boolean;
  ollama_models: string[];
  whisper_ready: boolean;
  whisper_model: string | null;
  gguf_ready: boolean;
  gguf_model: string | null;
  local_worker_ready: boolean;
  whisper_loaded: boolean;
}

export interface Status {
  accessibility: boolean;
  microphone: boolean;
  input_monitoring: boolean;
  has_openai_key: boolean;
  has_anthropic_key: boolean;
}

export interface StatsSummary {
  total_words: number;
  total_sessions: number;
  total_speaking_secs: number;
  avg_wpm: number;
  best_wpm: number;
  words_today: number;
  wpm_today: number;
  day_streak: number;
  time_saved_secs: number;
  last7_words: number[];
}

export const EMPTY_STATS: StatsSummary = {
  total_words: 0,
  total_sessions: 0,
  total_speaking_secs: 0,
  avg_wpm: 0,
  best_wpm: 0,
  words_today: 0,
  wpm_today: 0,
  day_streak: 0,
  time_saved_secs: 0,
  last7_words: [0, 0, 0, 0, 0, 0, 0],
};

export const DEFAULT_SETTINGS: Settings = {
  cleanup_mode: "ollama",
  cleanup_level: "light",
  openai_model: "gpt-4o-mini",
  openai_base_url: "",
  anthropic_model: "claude-haiku-4-5",
  ollama_base_url: "http://localhost:11434/v1",
  ollama_model: "qwen3:1.7b",
  local_model: "",
  launch_at_login: false,
  ptt_hotkey: "option+w",
  writing_style: "default",
  sound_on_start: true,
};

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

export async function getSettings(): Promise<Settings> {
  try {
    return await invoke<Settings>("get_settings");
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export async function setSettings(settings: Settings): Promise<void> {
  try {
    await invoke<void>("set_settings", { settings });
  } catch {
    /* browser preview — no-op */
  }
}

export async function getStatus(): Promise<Status> {
  try {
    return await invoke<Status>("get_status");
  } catch {
    return {
      accessibility: false,
      microphone: false,
      input_monitoring: false,
      has_openai_key: false,
      has_anthropic_key: false,
    };
  }
}

export async function getStats(): Promise<StatsSummary> {
  try {
    const tz = new Date().getTimezoneOffset(); // minutes to add to local -> UTC
    return await invoke<StatsSummary>("get_stats", { tzOffsetMinutes: tz });
  } catch {
    return EMPTY_STATS;
  }
}

export async function requestMicrophone(): Promise<void> {
  try {
    await invoke<void>("request_microphone");
  } catch {
    /* browser preview */
  }
}

export async function requestAccessibility(): Promise<void> {
  try {
    await invoke<void>("request_accessibility");
  } catch {
    /* browser preview */
  }
}

export async function requestInputMonitoring(): Promise<void> {
  try {
    await invoke<void>("request_input_monitoring");
  } catch {
    /* browser preview */
  }
}

export async function setApiKey(provider: "openai" | "anthropic", key: string): Promise<void> {
  try {
    await invoke<void>("set_api_key", { provider, key });
  } catch {
    /* browser preview */
  }
}

// ── History ────────────────────────────────────────────────────────────────
export interface HistoryItem {
  ts_unix: number;
  text: string;
  raw_text?: string;
  app: string | null;
  words: number;
}

export async function getHistory(): Promise<HistoryItem[]> {
  try {
    return await invoke<HistoryItem[]>("get_history");
  } catch {
    return [];
  }
}

// ── Dictionary ───────────────────────────────────────────────────────────────
export interface DictEntry {
  correct: string;
  mishears: string[];
  auto: boolean;
}

export async function getDictionary(): Promise<DictEntry[]> {
  try {
    return await invoke<DictEntry[]>("get_dictionary");
  } catch {
    return [];
  }
}

export async function addDictionaryEntry(correct: string, mishears: string[]): Promise<void> {
  try {
    await invoke<void>("add_dictionary_entry", { correct, mishears });
  } catch {
    /* browser preview — no-op */
  }
}

export async function removeDictionaryEntry(correct: string): Promise<void> {
  try {
    await invoke<void>("remove_dictionary_entry", { correct });
  } catch {
    /* browser preview — no-op */
  }
}

export async function getServices(): Promise<ServicesStatus | null> {
  try {
    return await invoke<ServicesStatus>("get_services");
  } catch {
    return null;
  }
}

export async function startOllama(): Promise<void> {
  await invoke<void>("start_ollama");
}

export async function pullOllamaModel(model: string): Promise<void> {
  await invoke<void>("pull_ollama_model", { model });
}

export async function setLaunchAtLogin(enabled: boolean): Promise<void> {
  try {
    await invoke<void>("set_launch_at_login", { enabled });
  } catch {
    /* browser preview */
  }
}

export async function isLaunchAtLoginEnabled(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_launch_at_login_enabled");
  } catch {
    return false;
  }
}

export async function deleteHistory(tsUnix: number): Promise<boolean> {
  try {
    return await invoke<boolean>("delete_history", { tsUnix });
  } catch {
    return false;
  }
}

export interface InsightReport {
  generated_at: number;
  sessions_analyzed: number;
  reading_grade: string;
  complexity: string;
  domain_depth: string;
  summary: string;
  topics: string[];
  vocabulary_note: string;
  error?: string;
}

export async function analyzeInsights(forceRefresh = false): Promise<InsightReport | null> {
  try {
    return await invoke<InsightReport>("analyze_insights", { forceRefresh });
  } catch {
    return null;
  }
}

export async function getHotkeyPresets(): Promise<[string, string][]> {
  try {
    return await invoke<[string, string][]>("get_hotkey_presets");
  } catch {
    return [
      ["option+w", "Option + W (recommended)"],
      ["fn", "Fn / Globe key"],
    ];
  }
}

export interface LanguageStats {
  sessions_analyzed: number;
  avg_words_per_session: number;
  avg_wpm: number;
  cleanup_edit_rate: number;
  filler_per_100_words: number;
  avg_sentence_length: number;
  unique_word_ratio: number;
  top_apps: [string, number][];
}

export async function getLanguageStats(): Promise<LanguageStats | null> {
  try {
    return await invoke<LanguageStats>("get_language_stats");
  } catch {
    return null;
  }
}

export interface Snippet {
  trigger: string;
  expansion: string;
}

export async function getSnippets(): Promise<Snippet[]> {
  try {
    return await invoke<Snippet[]>("get_snippets");
  } catch {
    return [];
  }
}

export async function addSnippet(trigger: string, expansion: string): Promise<void> {
  await invoke<void>("add_snippet", { trigger, expansion });
}

export async function removeSnippet(trigger: string): Promise<boolean> {
  try {
    return await invoke<boolean>("remove_snippet", { trigger });
  } catch {
    return false;
  }
}

export interface TransformPreset {
  id: string;
  name: string;
  instruction: string;
}

export async function getTransforms(): Promise<TransformPreset[]> {
  try {
    return await invoke<TransformPreset[]>("get_transforms");
  } catch {
    return [];
  }
}

export async function saveTransform(preset: TransformPreset): Promise<void> {
  await invoke<void>("save_transform", { preset });
}

export async function removeTransform(id: string): Promise<boolean> {
  try {
    return await invoke<boolean>("remove_transform", { id });
  } catch {
    return false;
  }
}

export async function runTransform(presetId: string, instruction?: string): Promise<string> {
  return await invoke<string>("run_transform", { presetId, instruction: instruction ?? null });
}

