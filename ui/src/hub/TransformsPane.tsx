import { useCallback, useEffect, useState } from "react";
import { font } from "../tokens/values";
import { theme } from "./theme";
import { Button, Card, PageTitle } from "./ui";
import {
  getTransforms,
  removeTransform,
  runTransform,
  saveTransform,
  type TransformPreset,
} from "./api";

export function TransformsPane() {
  const [presets, setPresets] = useState<TransformPreset[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [customId, setCustomId] = useState("");
  const [customName, setCustomName] = useState("");
  const [customInstruction, setCustomInstruction] = useState("");

  const refresh = useCallback(async () => {
    setPresets(await getTransforms());
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const inputStyle = {
    width: "100%",
    background: theme.cardBgSubtle,
    border: `1px solid ${theme.border}`,
    borderRadius: 10,
    padding: "9px 12px",
    color: theme.textBody,
    fontFamily: font.ui,
    fontSize: 13.5,
    outline: "none",
    boxSizing: "border-box" as const,
  };

  const apply = async (id: string) => {
    setBusy(id);
    setMessage(null);
    try {
      const out = await runTransform(id);
      setMessage(`Applied — ${out.length} characters pasted.`);
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div style={{ maxWidth: 720 }}>
      <PageTitle>Transforms</PageTitle>
      <p style={{ color: theme.textMuted, fontSize: 14, lineHeight: 1.55, marginTop: -8, marginBottom: 20 }}>
        Highlight text in any app, then run a transform here. WhimprFlow copies the selection, rewrites it
        via Ollama, and pastes the result back. Requires Accessibility + Ollama running.
      </p>

      <Card style={{ marginBottom: 16, borderColor: theme.accentSoftBorder }}>
        <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>Voice Command Mode</div>
        <div style={{ fontSize: 13, color: theme.textMuted, marginTop: 5, lineHeight: 1.5 }}>
          Select text, hold <b>Command + Control + Option</b> (or <b>Fn + Control</b>), speak an
          instruction, then release. With no selection, WhimprFlow generates the requested content
          at the cursor. Esc cancels. Disable under Settings → Experimental.
        </div>
      </Card>

      {message && (
        <Card style={{ marginBottom: 16, borderColor: theme.accentSoftBorder }}>
          <div style={{ fontSize: 13, color: theme.textBody }}>{message}</div>
        </Card>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: 10, marginBottom: 20 }}>
        {presets.map((p) => (
          <Card key={p.id} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong }}>{p.name}</div>
              <div style={{ fontSize: 12.5, color: theme.textMuted, marginTop: 4 }}>{p.instruction}</div>
            </div>
            <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
              <Button size="sm" disabled={busy === p.id} onClick={() => void apply(p.id)}>
                {busy === p.id ? "Running…" : "Run"}
              </Button>
              {!["polish", "summarize", "formal", "bullets"].includes(p.id) && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={async () => {
                    await removeTransform(p.id);
                    void refresh();
                  }}
                >
                  Delete
                </Button>
              )}
            </div>
          </Card>
        ))}
      </div>

      <Card>
        <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong, marginBottom: 12 }}>
          Custom transform
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <input
            value={customName}
            onChange={(e) => {
              setCustomName(e.target.value);
              setCustomId(e.target.value.toLowerCase().replace(/\s+/g, "-").slice(0, 32));
            }}
            placeholder="Name (e.g. Shorter)"
            style={inputStyle}
          />
          <textarea
            value={customInstruction}
            onChange={(e) => setCustomInstruction(e.target.value)}
            placeholder="Instruction for the model"
            rows={3}
            style={{ ...inputStyle, resize: "vertical" }}
          />
          <Button
            onClick={async () => {
              if (!customId.trim() || !customName.trim() || !customInstruction.trim()) return;
              await saveTransform({
                id: customId.trim(),
                name: customName.trim(),
                instruction: customInstruction.trim(),
              });
              setCustomName("");
              setCustomId("");
              setCustomInstruction("");
              void refresh();
            }}
          >
            Save preset
          </Button>
        </div>
      </Card>
    </div>
  );
}
