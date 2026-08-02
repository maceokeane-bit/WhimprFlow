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
  dictation_language: string;
  onboarding_complete: boolean;
  show_flow_bar: boolean;
  flow_bar_snoozed_until: number | null;
  /** Push-to-talk hotkey, e.g. `option+w` or `fn`. */
  ptt_hotkey: string;
  writing_style: WritingStyle;
  sound_on_start: boolean;
  /** Pause Spotify/Music/browser media while dictating. */
  pause_media_while_dictating: boolean;
  /** Hold Cmd+Ctrl+Option (or Fn+Ctrl) to run voice transforms. */
  command_mode_enabled: boolean;
  /** Persist dictation sessions in local history. */
  store_history: boolean;
  /** Auto-delete history older than N days; 0 = keep forever. */
  history_retention_days: number;
  /** Show rolling ASR preview while holding push-to-talk. */
  live_preview_asr: boolean;
  /** Pass caret-surrounding text into cleanup prompts. */
  context_awareness: boolean;
  /** Keep WAV audio for history sessions (pruned with retention). */
  retain_audio: boolean;
}

export type WritingStyle = "default" | "formal" | "casual" | "very_casual" | "excited";

export interface ServicesStatus {
  ollama_running: boolean;
  ollama_models: string[];
  asr_ready: boolean;
  asr_model: string | null;
  gguf_ready: boolean;
  gguf_model: string | null;
  local_worker_ready: boolean;
  asr_loaded: boolean;
}

export interface Status {
  accessibility: boolean;
  microphone: boolean;
  input_monitoring: boolean;
  has_openai_key: boolean;
  has_anthropic_key: boolean;
}

export interface ModelDownloadStatus {
  state: "missing" | "verifying" | "downloading" | "ready" | "cancelled" | "error";
  model: string;
  downloaded_bytes: number;
  total_bytes: number;
  error: string | null;
}

export interface CleanupModelStatus {
  state: "missing" | "verifying" | "downloading" | "ready" | "cancelled" | "error";
  model: string;
  downloaded_bytes: number;
  total_bytes: number;
  error: string | null;
}

export const EMPTY_MODEL_STATUS: ModelDownloadStatus = {
  state: "missing",
  model: "Parakeet v3 + Silero VAD",
  downloaded_bytes: 0,
  total_bytes: 672_427_228,
  error: null,
};

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
  ollama_model: "qwen3:8b",
  local_model: "",
  launch_at_login: false,
  dictation_language: "en",
  onboarding_complete: false,
  show_flow_bar: true,
  flow_bar_snoozed_until: null,
  ptt_hotkey: "option+w",
  writing_style: "default",
  sound_on_start: true,
  pause_media_while_dictating: true,
  command_mode_enabled: true,
  store_history: true,
  history_retention_days: 14,
  live_preview_asr: true,
  context_awareness: true,
  retain_audio: true,
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

export async function getModelDownloadStatus(): Promise<ModelDownloadStatus> {
  try {
    return await invoke<ModelDownloadStatus>("get_model_download_status");
  } catch {
    return EMPTY_MODEL_STATUS;
  }
}

export async function startModelDownload(): Promise<void> {
  await invoke<void>("start_model_download");
}

export async function cancelModelDownload(): Promise<void> {
  await invoke<void>("cancel_model_download");
}

export async function getCleanupModelStatus(): Promise<CleanupModelStatus> {
  try {
    return await invoke<CleanupModelStatus>("get_cleanup_model_status");
  } catch {
    return {
      state: "missing",
      model: "Qwen3 4B Q4_K_M",
      downloaded_bytes: 0,
      total_bytes: 2_497_280_736,
      error: null,
    };
  }
}

export async function startCleanupModelDownload(): Promise<void> {
  await invoke<void>("start_cleanup_model_download");
}

export async function cancelCleanupModelDownload(): Promise<void> {
  await invoke<void>("cancel_cleanup_model_download");
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

export interface MicrophoneTestResult {
  peak: number;
  heard_voice: boolean;
}

export async function testMicrophone(): Promise<MicrophoneTestResult> {
  try {
    return await invoke<MicrophoneTestResult>("test_microphone");
  } catch {
    return { peak: 0, heard_voice: false };
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

export async function relaunchApp(): Promise<void> {
  await invoke<void>("relaunch_app");
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
  has_audio?: boolean;
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

export async function clearHistory(): Promise<number> {
  try {
    return await invoke<number>("clear_history");
  } catch {
    return 0;
  }
}

/** Returns a blob URL for retained dictation audio, or null. */
export async function getHistoryAudioUrl(tsUnix: number): Promise<string | null> {
  try {
    const bytes = await invoke<number[]>("read_history_audio", { tsUnix });
    if (!bytes?.length) return null;
    const blob = new Blob([new Uint8Array(bytes)], { type: "audio/wav" });
    return URL.createObjectURL(blob);
  } catch {
    return null;
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

