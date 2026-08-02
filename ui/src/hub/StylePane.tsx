import { font } from "../tokens/values";
import { theme } from "./theme";
import { Card, PageTitle } from "./ui";
import type { Settings, WritingStyle } from "./api";

const STYLES: { value: WritingStyle; label: string; hint: string }[] = [
  { value: "default", label: "Default", hint: "No extra tone rules — cleanup level drives edits." },
  { value: "formal", label: "Formal", hint: "Standard capitalization and punctuation. Complete sentences." },
  { value: "casual", label: "Casual", hint: "Conversational; trailing periods stripped on very short messages." },
  { value: "very_casual", label: "Very casual", hint: "Lowercase OK for short messages; minimal punctuation." },
  { value: "excited", label: "Excited", hint: "A bit more energy — extra exclamation where it fits." },
];

export function StylePane({
  settings,
  onChange,
}: {
  settings: Settings;
  onChange: (s: Settings) => void;
}) {
  return (
    <div style={{ maxWidth: 720 }}>
      <PageTitle>Style</PageTitle>
      <p style={{ color: theme.textMuted, fontSize: 14, lineHeight: 1.55, marginTop: -8, marginBottom: 20 }}>
        Personalized Style adjusts capitalization, punctuation, and spacing only — not your word choice.
        It layers on top of Auto Cleanup in Settings.
      </p>

      <Card>
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {STYLES.map((s) => {
            const selected = settings.writing_style === s.value;
            return (
              <button
                key={s.value}
                onClick={() => onChange({ ...settings, writing_style: s.value })}
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
                <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>{s.label}</div>
                <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 2 }}>{s.hint}</div>
              </button>
            );
          })}
        </div>
      </Card>
    </div>
  );
}
