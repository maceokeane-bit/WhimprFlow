//! Voice-triggered text snippets: say the trigger phrase, get the expansion.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// One snippet: trigger phrase → expanded text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub trigger: String,
    pub expansion: String,
}

/// Persisted snippet list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnippetStore {
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

impl SnippetStore {
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

    pub fn add(&mut self, trigger: String, expansion: String) {
        let trigger = trigger.trim().to_string();
        if trigger.is_empty() {
            return;
        }
        if let Some(existing) = self
            .snippets
            .iter_mut()
            .find(|s| s.trigger.eq_ignore_ascii_case(&trigger))
        {
            existing.expansion = expansion;
        } else {
            self.snippets.push(Snippet { trigger, expansion });
        }
    }

    pub fn remove(&mut self, trigger: &str) -> bool {
        let before = self.snippets.len();
        self.snippets
            .retain(|s| !s.trigger.eq_ignore_ascii_case(trigger));
        self.snippets.len() < before
    }

    /// If the whole utterance matches a trigger (case-insensitive), return the expansion.
    pub fn expand(&self, text: &str) -> Option<String> {
        let t = text.trim();
        self.snippets
            .iter()
            .find(|s| s.trigger.eq_ignore_ascii_case(t))
            .map(|s| s.expansion.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_exact_trigger() {
        let mut store = SnippetStore::default();
        store.add("my sig".into(), "Best,\nMaceo".into());
        assert_eq!(store.expand("my sig"), Some("Best,\nMaceo".into()));
        assert_eq!(store.expand("something else"), None);
    }
}
