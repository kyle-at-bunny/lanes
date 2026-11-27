//! A session's conversation as shown in the Transcript tab; each agent adapter reads its own format into this.

const MAX_ENTRY_CHARS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptEntry {
    pub speaker: Speaker,
    pub text: String,
}

/// An entry with long text truncated, so one huge tool output can't swamp the tab.
pub(crate) fn entry(speaker: Speaker, text: &str) -> TranscriptEntry {
    let trimmed = text.trim();
    let text = match trimmed.char_indices().nth(MAX_ENTRY_CHARS) {
        Some((cut, _)) => format!("{}… [truncated]", &trimmed[..cut]),
        None => trimmed.to_string(),
    };
    TranscriptEntry { speaker, text }
}
