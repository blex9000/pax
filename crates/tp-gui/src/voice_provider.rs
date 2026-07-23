const ALLOWED_STARTS: &[&str] = &[
    "scrivi:",
    "scrivi letteralmente:",
    "va a capo",
    "tastiera:",
    "pax:",
];

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GeminiTranscript {
    pub transcript: Option<String>,
    pub command: String,
    pub raw_text: String,
    pub recorder: String,
    pub audio_bytes: u64,
    pub audio_peak: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct VoiceContext {
    pub panel_type: Option<String>,
    pub workspace: Option<serde_json::Value>,
}

pub(crate) fn normalize_protocol_phrase(text: &str) -> Result<String, String> {
    let mut phrase = text.trim().to_string();
    phrase = phrase.trim().trim_matches(['"', '\'']).to_string();

    let lines: Vec<_> = phrase
        .lines()
        .map(|line| line.trim().trim_start_matches('-').trim())
        .filter(|line| !line.is_empty())
        .collect();
    phrase = lines.join(" ");

    let lower = phrase.to_lowercase();
    if ALLOWED_STARTS
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        Ok(phrase)
    } else {
        Err(format!("Gemini returned non-protocol text: {phrase:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_multiline_protocol_response() {
        let parsed = normalize_protocol_phrase("scrivi: ls\n- tastiera: invio").unwrap();

        assert_eq!(parsed, "scrivi: ls tastiera: invio");
    }

    #[test]
    fn rejects_non_protocol_response() {
        assert!(normalize_protocol_phrase("ciao").is_err());
    }
}
