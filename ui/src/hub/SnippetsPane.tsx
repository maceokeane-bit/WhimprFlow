import { useCallback, useEffect, useState } from "react";
import { font } from "../tokens/values";
import { theme } from "./theme";
import { Button, Card, PageTitle } from "./ui";
import { addSnippet, getSnippets, removeSnippet, type Snippet } from "./api";

export function SnippetsPane() {
  const [items, setItems] = useState<Snippet[]>([]);
  const [trigger, setTrigger] = useState("");
  const [expansion, setExpansion] = useState("");

  const refresh = useCallback(async () => {
    setItems(await getSnippets());
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

  return (
    <div style={{ maxWidth: 720 }}>
      <PageTitle>Snippets</PageTitle>
      <p style={{ color: theme.textMuted, fontSize: 14, lineHeight: 1.55, marginTop: -8, marginBottom: 20 }}>
        Say the trigger phrase exactly — WhimprFlow expands it before cleanup. Great for signatures,
        addresses, and boilerplate.
      </p>

      <Card style={{ marginBottom: 16 }}>
        <div style={{ fontSize: 14, fontWeight: 600, color: theme.textStrong, marginBottom: 12 }}>Add snippet</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <input
            value={trigger}
            onChange={(e) => setTrigger(e.target.value)}
            placeholder='Trigger phrase (e.g. "my signature")'
            style={inputStyle}
          />
          <textarea
            value={expansion}
            onChange={(e) => setExpansion(e.target.value)}
            placeholder="Expanded text"
            rows={4}
            style={{ ...inputStyle, resize: "vertical" }}
          />
          <Button
            onClick={async () => {
              if (!trigger.trim() || !expansion.trim()) return;
              await addSnippet(trigger.trim(), expansion);
              setTrigger("");
              setExpansion("");
              void refresh();
            }}
          >
            Save snippet
          </Button>
        </div>
      </Card>

      <Card pad={0}>
        {items.length === 0 ? (
          <div style={{ padding: "32px 18px", textAlign: "center", color: theme.textFaint, fontSize: 13.5 }}>
            No snippets yet. Add one above, then hold your hotkey and say the trigger phrase.
          </div>
        ) : (
          items.map((s) => (
            <div
              key={s.trigger}
              style={{
                display: "flex",
                gap: 12,
                padding: "14px 18px",
                borderBottom: `1px solid ${theme.border}`,
                alignItems: "flex-start",
              }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: theme.accentDeep }}>"{s.trigger}"</div>
                <div style={{ fontSize: 13, color: theme.textBody, marginTop: 6, whiteSpace: "pre-wrap" }}>
                  {s.expansion}
                </div>
              </div>
              <button
                onClick={async () => {
                  await removeSnippet(s.trigger);
                  void refresh();
                }}
                style={{
                  border: "none",
                  background: "transparent",
                  cursor: "pointer",
                  color: theme.textFaint,
                  fontSize: 18,
                }}
              >
                ×
              </button>
            </div>
          ))
        )}
      </Card>
    </div>
  );
}
