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
    /// Push-to-talk hotkey, e.g. `option+w` or `fn`. Configured in-app (Accessibility required).
    #[serde(default = "crate::hotkey_binding::default_ptt_hotkey")]
    pub ptt_hotkey: String,
    /// Personalized caps/punctuation tone layered on cleanup.
    #[serde(default)]
    pub writing_style: WritingStyle,
    /// Play the record-start ping.
    pub sound_on_start: bool,
}

fn default_ollama_base_url() -> String {
    "http://localhost:11434/v1".to_string()
}

fn default_ollama_model() -> String {
    "qwen3:1.7b".to_string()
}

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
            ptt_hotkey: crate::hotkey_binding::default_ptt_hotkey(),
            writing_style: WritingStyle::default(),
            sound_on_start: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
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
}
