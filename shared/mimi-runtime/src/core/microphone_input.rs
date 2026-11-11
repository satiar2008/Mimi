//! Select one bounded confirmed microphone pair for an explicit insertion.
use crate::core::{audio_input::AudioSource, models::SubtitleSnapshot};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MicrophoneInsertion {
    pub utterance_id: Option<String>,
    pub created_at_ms: u64,
    pub text: String,
}

impl MicrophoneInsertion {
    pub fn from_subtitles(snapshot: &SubtitleSnapshot) -> Option<Self> {
        let track = snapshot
            .tracks
            .iter()
            .find(|t| t.audio_source == AudioSource::Microphone)?;
        if !track.display_pair_final {
            return None;
        }
        let pair = track.display_pair.as_ref()?;
        let text = insertion_text(&pair.translation)?;
        Some(Self {
            utterance_id: pair.utterance_id.clone(),
            created_at_ms: track.display_pair_created_at_ms?,
            text,
        })
    }
}

/// Text input never injects control keys, line submission, or focus traversal.
pub fn insertion_text(text: &str) -> Option<String> {
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text = text.trim();
    (!text.is_empty() && text.encode_utf16().count() <= 4096).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn microphone_input_preserves_unicode_without_submission_keys() {
        assert_eq!(
            insertion_text("سلام Mimi 2.0 (test) 👋\r\nnext\ttext"),
            Some("سلام Mimi 2.0 (test) 👋  next text".into())
        );
        assert!(insertion_text("\r\n\t").is_none());
        assert!(insertion_text(&"x".repeat(4097)).is_none());
        assert!(insertion_text(&"👋".repeat(2049)).is_none());
    }
    #[test]
    fn microphone_input_requires_confirmed_microphone_not_system_or_draft() {
        let mut value = serde_json::json!({
            "tracks": [{"audioSource":"microphone", "source":{"text":"source","isFinal":true,"utteranceId":null},
              "translation":{"text":"translation","isFinal":true,"utteranceId":null},"history":[],"previewPair":null,
              "displayPair":{"source":"source","translation":"سلام Mimi","utteranceId":"u1"},
              "displayPairFinal":true,"displayPairCreatedAt":42,"detectedLanguage":null,
              "isTranslationPending":false,"isTranslationPreviewPending":false,"isTranslationTimedOut":false,"translationRecovery":null}],
            "source":{"text":"","isFinal":false,"utteranceId":null},
            "translation":{"text":"","isFinal":false,"utteranceId":null},"history":[],"previewPair":null
        });
        let parse =
            |v: &serde_json::Value| serde_json::from_value::<SubtitleSnapshot>(v.clone()).unwrap();
        assert_eq!(
            MicrophoneInsertion::from_subtitles(&parse(&value))
                .unwrap()
                .text,
            "سلام Mimi"
        );
        value["tracks"][0]["displayPairFinal"] = false.into();
        assert!(MicrophoneInsertion::from_subtitles(&parse(&value)).is_none());
        value["tracks"][0]["displayPairFinal"] = true.into();
        value["tracks"][0]["audioSource"] = "system".into();
        assert!(MicrophoneInsertion::from_subtitles(&parse(&value)).is_none());
    }
}
