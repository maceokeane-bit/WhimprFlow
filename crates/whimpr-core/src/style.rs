//! Personalized writing style — caps, punctuation, and spacing only (not word choice).

use serde::{Deserialize, Serialize};

/// User-selected tone applied on top of cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WritingStyle {
    #[default]
    Default,
    Formal,
    Casual,
    VeryCasual,
    Excited,
}

impl WritingStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Formal => "Formal",
            Self::Casual => "Casual",
            Self::VeryCasual => "Very casual",
            Self::Excited => "Excited",
        }
    }

    /// Prompt fragment appended to the cleanup system message.
    pub fn modifier(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Formal => Some(
                "WRITING STYLE: Formal. Use standard capitalization and punctuation. Complete \
                 sentences. Do not change word choice.",
            ),
            Self::Casual => Some(
                "WRITING STYLE: Casual. Light punctuation is fine; conversational tone. Strip \
                 trailing periods on very short messages (under ~8 words). Do not change word choice.",
            ),
            Self::VeryCasual => Some(
                "WRITING STYLE: Very casual. Lowercase is acceptable for short messages; minimal \
                 punctuation; no email-style greeting or sign-off unless dictated. Do not change word choice.",
            ),
            Self::Excited => Some(
                "WRITING STYLE: Excited. Standard capitalization with slightly more exclamation \
                 points where the speaker sounds enthusiastic. Do not change word choice or add new facts.",
            ),
        }
    }
}
