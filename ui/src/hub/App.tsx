import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { font } from "../tokens/values";
import { theme } from "./theme";
import { Onboarding } from "./Onboarding";
import { Sidebar, type Page } from "./Sidebar";
import { Home } from "./Home";
import { Insights } from "./Insights";
import { DictionaryPane } from "./DictionaryPane";
import { SettingsPane } from "./SettingsPane";
import { Help } from "./Help";
import { SnippetsPane } from "./SnippetsPane";
import { StylePane } from "./StylePane";
import { TransformsPane } from "./TransformsPane";
import { ComingSoon } from "./ComingSoon";
import type { IconName } from "./icons";
import {
  getSettings,
  getModelDownloadStatus,
  setSettings,
  getStatus,
  type ModelDownloadStatus,
  type Settings,
  type Status,
  DEFAULT_SETTINGS,
  EMPTY_MODEL_STATUS,
} from "./api";

// Placeholder screens that are routed but not yet built.
const SOON: Partial<Record<Page, { icon: IconName; title: string; desc: string }>> = {
  scratchpad: {
    icon: "scratchpad",
    title: "Scratchpad",
    desc: "A quiet place to dictate long-form and shape it before it lands anywhere else.",
  },
};

export function App() {
  const [page, setPage] = useState<Page>("home");
  const [settings, setLocalSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [entered, setEntered] = useState(false);
  const [model, setModel] = useState<ModelDownloadStatus>(EMPTY_MODEL_STATUS);
  const [status, setStatus] = useState<Status>({
    accessibility: false,
    microphone: false,
    input_monitoring: false,
    has_openai_key: false,
    has_anthropic_key: false,
  });

  const refresh = useCallback(() => {
    void getStatus().then(setStatus);
    void getModelDownloadStatus().then(setModel);
  }, []);

  useEffect(() => {
    getSettings().then(setLocalSettings);
    refresh();
    let unlisten: (() => void) | undefined;
    void listen<Page>("whimpr://hub/navigate", (event) => setPage(event.payload)).then(
      (cleanup) => (unlisten = cleanup),
    );
    return () => unlisten?.();
  }, [refresh]);

  const update = (s: Settings) => {
    setLocalSettings(s);
    void setSettings(s);
  };

  // Gate first run until permissions, models, language, and tutorial are complete.
  const setupIncomplete =
    !settings.onboarding_complete ||
    !status.accessibility ||
    !status.microphone ||
    model.state !== "ready";
  if (setupIncomplete && !entered) {
    return (
      <Onboarding
        status={status}
        model={model}
        settings={settings}
        onSettingsChange={update}
        refresh={refresh}
        onEnter={() => {
          update({ ...settings, onboarding_complete: true });
          setEntered(true);
        }}
      />
    );
  }

  const soon = SOON[page];

  return (
    <div
      style={{
        display: "flex",
        height: "100vh",
        fontFamily: font.ui,
        color: theme.textBody,
        background: theme.pageBg,
      }}
    >
      <Sidebar page={page} setPage={setPage} />
      <main style={{ flex: 1, minWidth: 0, overflowY: "auto" }}>
        <div style={{ padding: "36px 44px", margin: "0 auto", maxWidth: 1120 }}>
          {page === "home" && <Home />}
          {page === "insights" && <Insights />}
          {page === "dictionary" && <DictionaryPane />}
          {page === "snippets" && <SnippetsPane />}
          {page === "style" && <StylePane settings={settings} onChange={update} />}
          {page === "transforms" && <TransformsPane />}
          {page === "settings" && (
            <SettingsPane settings={settings} onChange={update} status={status} refresh={refresh} />
          )}
          {page === "help" && <Help />}
          {soon && <ComingSoon icon={soon.icon} title={soon.title} desc={soon.desc} />}
        </div>
      </main>
    </div>
  );
}
