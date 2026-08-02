import { useCallback, useEffect, useState } from "react";
import { font } from "../tokens/values";
import { theme } from "./theme";
import { Button, Card, Dot, PageTitle, Segmented } from "./ui";
import {
  cancelCleanupModelDownload,
  clearHistory,
  getCleanupModelStatus,
  getServices,
  getHotkeyPresets,
  isLaunchAtLoginEnabled,
  pullOllamaModel,
  requestAccessibility,
  requestInputMonitoring,
  requestMicrophone,
  setApiKey,
  setLaunchAtLogin,
  startCleanupModelDownload,
  startOllama,
  type CleanupModelStatus,
  type CleanupLevel,
  type CleanupMode,
  type ServicesStatus,
  type Settings,
  type Status,
} from "./api";

const RETENTION_OPTIONS: { value: number; label: string }[] = [
  { value: 7, label: "7 days" },
  { value: 14, label: "14 days" },
  { value: 30, label: "30 days" },
  { value: 0, label: "Forever" },
];

const DEFAULT_OLLAMA_MODEL = "qwen3:8b";
const DICTATION_LANGUAGES = [
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["ja", "Japanese"],
  ["ko", "Korean"],
  ["zh", "Chinese"],
] as const;

/** True if `tag` is installed — handles `model` vs `model:latest` aliases. */
function ollamaModelInstalled(models: string[], tag: string): boolean {
  if (models.includes(tag)) return true;
  return models.some((m) => m.startsWith(`${tag}:`) || (tag.includes(":") && m.startsWith(`${tag.split(":")[0]}:`)));
}

const MODES: { value: CleanupMode; label: string; hint: string }[] = [
  { value: "raw", label: "Raw", hint: "Paste exactly what you said" },
  { value: "local", label: "Local GGUF", hint: "Offline backup — uses the .gguf file in your models folder via llama.cpp" },
  { value: "ollama", label: "Ollama", hint: "Default path — uses models you already pulled with Ollama (recommended)" },
  { value: "open_ai", label: "OpenAI", hint: "Cloud cleanup via OpenAI, DeepSeek, OpenRouter, etc." },
  { value: "anthropic", label: "Anthropic", hint: "Cloud cleanup via Claude" },
];

const LEVELS: { value: CleanupLevel; label: string; hint: string }[] = [
  { value: "none", label: "None", hint: "Transcribe exactly what you said, including mistakes." },
  { value: "light", label: "Light", hint: "Clean up filler words and grammar. (Recommended)" },
  { value: "medium", label: "Medium", hint: "Edit for clarity and conciseness." },
  { value: "high", label: "High", hint: "Rewrite for brevity and polish." },
];

function SectionTitle({ children, sub }: { children: React.ReactNode; sub?: string }) {
  return (
    <div style={{ marginBottom: 14 }}>
      <div style={{ fontSize: 15, fontWeight: 600, color: theme.textStrong }}>{children}</div>
      {sub && <div style={{ color: theme.textMuted, fontSize: 13, marginTop: 4 }}>{sub}</div>}
    </div>
  );
}

function KeyField({
  label,
  configured,
  onSave,
}: {
  label: string;
  configured: boolean;
  onSave: (key: string) => void;
}) {
  const [value, setValue] = useState("");
  const [saved, setSaved] = useState(false);
  return (
    <div style={{ marginTop: 16 }}>
      <div style={{ fontSize: 13, marginBottom: 7, display: "flex", alignItems: "center", color: theme.textBody }}>
        <Dot ok={configured} />
        {label} {configured ? "— configured" : "— not set"}
      </div>
      <div style={{ display: "flex", gap: 8 }}>
        <input
          type="password"
          value={value}
          placeholder={configured ? "Enter a new key to replace" : "Paste your API key"}
          onChange={(e) => {
            setValue(e.target.value);
            setSaved(false);
          }}
          style={{
            flex: 1,
            background: theme.cardBgSubtle,
            border: `1px solid ${theme.border}`,
            borderRadius: 10,
            padding: "9px 12px",
            color: theme.textBody,
            fontFamily: font.mono,
            fontSize: 13,
            outline: "none",
          }}
        />
        <Button
          onClick={() => {
            onSave(value);
            setValue("");
            setSaved(true);
          }}
        >
          Save
        </Button>
      </div>
      {saved && <div style={{ fontSize: 12, color: theme.accentDeep, marginTop: 6 }}>Saved to keychain ✓</div>}
    </div>
  );
}

function PermRow({
  ok,
  label,
  detail,
  onClick,
}: {
  ok: boolean;
  label: string;
  detail: string;
  onClick: () => void;
}) {
  return (
    <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
      <div style={{ display: "flex", alignItems: "center", fontSize: 13 }}>
        <Dot ok={ok} />
        <span style={{ color: theme.textBody }}>
          <b>{label}</b> <span style={{ color: theme.textMuted }}>— {detail}</span>
        </span>
      </div>
      {ok ? (
        <span style={{ color: theme.accentDeep, fontSize: 13, fontWeight: 600 }}>Granted</span>
      ) : (
        <Button variant="ghost" size="sm" onClick={onClick}>
          Grant
        </Button>
      )}
    </div>
  );
}

function ServiceRow({
  ok,
  label,
  detail,
  action,
}: {
  ok: boolean;
  label: string;
  detail: string;
  action?: React.ReactNode;
}) {
  return (
    <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
      <div style={{ display: "flex", alignItems: "flex-start", fontSize: 13, minWidth: 0 }}>
        <Dot ok={ok} />
        <span style={{ color: theme.textBody }}>
          <b>{label}</b>{" "}
          <span style={{ color: theme.textMuted }}>— {detail}</span>
        </span>
      </div>
      {action}
    </div>
  );
}

function HotkeyPicker({
  settings,
  onChange,
}: {
  settings: Settings;
  onChange: (s: Settings) => void;
}) {
  const [presets, setPresets] = useState<[string, string][]>([]);

  useEffect(() => {
    void getHotkeyPresets().then(setPresets);
  }, []);

  const inputStyle = {
    width: "100%",
    background: theme.cardBgSubtle,
    border: `1px solid ${theme.border}`,
    borderRadius: 10,
    padding: "9px 12px",
    color: theme.textBody,
    fontFamily: font.mono,
    fontSize: 13,
    outline: "none",
    boxSizing: "border-box" as const,
  };

  return (
    <div>
      <select
        value={settings.ptt_hotkey}
        onChange={(e) => onChange({ ...settings, ptt_hotkey: e.target.value })}
        style={inputStyle}
      >
        {presets.map(([value, label]) => (
          <option key={value} value={value}>
            {label}
          </option>
        ))}
        {!presets.some(([v]) => v === settings.ptt_hotkey) && (
          <option value={settings.ptt_hotkey}>{settings.ptt_hotkey} (custom)</option>
        )}
      </select>
      <div style={{ fontSize: 12, color: theme.textFaint, marginTop: 8, lineHeight: 1.5 }}>
        Default is Option + W. WhimprFlow suppresses that keystroke so it won't type ∑ or other
        Option-letter symbols. Requires Accessibility — relaunch after granting if the hotkey
        still types characters.
      </div>
    </div>
  );
}

function ServicesCard({
  settings,
  onChange,
}: {
  settings: Settings;
  onChange: (s: Settings) => void;
}) {
  const [services, setServices] = useState<ServicesStatus | null>(null);
  const [cleanupModel, setCleanupModel] = useState<CleanupModelStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [loginAtBoot, setLoginAtBoot] = useState(settings.launch_at_login);

  const refresh = useCallback(async () => {
    const [serviceStatus, modelStatus, login] = await Promise.all([
      getServices(),
      getCleanupModelStatus(),
      isLaunchAtLoginEnabled(),
    ]);
    setServices(serviceStatus);
    setCleanupModel(modelStatus);
    setLoginAtBoot(login);
  }, []);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), 4000);
    return () => window.clearInterval(id);
  }, [refresh]);

  const selectedInstalled = services
    ? ollamaModelInstalled(services.ollama_models, settings.ollama_model)
    : false;
  const cleanupPercent =
    cleanupModel && cleanupModel.total_bytes > 0
      ? Math.round((cleanupModel.downloaded_bytes / cleanupModel.total_bytes) * 100)
      : 0;

  const inputStyle = {
    width: "100%",
    background: theme.cardBgSubtle,
    border: `1px solid ${theme.border}`,
    borderRadius: 10,
    padding: "9px 12px",
    color: theme.textBody,
    fontFamily: font.mono,
    fontSize: 13,
    outline: "none",
    boxSizing: "border-box" as const,
  };

  return (
    <Card style={{ marginBottom: 16 }}>
      <SectionTitle sub="WhimprFlow prefers Parakeet and falls back to Whisper. Ollama is a separate app — start it once, then leave it running.">
        Services
      </SectionTitle>
      <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        <ServiceRow
          ok={!!services?.asr_loaded}
          label="On-device speech recognition"
          detail={
            services?.asr_model
              ? `${services.asr_model}${services.asr_loaded ? " — loaded" : " — loading…"}`
              : "model file missing"
          }
        />
        <ServiceRow
          ok={!!services?.ollama_running}
          label="Ollama (text cleanup)"
          detail={
            services?.ollama_running
              ? `${services.ollama_models.length} model(s) installed`
              : "not running — click Start"
          }
          action={
            !services?.ollama_running ? (
              <Button
                size="sm"
                disabled={busy === "ollama"}
                onClick={async () => {
                  setBusy("ollama");
                  await startOllama();
                  setTimeout(() => {
                    void refresh();
                    setBusy(null);
                  }, 2500);
                }}
              >
                Start Ollama
              </Button>
            ) : (
              <span style={{ color: theme.accentDeep, fontSize: 13, fontWeight: 600 }}>Running</span>
            )
          }
        />
        <ServiceRow
          ok={cleanupModel?.state === "ready" || !!services?.gguf_ready}
          label="GGUF backup (offline cleanup)"
          detail={
            cleanupModel?.state === "downloading"
              ? `downloading Qwen3 4B… ${cleanupPercent}%`
              : cleanupModel?.state === "verifying"
                ? "verifying downloaded model…"
                : cleanupModel?.error ??
                  services?.gguf_model ??
                  "optional 2.5 GB fully local fallback"
          }
          action={
            cleanupModel?.state === "downloading" ? (
              <Button size="sm" variant="ghost" onClick={() => void cancelCleanupModelDownload()}>
                Cancel
              </Button>
            ) : cleanupModel?.state === "ready" || services?.gguf_ready ? (
              <span style={{ color: theme.accentDeep, fontSize: 13, fontWeight: 600 }}>Ready</span>
            ) : (
              <Button
                size="sm"
                onClick={async () => {
                  await startCleanupModelDownload();
                  void refresh();
                }}
              >
                Download
              </Button>
            )
          }
        />

        {settings.cleanup_mode === "ollama" && (
          <div style={{ marginTop: 4 }}>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginBottom: 6 }}>
              Cleanup model (default {DEFAULT_OLLAMA_MODEL})
            </div>
            {services && services.ollama_models.length > 0 ? (
              <select
                value={settings.ollama_model}
                onChange={(e) => onChange({ ...settings, ollama_model: e.target.value })}
                style={inputStyle}
              >
                {!selectedInstalled && (
                  <option value={settings.ollama_model}>{settings.ollama_model} (not installed)</option>
                )}
                {services.ollama_models.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                value={settings.ollama_model}
                placeholder={DEFAULT_OLLAMA_MODEL}
                onChange={(e) => onChange({ ...settings, ollama_model: e.target.value })}
                style={inputStyle}
              />
            )}
            {!selectedInstalled && services?.ollama_running && (
              <div style={{ display: "flex", gap: 8, marginTop: 10, alignItems: "center" }}>
                <Button
                  size="sm"
                  disabled={busy === "pull"}
                  onClick={async () => {
                    setBusy("pull");
                    await pullOllamaModel(settings.ollama_model || DEFAULT_OLLAMA_MODEL);
                    setTimeout(() => {
                      void refresh();
                      setBusy(null);
                    }, 3000);
                  }}
                >
                  Pull {settings.ollama_model || DEFAULT_OLLAMA_MODEL}
                </Button>
                <span style={{ fontSize: 12, color: theme.textMuted }}>
                  Downloads in the background — check Ollama menubar for progress.
                </span>
              </div>
            )}
          </div>
        )}

        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 12,
            paddingTop: 8,
            borderTop: `1px solid ${theme.border}`,
          }}
        >
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>Launch WhimprFlow at login</div>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 4 }}>
              Keeps the dictation pill ready. Also enable “Launch Ollama at login” in the Ollama menubar app.
            </div>
          </div>
          <Segmented
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={loginAtBoot ? "on" : "off"}
            onChange={(v) => {
              const on = v === "on";
              setLoginAtBoot(on);
              onChange({ ...settings, launch_at_login: on });
              void setLaunchAtLogin(on);
            }}
          />
        </div>
      </div>
    </Card>
  );
}

export function SettingsPane({
  settings,
  onChange,
  status,
  refresh,
}: {
  settings: Settings;
  onChange: (s: Settings) => void;
  status: Status;
  refresh: () => void;
}) {
  const [clearing, setClearing] = useState(false);
  const [clearNote, setClearNote] = useState<string | null>(null);

  return (
    <div style={{ maxWidth: 720 }}>
      <PageTitle>Settings</PageTitle>

      <ServicesCard settings={settings} onChange={onChange} />

      <Card style={{ marginBottom: 16 }}>
        <SectionTitle sub="Hold this key combo anywhere to dictate. Requires Accessibility permission — no macOS System Settings shortcut needed.">
          Push-to-talk hotkey
        </SectionTitle>
        <HotkeyPicker settings={settings} onChange={onChange} />
      </Card>

      <Card style={{ marginBottom: 16 }}>
        <SectionTitle sub="Used by both Parakeet and the Whisper fallback.">
          Dictation language
        </SectionTitle>
        <select
          value={settings.dictation_language}
          onChange={(event) =>
            onChange({ ...settings, dictation_language: event.target.value })
          }
          style={{
            width: "100%",
            background: theme.cardBgSubtle,
            border: `1px solid ${theme.border}`,
            borderRadius: 10,
            padding: "10px 12px",
            color: theme.textBody,
            fontFamily: font.ui,
            fontSize: 13,
          }}
        >
          {DICTATION_LANGUAGES.map(([code, label]) => (
            <option key={code} value={code}>
              {label}
            </option>
          ))}
        </select>
      </Card>

      <Card style={{ marginBottom: 16 }}>
        <SectionTitle sub="Where your dictation is cleaned up before it's typed.">Cleanup Engine</SectionTitle>
        <Segmented
          options={MODES.map((m) => ({ value: m.value, label: m.label }))}
          value={settings.cleanup_mode}
          onChange={(v) => onChange({ ...settings, cleanup_mode: v })}
        />
        <div style={{ color: theme.textMuted, fontSize: 12.5, marginTop: 10 }}>
          {MODES.find((m) => m.value === settings.cleanup_mode)?.hint}
        </div>

        {settings.cleanup_mode === "ollama" && (
          <div style={{ marginTop: 14 }}>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginBottom: 6 }}>Ollama base URL</div>
            <input
              type="text"
              value={settings.ollama_base_url}
              placeholder="http://localhost:11434/v1"
              onChange={(e) => onChange({ ...settings, ollama_base_url: e.target.value })}
              style={{
                width: "100%",
                background: theme.cardBgSubtle,
                border: `1px solid ${theme.border}`,
                borderRadius: 10,
                padding: "9px 12px",
                color: theme.textBody,
                fontFamily: font.mono,
                fontSize: 13,
                outline: "none",
                boxSizing: "border-box",
              }}
            />
            <div style={{ fontSize: 12, color: theme.textFaint, marginTop: 8 }}>
              Avoid reasoning/coder models (deepseek-r1, qwen3-coder) for dictation cleanup.
              qwen3:8b works well with Ollama.
            </div>
          </div>
        )}

        {settings.cleanup_mode === "local" && (
          <div style={{ marginTop: 14 }}>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginBottom: 6 }}>
              GGUF filename (optional — blank auto-picks the best .gguf in the models folder)
            </div>
            <input
              type="text"
              value={settings.local_model}
              placeholder="qwen2.5-1.5b-instruct-q4_k_m.gguf"
              onChange={(e) => onChange({ ...settings, local_model: e.target.value })}
              style={{
                width: "100%",
                background: theme.cardBgSubtle,
                border: `1px solid ${theme.border}`,
                borderRadius: 10,
                padding: "9px 12px",
                color: theme.textBody,
                fontFamily: font.mono,
                fontSize: 13,
                outline: "none",
                boxSizing: "border-box",
              }}
            />
          </div>
        )}

        {settings.cleanup_mode === "open_ai" && (
        <>
        <KeyField
          label="OpenAI API key"
          configured={status.has_openai_key}
          onSave={(k) => {
            setApiKey("openai", k);
            setTimeout(refresh, 400);
          }}
        />
        <div style={{ marginTop: 12, display: "flex", gap: 8 }}>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginBottom: 6 }}>
              Base URL (blank = OpenAI; DeepSeek: https://api.deepseek.com/v1)
            </div>
            <input
              type="text"
              value={settings.openai_base_url}
              placeholder="https://api.deepseek.com/v1"
              onChange={(e) => onChange({ ...settings, openai_base_url: e.target.value })}
              style={{
                width: "100%",
                background: theme.cardBgSubtle,
                border: `1px solid ${theme.border}`,
                borderRadius: 10,
                padding: "9px 12px",
                color: theme.textBody,
                fontFamily: font.mono,
                fontSize: 13,
                outline: "none",
                boxSizing: "border-box",
              }}
            />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginBottom: 6 }}>
              Model (OpenAI, DeepSeek, OpenRouter slug, etc.)
            </div>
            <input
              type="text"
              value={settings.openai_model}
              placeholder="deepseek-chat"
              onChange={(e) => onChange({ ...settings, openai_model: e.target.value })}
              style={{
                width: "100%",
                background: theme.cardBgSubtle,
                border: `1px solid ${theme.border}`,
                borderRadius: 10,
                padding: "9px 12px",
                color: theme.textBody,
                fontFamily: font.mono,
                fontSize: 13,
                outline: "none",
                boxSizing: "border-box",
              }}
            />
          </div>
        </div>
        </>
        )}

        {settings.cleanup_mode === "anthropic" && (
        <KeyField
          label="Anthropic API key"
          configured={status.has_anthropic_key}
          onSave={(k) => {
            setApiKey("anthropic", k);
            setTimeout(refresh, 400);
          }}
        />
        )}
      </Card>

      <Card style={{ marginBottom: 16 }}>
        <SectionTitle>Auto Cleanup</SectionTitle>
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {LEVELS.map((l) => {
            const selected = settings.cleanup_level === l.value;
            return (
              <button
                key={l.value}
                onClick={() => onChange({ ...settings, cleanup_level: l.value })}
                style={{
                  textAlign: "left",
                  cursor: "pointer",
                  borderRadius: 12,
                  padding: "12px 14px",
                  fontFamily: font.ui,
                  background: selected ? theme.accentSoft : theme.cardBgSubtle,
                  border: `1px solid ${selected ? theme.accentSoftBorder : theme.border}`,
                  color: theme.textBody,
                }}
              >
                <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>{l.label}</div>
                <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2 }}>{l.hint}</div>
              </button>
            );
          })}
        </div>
      </Card>

      <Card style={{ marginBottom: 16 }}>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>
              Show Flow Bar
            </div>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2 }}>
              Shows recording and processing status. Dictation continues while it is hidden.
            </div>
          </div>
          <Segmented
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={settings.show_flow_bar ? "on" : "off"}
            onChange={(v) =>
              onChange({
                ...settings,
                show_flow_bar: v === "on",
                flow_bar_snoozed_until: v === "on" ? null : settings.flow_bar_snoozed_until,
              })
            }
          />
        </div>
      </Card>

      <Card style={{ marginBottom: 16 }}>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>
              Dictation sounds
            </div>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2 }}>
              Ping on start, tink on release, pop when text is inserted.
            </div>
          </div>
          <Segmented
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={settings.sound_on_start ? "on" : "off"}
            onChange={(v) => onChange({ ...settings, sound_on_start: v === "on" })}
          />
        </div>
      </Card>

      <Card style={{ marginBottom: 16 }}>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>
              Pause media while dictating
            </div>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2 }}>
              Pauses Spotify, Music, and browser video when you start recording; resumes when you stop.
            </div>
          </div>
          <Segmented
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={settings.pause_media_while_dictating ? "on" : "off"}
            onChange={(v) => onChange({ ...settings, pause_media_while_dictating: v === "on" })}
          />
        </div>
      </Card>

      <Card style={{ marginBottom: 16 }}>
        <SectionTitle sub="Grant these to WhimprFlow, then quit and reopen the app if a dot stays grey.">
          Permissions
        </SectionTitle>
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <PermRow
            ok={status.accessibility}
            label="Accessibility"
            detail={
              status.accessibility
                ? "granted — hotkey works everywhere + types your words"
                : "required: global hotkey + typing into other apps"
            }
            onClick={() => {
              requestAccessibility();
              setTimeout(refresh, 800);
            }}
          />
          <PermRow
            ok={status.microphone}
            label="Microphone"
            detail={status.microphone ? "granted" : "hears what you say"}
            onClick={() => {
              requestMicrophone();
              setTimeout(refresh, 1000);
            }}
          />
          <PermRow
            ok={status.input_monitoring}
            label="Input Monitoring"
            detail="optional — extra reliability for key detection"
            onClick={() => {
              requestInputMonitoring();
              setTimeout(refresh, 1000);
            }}
          />
        </div>
      </Card>

      <Card style={{ marginBottom: 16 }}>
        <SectionTitle sub="Speech-to-text always runs on this Mac. Cleanup stays local unless you pick a cloud engine.">
          Data & Privacy
        </SectionTitle>
        <div style={{ fontSize: 13, color: theme.textMuted, lineHeight: 1.55, marginBottom: 16 }}>
          Transcription always runs on this Mac. Optional retained WAVs stay in local app support and
          are never uploaded. Cleanup uses Local GGUF or Ollama on-device by default; OpenAI / Anthropic
          send only transcript text when you choose those engines. Keys live in your keychain.
        </div>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 12,
            marginBottom: 14,
          }}
        >
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>
              Store dictation history
            </div>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2 }}>
              Saves sessions locally for Home and Insights. Off means new dictations aren’t kept.
            </div>
          </div>
          <Segmented
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={settings.store_history ? "on" : "off"}
            onChange={(v) => onChange({ ...settings, store_history: v === "on" })}
          />
        </div>
        {settings.store_history && (
          <>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: 12,
                marginBottom: 14,
              }}
            >
              <div>
                <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>
                  Keep dictation audio
                </div>
                <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2 }}>
                  Saves a local WAV per session so you can replay mistakes. Pruned with history
                  retention.
                </div>
              </div>
              <Segmented
                options={[
                  { value: "on", label: "On" },
                  { value: "off", label: "Off" },
                ]}
                value={settings.retain_audio ? "on" : "off"}
                onChange={(v) => onChange({ ...settings, retain_audio: v === "on" })}
              />
            </div>
            <div style={{ marginBottom: 14 }}>
              <div style={{ fontSize: 13, color: theme.textMuted, marginBottom: 8 }}>
                Auto-delete history older than
              </div>
              <Segmented
                options={RETENTION_OPTIONS.map((o) => ({
                  value: String(o.value),
                  label: o.label,
                }))}
                value={String(settings.history_retention_days)}
                onChange={(v) =>
                  onChange({ ...settings, history_retention_days: Number(v) })
                }
              />
            </div>
          </>
        )}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 12,
            marginBottom: 14,
          }}
        >
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>
              Context awareness
            </div>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2 }}>
              Reads ~200 characters around the caret so cleanup can match tone and nearby names.
            </div>
          </div>
          <Segmented
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={settings.context_awareness ? "on" : "off"}
            onChange={(v) => onChange({ ...settings, context_awareness: v === "on" })}
          />
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
          <Button
            variant="ghost"
            disabled={clearing}
            onClick={async () => {
              if (!window.confirm("Delete all local dictation history? This cannot be undone.")) {
                return;
              }
              setClearing(true);
              const removed = await clearHistory();
              setClearing(false);
              setClearNote(
                removed > 0
                  ? `Cleared ${removed} session${removed === 1 ? "" : "s"}.`
                  : "History was already empty.",
              );
            }}
          >
            {clearing ? "Clearing…" : "Clear all history"}
          </Button>
          {clearNote && (
            <span style={{ fontSize: 12.5, color: theme.textMuted }}>{clearNote}</span>
          )}
        </div>
      </Card>

      <Card>
        <SectionTitle sub="Features that change how WhimprFlow behaves — easy to turn off.">
          Experimental
        </SectionTitle>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 12,
            marginBottom: 16,
          }}
        >
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>
              Live transcript preview
            </div>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2, lineHeight: 1.5 }}>
              Shows a rolling ASR preview in the Flow Bar while you hold push-to-talk. Final text
              still comes from the full pass on release.
            </div>
          </div>
          <Segmented
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={settings.live_preview_asr ? "on" : "off"}
            onChange={(v) => onChange({ ...settings, live_preview_asr: v === "on" })}
          />
        </div>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
          <div>
            <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>
              Command Mode
            </div>
            <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2, lineHeight: 1.5 }}>
              Hold <b>Command + Control + Option</b> (or <b>Fn + Control</b>) after selecting text to
              speak a transform, or generate at the cursor with no selection.
            </div>
          </div>
          <Segmented
            options={[
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            value={settings.command_mode_enabled ? "on" : "off"}
            onChange={(v) => onChange({ ...settings, command_mode_enabled: v === "on" })}
          />
        </div>
      </Card>
    </div>
  );
}
