//! User settings, persisted as JSON. Drives the cleanup engine (which provider,
//! how aggressive) and other behavior. Kept dependency-light so it lives in core.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::cleanup::CleanupLevel;
use crate::style::WritingStyle;

/// Which cleanup engine processes transcripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CleanupMode {
    /// Paste the raw transcript (no cleanup).
    Raw,
    /// Local on-device GGUF worker (offline backup when Ollama is down).
    Local,
    /// Ollama on this machine (`http://localhost:11434`, OpenAI-compatible API).
    #[default]
    Ollama,
    /// OpenAI cloud (or any OpenAI-compatible API — DeepSeek, OpenRouter, etc.).
    OpenAi,
    /// Anthropic cloud.
    Anthropic,
}

/// Persisted user configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub cleanup_mode: CleanupMode,
    pub cleanup_level: CleanupLevel,
    pub openai_model: String,
    /// API root for the "OpenAI" cleanup mode, e.g. `https://openrouter.ai/api/v1`
    /// to route through OpenRouter instead of OpenAI directly (same wire format).
    /// Empty string (the default) means OpenAI's own endpoint.
    #[serde(default)]
    pub openai_base_url: String,
    pub anthropic_model: String,
    /// Ollama API root, e.g. `http://localhost:11434/v1`.
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,
    /// Ollama model tag, e.g. `qwen3:8b`.
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,
    /// Optional GGUF filename under `models/` for Local mode (blank = auto-detect).
    #[serde(default)]
    pub local_model: String,
    /// Start WhimprFlow automatically when you log in to macOS.
    #[serde(default)]
    pub launch_at_login: bool,
    /// BCP-47/ISO language code used by the ASR backend.
    #[serde(default = "default_dictation_language")]
    pub dictation_language: String,
    /// The first-run tutorial and required setup steps have been completed.
    #[serde(default)]
    pub onboarding_complete: bool,
    /// Keep the floating Flow Bar visible and dictation enabled.
    #[serde(default = "default_show_flow_bar")]
    pub show_flow_bar: bool,
    /// Unix timestamp until which the Flow Bar is temporarily snoozed.
    #[serde(default)]
    pub flow_bar_snoozed_until: Option<u64>,
    /// Push-to-talk hotkey, e.g. `option+w` or `fn`. Configured in-app (Accessibility required).
    #[serde(default = "crate::hotkey_binding::default_ptt_hotkey")]
    pub ptt_hotkey: String,
    /// Personalized caps/punctuation tone layered on cleanup.
    #[serde(default)]
    pub writing_style: WritingStyle,
    /// Play the record-start ping.
    pub sound_on_start: bool,
    /// Pause Spotify/Music/browser media when dictation starts; resume when it stops.
    #[serde(default = "default_pause_media_while_dictating")]
    pub pause_media_while_dictating: bool,
}

fn default_pause_media_while_dictating() -> bool {
    true
}

fn default_show_flow_bar() -> bool {
    true
}

fn default_dictation_language() -> String {
    "en".to_string()
}

fn default_ollama_base_url() -> String {
    "http://localhost:11434/v1".to_string()
}

fn default_ollama_model() -> String {
    "qwen3:8b".to_string()
}

/// Legacy default before we standardized on qwen3:8b for cleanup + insights.
const LEGACY_OLLAMA_MODEL: &str = "qwen3:1.7b";

impl Default for Settings {
    fn default() -> Self {
        Self {
            cleanup_mode: CleanupMode::default(),
            cleanup_level: CleanupLevel::Light,
            openai_model: "gpt-4o-mini".to_string(),
            openai_base_url: String::new(),
            anthropic_model: "claude-haiku-4-5".to_string(),
            ollama_base_url: default_ollama_base_url(),
            ollama_model: default_ollama_model(),
            local_model: String::new(),
            launch_at_login: false,
            dictation_language: default_dictation_language(),
            onboarding_complete: false,
            show_flow_bar: default_show_flow_bar(),
            flow_bar_snoozed_until: None,
            ptt_hotkey: crate::hotkey_binding::default_ptt_hotkey(),
            writing_style: WritingStyle::default(),
            sound_on_start: true,
            pause_media_while_dictating: default_pause_media_while_dictating(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // One-time migration: older installs defaulted to the smaller tag.
        if settings.ollama_model == LEGACY_OLLAMA_MODEL {
            settings.ollama_model = default_ollama_model();
            let _ = settings.save(path);
        }

        settings
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.cleanup_mode, CleanupMode::Ollama);
        assert_eq!(s.cleanup_level, CleanupLevel::Light);
        assert_eq!(s.dictation_language, "en");
        assert!(!s.onboarding_complete);
    }

    #[test]
    fn round_trips_json() {
        let s = Settings {
            cleanup_mode: CleanupMode::Local,
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cleanup_mode, CleanupMode::Local);
    }

    #[test]
    fn migrates_legacy_ollama_model_on_load() {
        let dir = std::env::temp_dir().join(format!("whimpr-settings-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"cleanup_mode":"ollama","cleanup_level":"light","openai_model":"gpt-4o-mini","openai_base_url":"","anthropic_model":"claude-haiku-4-5","ollama_base_url":"http://localhost:11434/v1","ollama_model":"qwen3:1.7b","local_model":"","launch_at_login":false,"ptt_hotkey":"option+w","writing_style":"default","sound_on_start":true}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.ollama_model, "qwen3:8b");
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("qwen3:8b"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
