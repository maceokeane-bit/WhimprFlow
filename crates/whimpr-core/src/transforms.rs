//! Named rewrite presets (Polish, Summarize, etc.) for selected text.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// A transform preset the user can run on highlighted text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformPreset {
    pub id: String,
    pub name: String,
    pub instruction: String,
}

/// Persisted transform list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformStore {
    pub presets: Vec<TransformPreset>,
}

impl Default for TransformStore {
    fn default() -> Self {
        Self {
            presets: vec![
                TransformPreset {
                    id: "polish".into(),
                    name: "Polish".into(),
                    instruction: "Improve clarity and conciseness. Fix grammar and punctuation. \
                                    Preserve meaning and facts.".into(),
                },
                TransformPreset {
                    id: "summarize".into(),
                    name: "Summarize".into(),
                    instruction: "Summarize the selected text in 2-4 sentences. Keep key facts.".into(),
                },
                TransformPreset {
                    id: "formal".into(),
                    name: "Make formal".into(),
                    instruction: "Rewrite in a professional, formal tone. Preserve all facts.".into(),
                },
                TransformPreset {
                    id: "bullets".into(),
                    name: "To bullet list".into(),
                    instruction: "Convert the content into a clear bulleted list. One idea per line.".into(),
                },
            ],
        }
    }
}

impl TransformStore {
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

    pub fn find(&self, id: &str) -> Option<&TransformPreset> {
        self.presets.iter().find(|p| p.id == id)
    }

    pub fn upsert(&mut self, preset: TransformPreset) {
        if let Some(existing) = self.presets.iter_mut().find(|p| p.id == preset.id) {
            *existing = preset;
        } else {
            self.presets.push(preset);
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.presets.len();
        self.presets.retain(|p| p.id != id);
        self.presets.len() < before
    }
}

/// System prompt for transform / command mode.
pub const TRANSFORM_SYSTEM: &str = "\
You transform the user's SELECTED TEXT according to their instruction. Return ONLY the \
transformed text — no explanation, quotes, or markdown fences. Preserve facts, names, \
numbers, and meaning unless the instruction explicitly asks to change them.";
