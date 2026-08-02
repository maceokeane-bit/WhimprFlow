import { useEffect, useState } from "react";
import { font, palette } from "../tokens/values";
import { theme } from "./theme";
import {
  cancelModelDownload,
  requestAccessibility,
  requestMicrophone,
  requestInputMonitoring,
  relaunchApp,
  startModelDownload,
  testMicrophone,
  type MicrophoneTestResult,
  type ModelDownloadStatus,
  type Settings,
  type Status,
} from "./api";

const DICTATION_LANGUAGES = [
  { code: "en", label: "English" },
  { code: "es", label: "Spanish" },
  { code: "fr", label: "French" },
  { code: "de", label: "German" },
  { code: "it", label: "Italian" },
  { code: "pt", label: "Portuguese" },
  { code: "ja", label: "Japanese" },
  { code: "ko", label: "Korean" },
  { code: "zh", label: "Chinese" },
] as const;

// A blocking permission gate: the app can't be used until Accessibility and
// Microphone are granted. The three permissions are presented in order (each
// unlocks the next), and their state flips live as macOS applies them.

function Step({
  n,
  title,
  detail,
  done,
  active,
  locked,
  required,
  onGrant,
}: {
  n: number;
  title: string;
  detail: string;
  done: boolean;
  active: boolean;
  locked: boolean;
  required: boolean;
  onGrant: () => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 16,
        padding: "16px 18px",
        borderRadius: 14,
        marginBottom: 12,
        background: active ? theme.accentSoft : theme.cardBg,
        border: `1px solid ${active ? theme.accentSoftBorder : theme.border}`,
        boxShadow: theme.shadowSoft,
        opacity: locked ? 0.5 : 1,
      }}
    >
      <div
        style={{
          flex: "0 0 auto",
          width: 30,
          height: 30,
          borderRadius: 9999,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontWeight: 700,
          fontSize: 14,
          color: done ? "#fff" : theme.textMuted,
          background: done ? theme.accentDeep : theme.track,
        }}
      >
        {done ? "✓" : n}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 15, fontWeight: 600, color: theme.textStrong }}>
          {title}{" "}
          <span style={{ fontSize: 12, color: theme.textFaint, fontWeight: 400 }}>
            {required ? "· required" : "· optional"}
          </span>
        </div>
        <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 2 }}>{detail}</div>
      </div>
      {done ? (
        <span style={{ color: theme.accentDeep, fontSize: 13, fontWeight: 600 }}>Granted</span>
      ) : (
        <button
          onClick={onGrant}
          disabled={locked}
          style={{
            cursor: locked ? "default" : "pointer",
            border: "none",
            borderRadius: 10,
            padding: "9px 16px",
            fontSize: 13,
            fontWeight: 600,
            fontFamily: font.ui,
            color: "#fff",
            background: locked ? theme.textFaint : palette.slate900,
            whiteSpace: "nowrap",
          }}
        >
          Grant
        </button>
      )}
    </div>
  );
}

function ModelStep({
  n,
  model,
  unlocked,
}: {
  n: number;
  model: ModelDownloadStatus;
  unlocked: boolean;
}) {
  const ready = model.state === "ready";
  const busy = model.state === "downloading" || model.state === "verifying";
  const percent =
    model.total_bytes > 0 ? Math.min(100, (model.downloaded_bytes / model.total_bytes) * 100) : 0;
  const downloadedGb = (model.downloaded_bytes / 1_000_000_000).toFixed(1);
  const totalGb = (model.total_bytes / 1_000_000_000).toFixed(1);

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 16,
        padding: "16px 18px",
        borderRadius: 14,
        marginBottom: 12,
        background: unlocked && !ready ? theme.accentSoft : theme.cardBg,
        border: `1px solid ${unlocked && !ready ? theme.accentSoftBorder : theme.border}`,
        boxShadow: theme.shadowSoft,
        opacity: unlocked ? 1 : 0.5,
      }}
    >
      <div
        style={{
          flex: "0 0 auto",
          width: 30,
          height: 30,
          borderRadius: 9999,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontWeight: 700,
          fontSize: 14,
          color: ready ? "#fff" : theme.textMuted,
          background: ready ? theme.accentDeep : theme.track,
        }}
      >
        {ready ? "✓" : n}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 15, fontWeight: 600, color: theme.textStrong }}>
          Speech model <span style={{ fontSize: 12, color: theme.textFaint, fontWeight: 400 }}>· required</span>
        </div>
        <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 2 }}>
          {ready
            ? "Verified and ready for private, on-device transcription."
            : model.state === "verifying"
              ? "Verifying the existing model…"
              : model.state === "downloading"
                ? `Downloading ${downloadedGb} of ${totalGb} GB…`
                : model.error ?? "Download Parakeet and voice detection for on-device transcription (0.7 GB)."}
        </div>
        {busy && (
          <div
            style={{
              height: 5,
              borderRadius: 999,
              background: theme.track,
              overflow: "hidden",
              marginTop: 9,
            }}
          >
            <div
              style={{
                height: "100%",
                width: `${model.state === "verifying" ? 100 : percent}%`,
                background: theme.accentDeep,
                transition: "width 180ms ease",
                opacity: model.state === "verifying" ? 0.55 : 1,
              }}
            />
          </div>
        )}
      </div>
      {!ready &&
        (model.state === "downloading" ? (
          <button
            onClick={() => void cancelModelDownload()}
            style={{
              border: `1px solid ${theme.border}`,
              borderRadius: 10,
              padding: "9px 14px",
              fontFamily: font.ui,
              background: theme.cardBg,
              color: theme.textBody,
              cursor: "pointer",
            }}
          >
            Cancel
          </button>
        ) : (
          <button
            onClick={() => void startModelDownload()}
            disabled={!unlocked || model.state === "verifying"}
            style={{
              cursor: unlocked && model.state !== "verifying" ? "pointer" : "default",
              border: "none",
              borderRadius: 10,
              padding: "9px 16px",
              fontSize: 13,
              fontWeight: 600,
              fontFamily: font.ui,
              color: "#fff",
              background:
                unlocked && model.state !== "verifying" ? palette.slate900 : theme.textFaint,
              whiteSpace: "nowrap",
            }}
          >
            {model.state === "error" || model.state === "cancelled" ? "Retry" : "Download"}
          </button>
        ))}
    </div>
  );
}

function MicrophoneTestStep({
  result,
  testing,
  unlocked,
  onTest,
}: {
  result: MicrophoneTestResult | null;
  testing: boolean;
  unlocked: boolean;
  onTest: () => void;
}) {
  const passed = result?.heard_voice === true;
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 16,
        padding: "16px 18px",
        borderRadius: 14,
        marginBottom: 12,
        background: unlocked && !passed ? theme.accentSoft : theme.cardBg,
        border: `1px solid ${unlocked && !passed ? theme.accentSoftBorder : theme.border}`,
        boxShadow: theme.shadowSoft,
        opacity: unlocked ? 1 : 0.5,
      }}
    >
      <div
        style={{
          width: 30,
          height: 30,
          borderRadius: 999,
          display: "grid",
          placeItems: "center",
          fontWeight: 700,
          color: passed ? "#fff" : theme.textMuted,
          background: passed ? theme.accentDeep : theme.track,
        }}
      >
        {passed ? "✓" : 4}
      </div>
      <div style={{ flex: 1 }}>
        <div style={{ fontSize: 15, fontWeight: 600, color: theme.textStrong }}>
          Test your microphone{" "}
          <span style={{ fontSize: 12, color: theme.textFaint, fontWeight: 400 }}>· recommended</span>
        </div>
        <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 2 }}>
          {testing
            ? "Speak normally for a moment…"
            : passed
              ? "Your voice came through clearly."
              : result
                ? "We did not hear enough audio. Check your selected microphone and try again."
                : "Confirm WhimprFlow can hear your voice before your first dictation."}
        </div>
      </div>
      <button
        onClick={onTest}
        disabled={!unlocked || testing}
        style={{
          border: "none",
          borderRadius: 10,
          padding: "9px 14px",
          fontFamily: font.ui,
          fontWeight: 600,
          background: unlocked ? palette.slate900 : theme.textFaint,
          color: "#fff",
          cursor: unlocked && !testing ? "pointer" : "default",
        }}
      >
        {testing ? "Listening…" : result ? "Test again" : "Start test"}
      </button>
    </div>
  );
}

function LanguageStep({
  value,
  unlocked,
  onChange,
}: {
  value: string;
  unlocked: boolean;
  onChange: (language: string) => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 16,
        padding: "16px 18px",
        borderRadius: 14,
        marginBottom: 12,
        background: theme.cardBg,
        border: `1px solid ${theme.border}`,
        boxShadow: theme.shadowSoft,
        opacity: unlocked ? 1 : 0.5,
      }}
    >
      <div
        style={{
          width: 30,
          height: 30,
          borderRadius: 999,
          display: "grid",
          placeItems: "center",
          fontWeight: 700,
          color: value ? "#fff" : theme.textMuted,
          background: value ? theme.accentDeep : theme.track,
        }}
      >
        {value ? "✓" : 5}
      </div>
      <div style={{ flex: 1 }}>
        <div style={{ fontSize: 15, fontWeight: 600, color: theme.textStrong }}>
          Dictation language <span style={{ fontSize: 12, color: theme.textFaint }}>· required</span>
        </div>
        <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 2 }}>
          Choose the language WhimprFlow should recognize.
        </div>
      </div>
      <select
        value={value}
        disabled={!unlocked}
        onChange={(event) => onChange(event.target.value)}
        style={{
          border: `1px solid ${theme.border}`,
          borderRadius: 10,
          padding: "9px 12px",
          fontFamily: font.ui,
          background: theme.cardBg,
          color: theme.textBody,
        }}
      >
        {DICTATION_LANGUAGES.map((language) => (
          <option key={language.code} value={language.code}>
            {language.label}
          </option>
        ))}
      </select>
    </div>
  );
}

function TutorialStep({ unlocked }: { unlocked: boolean }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 16,
        padding: "16px 18px",
        borderRadius: 14,
        marginBottom: 12,
        background: unlocked ? theme.accentSoft : theme.cardBg,
        border: `1px solid ${unlocked ? theme.accentSoftBorder : theme.border}`,
        boxShadow: theme.shadowSoft,
        opacity: unlocked ? 1 : 0.5,
      }}
    >
      <div
        style={{
          width: 30,
          height: 30,
          borderRadius: 999,
          display: "grid",
          placeItems: "center",
          fontWeight: 700,
          color: unlocked ? "#fff" : theme.textMuted,
          background: unlocked ? theme.accentDeep : theme.track,
        }}
      >
        {unlocked ? "✓" : 7}
      </div>
      <div>
        <div style={{ fontSize: 15, fontWeight: 600, color: theme.textStrong }}>Your first dictation</div>
        <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 3, lineHeight: 1.45 }}>
          Click a text field, hold <b>Fn</b>, speak naturally, then release. The Flow Bar shows
          listening, transcription, formatting, and insertion progress.
        </div>
      </div>
    </div>
  );
}

export function Onboarding({
  status,
  model,
  settings,
  onSettingsChange,
  refresh,
  onEnter,
}: {
  status: Status;
  model: ModelDownloadStatus;
  settings: Settings;
  onSettingsChange: (settings: Settings) => void;
  refresh: () => void;
  onEnter: () => void;
}) {
  const [inputRequested, setInputRequested] = useState(false);
  const [testingMicrophone, setTestingMicrophone] = useState(false);
  const [microphoneResult, setMicrophoneResult] = useState<MicrophoneTestResult | null>(null);
  // Poll live so the state flips the moment macOS applies each grant.
  useEffect(() => {
    const id = setInterval(refresh, 1200);
    return () => clearInterval(id);
  }, [refresh]);

  const acc = status.accessibility;
  const mic = status.microphone;
  const inp = status.input_monitoring;
  const canEnter =
    acc && mic && model.state === "ready" && settings.dictation_language.trim().length > 0;

  return (
    <div
      style={{
        minHeight: "100vh",
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "center",
        background: theme.pageBg,
        color: theme.textBody,
        fontFamily: font.ui,
        padding: "32px 24px",
        overflowY: "auto",
      }}
    >
      <div style={{ width: 560, maxWidth: "100%" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 6 }}>
          <div style={{ fontFamily: font.serif, fontSize: 30, fontWeight: 600, color: theme.textStrong }}>
            Set up WhimprFlow
          </div>
          <span
            style={{
              fontSize: 10,
              fontWeight: 700,
              letterSpacing: 0.4,
              textTransform: "uppercase",
              color: theme.accentDeep,
              background: theme.accentSoft,
              border: `1px solid ${theme.accentSoftBorder}`,
              borderRadius: 999,
              padding: "2px 7px",
            }}
          >
            Local
          </span>
        </div>
        <p style={{ color: theme.textMuted, lineHeight: 1.5, margin: "0 0 24px" }}>
          Grant the required permissions, verify your microphone, and choose your dictation
          language.
        </p>

        <Step
          n={1}
          title="Accessibility"
          detail="Detects the Fn key in every app and types your words. This is the one that makes the Fn key work everywhere."
          done={acc}
          active={!acc}
          locked={false}
          required
          onGrant={() => requestAccessibility()}
        />
        <Step
          n={2}
          title="Microphone"
          detail="Lets WhimprFlow hear what you say."
          done={mic}
          active={acc && !mic}
          locked={!acc}
          required
          onGrant={() => requestMicrophone()}
        />
        <Step
          n={3}
          title="Input Monitoring"
          detail="Extra reliability for key detection. Optional — you can enter without it."
          done={inp}
          active={acc && mic && !inp}
          locked={!(acc && mic)}
          required={false}
          onGrant={() => {
            setInputRequested(true);
            requestInputMonitoring();
          }}
        />
        {inputRequested && !inp && (
          <div
            style={{
              margin: "-4px 0 12px 46px",
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: 12,
              color: theme.textMuted,
              fontSize: 12.5,
            }}
          >
            <span>Input Monitoring may only take effect after restarting WhimprFlow.</span>
            <button
              onClick={() => void relaunchApp()}
              style={{
                border: `1px solid ${theme.border}`,
                borderRadius: 9,
                padding: "7px 11px",
                background: theme.cardBg,
                color: theme.textBody,
                fontFamily: font.ui,
                cursor: "pointer",
              }}
            >
              Relaunch now
            </button>
          </div>
        )}
        <MicrophoneTestStep
          result={microphoneResult}
          testing={testingMicrophone}
          unlocked={mic}
          onTest={() => {
            setTestingMicrophone(true);
            void testMicrophone()
              .then(setMicrophoneResult)
              .finally(() => setTestingMicrophone(false));
          }}
        />
        <LanguageStep
          value={settings.dictation_language}
          unlocked={mic}
          onChange={(dictation_language) =>
            onSettingsChange({ ...settings, dictation_language })
          }
        />
        <ModelStep n={6} model={model} unlocked={acc && mic} />
        <TutorialStep unlocked={canEnter} />

        <button
          onClick={onEnter}
          disabled={!canEnter}
          style={{
            marginTop: 12,
            width: "100%",
            cursor: canEnter ? "pointer" : "default",
            border: "none",
            borderRadius: 12,
            padding: "13px",
            fontSize: 15,
            fontWeight: 700,
            fontFamily: font.ui,
            color: "#fff",
            background: canEnter ? theme.accentDeep : theme.textFaint,
          }}
        >
          {canEnter ? "Enter WhimprFlow →" : "Complete setup to continue"}
        </button>

        <p style={{ fontSize: 12, color: theme.textFaint, lineHeight: 1.5, marginTop: 16 }}>
          If a permission stays grey after you flip it on in System Settings, toggle WhimprFlow off
          and back on in that pane — the state here will update within a second.
        </p>
      </div>
    </div>
  );
}
