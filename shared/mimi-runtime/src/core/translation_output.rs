//! Portable, opt-in output policy. Secrets are stored separately by the adapter.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutAction {
    #[default]
    Insert,
    Copy,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeechProvider {
    #[default]
    ElevenLabs,
    OpenAICompatible,
    MainService,
    /// Resolved natively from MainService; not offered as an independent key slot.
    Gemini,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechSettings {
    pub speech_provider: SpeechProvider,
    pub endpoint: String,
    pub model: String,
    pub voice: String,
    pub speed: f64,
    pub stability: f64,
    pub similarity: f64,
    pub device: String,
}
impl Default for SpeechSettings {
    fn default() -> Self {
        TranslationOutput::default().microphone_speech()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TranslationOutput {
    pub shortcut: String,
    pub shortcut_action: ShortcutAction,
    pub auto_copy: bool,
    pub speak_microphone: bool,
    pub speak_system: bool,
    pub speech_provider: SpeechProvider,
    pub endpoint: String,
    pub model: String,
    pub voice: String,
    pub speed: f64,
    pub stability: f64,
    pub similarity: f64,
    /// Native output device ID. Empty is deliberately unconfigured.
    pub device: String,
    /// None migrates legacy shared speech configuration without enabling a lane.
    pub system_speech: Option<SpeechSettings>,
    pub duck_system_audio: bool,
    /// Relative volume of other applications during incoming subtitle playback.
    pub original_volume: f64,
}
impl Default for TranslationOutput {
    fn default() -> Self {
        Self {
            shortcut: String::new(),
            shortcut_action: ShortcutAction::Insert,
            auto_copy: false,
            speak_microphone: false,
            speak_system: false,
            speech_provider: SpeechProvider::ElevenLabs,
            endpoint: "https://api.openai.com/v1/audio/speech".into(),
            model: String::new(),
            voice: String::new(),
            speed: 1.0,
            stability: 0.5,
            similarity: 0.75,
            device: String::new(),
            system_speech: None,
            duck_system_audio: true,
            original_volume: 0.2,
        }
    }
}
impl TranslationOutput {
    pub fn microphone_speech(&self) -> SpeechSettings {
        SpeechSettings {
            speech_provider: self.speech_provider.clone(),
            endpoint: self.endpoint.clone(),
            model: self.model.clone(),
            voice: self.voice.clone(),
            speed: self.speed,
            stability: self.stability,
            similarity: self.similarity,
            device: self.device.clone(),
        }
    }
    pub fn incoming_speech(&self) -> SpeechSettings {
        self.system_speech
            .clone()
            .unwrap_or_else(|| self.microphone_speech())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.shortcut.len() > 80 || !(0.05..=1.0).contains(&self.original_volume) {
            return Err("output_invalid_settings".into());
        }
        self.microphone_speech().validate(self.speak_microphone)?;
        self.incoming_speech().validate(self.speak_system)
    }
}
impl SpeechSettings {
    pub fn validate(&self, enabled: bool) -> Result<(), String> {
        if self.device.len() > 1024
            || !(0.7..=1.2).contains(&self.speed)
            || !(0.0..=1.0).contains(&self.stability)
            || !(0.0..=1.0).contains(&self.similarity)
        {
            return Err("output_invalid_settings".into());
        }
        if enabled {
            if !valid_id(&self.model) || !valid_id(&self.voice) || self.device.is_empty() {
                return Err("output_speech_configuration_required".into());
            }
            if self.speech_provider == SpeechProvider::OpenAICompatible {
                speech_endpoint(&self.endpoint)?;
            }
        }
        Ok(())
    }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
}

/// At most one new final per source per snapshot. Identity includes the session
/// generation, so two genuinely repeated sentences in different sessions survive.
#[derive(Default)]
pub struct OutputCursor {
    seen: [Option<(u64, Option<String>, u64)>; 2],
}
pub struct FinalOutput {
    pub source: crate::core::audio_input::AudioSource,
    pub text: String,
}
impl OutputCursor {
    pub fn observe(
        &mut self,
        snapshot: &crate::core::models::SubtitleSnapshot,
        generation: u64,
    ) -> Vec<FinalOutput> {
        let mut output = Vec::new();
        for track in snapshot.tracks.iter().take(2) {
            if !track.display_pair_final {
                continue;
            }
            let (Some(pair), Some(created)) =
                (&track.display_pair, track.display_pair_created_at_ms)
            else {
                continue;
            };
            if created < generation {
                continue;
            }
            let slot = usize::from(
                track.audio_source == crate::core::audio_input::AudioSource::Microphone,
            );
            let key = (generation, pair.utterance_id.clone(), created);
            if self.seen[slot].as_ref() == Some(&key) {
                continue;
            }
            self.seen[slot] = Some(key);
            if let Some(text) = crate::core::microphone_input::insertion_text(&pair.translation) {
                output.push(FinalOutput {
                    source: track.audio_source,
                    text,
                });
            }
        }
        output
    }
}
pub fn speech_endpoint(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value).map_err(|_| "output_invalid_endpoint".to_string())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.scheme(), "https" | "http")
        || (url.scheme() == "http"
            && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        return Err("output_invalid_endpoint".into());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incoming_reader_needs_no_microphone_model_voice_or_output() {
        let mut output = TranslationOutput {
            speak_system: true,
            system_speech: Some(SpeechSettings {
                model: "eleven_v3".into(),
                voice: "incoming-voice".into(),
                device: "speakers".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(output.validate().is_ok());
        assert!(!output.speak_microphone);
        assert!(output.model.is_empty() && output.device.is_empty());
        output.model = "microphone-model".into();
        output.device = "virtual-cable".into();
        assert_eq!(output.incoming_speech().model, "eleven_v3");
        assert_eq!(output.incoming_speech().device, "speakers");
    }
    #[test]
    fn legacy_configuration_migrates_without_enabling_external_outputs() {
        let output: TranslationOutput = serde_json::from_str(
            r#"{"model":"legacy-model","voice":"old-voice","device":"old-output"}"#,
        )
        .unwrap();
        assert_eq!(output.incoming_speech(), output.microphone_speech());
        assert!(!output.speak_microphone && !output.speak_system);
        let mut bad = output;
        bad.original_volume = 0.0;
        assert!(bad.validate().is_err());
        bad.original_volume = f64::NAN;
        assert!(bad.validate().is_err());
    }
    fn snapshot(final_pair: bool) -> crate::core::models::SubtitleSnapshot {
        serde_json::from_value(serde_json::json!({
            "tracks": [{"audioSource":"microphone","source":{"text":"source","isFinal":true},
                "translation":{"text":"translation","isFinal":true},"history":[],"previewPair":null,
                "displayPair":{"source":"source","translation":"Hello Mimi 12:34","utteranceId":"one"},
                "displayPairFinal":final_pair,"displayPairCreatedAt":10,"detectedLanguage":null,
                "isTranslationPending":false,"isTranslationPreviewPending":false,"isTranslationTimedOut":false,"translationRecovery":null}],
            "source":{"text":"","isFinal":false},"translation":{"text":"","isFinal":false},"history":[],"previewPair":null
        })).unwrap()
    }
    #[test]
    fn finals_only_and_identity_survives_draft_updates_and_repeated_sessions() {
        let mut cursor = OutputCursor::default();
        assert!(cursor.observe(&snapshot(false), 1).is_empty());
        let finals = cursor.observe(&snapshot(true), 1);
        assert_eq!(finals.len(), 1);
        assert_eq!(finals[0].text, "Hello Mimi 12:34");
        assert!(cursor.observe(&snapshot(true), 1).is_empty());
        assert!(cursor.observe(&snapshot(false), 1).is_empty());
        assert!(cursor.observe(&snapshot(true), 1).is_empty());
        assert!(cursor.observe(&snapshot(true), 20).is_empty());
        let mut next = snapshot(true);
        next.tracks[0].display_pair_created_at_ms = Some(30);
        assert_eq!(cursor.observe(&next, 20).len(), 1);
    }
    #[test]
    fn system_and_microphone_finals_keep_separate_identities() {
        let mut s = snapshot(true);
        let mut system = s.tracks[0].clone();
        system.audio_source = crate::core::audio_input::AudioSource::System;
        s.tracks.push(system);
        let mut cursor = OutputCursor::default();
        let finals = cursor.observe(&s, 1);
        assert_eq!(finals.len(), 2);
        assert_ne!(finals[0].source, finals[1].source);
        assert!(cursor.observe(&s, 1).is_empty());
    }
    #[test]
    fn old_installs_have_no_external_output() {
        let settings: TranslationOutput = serde_json::from_str("{}").unwrap();
        assert!(!settings.auto_copy && !settings.speak_microphone && !settings.speak_system);
        assert!(settings.shortcut.is_empty());
        assert!(settings.validate().is_ok());
    }
    #[test]
    fn speech_requires_explicit_model_voice_device_and_finite_controls() {
        let mut s = TranslationOutput {
            speak_microphone: true,
            ..Default::default()
        };
        assert!(s.validate().is_err());
        s.model = "eleven_v3".into();
        s.voice = "voice1".into();
        s.device = "device1".into();
        assert!(s.validate().is_ok());
        s.speed = f64::NAN;
        assert!(s.validate().is_err());
    }
    #[test]
    fn credentials_are_not_allowed_in_custom_urls() {
        for url in [
            "https://key@example.com/audio/speech",
            "http://example.com/audio/speech",
            "https://example.com/?key=secret",
        ] {
            assert!(speech_endpoint(url).is_err());
        }
        assert!(speech_endpoint("http://127.0.0.1:8000/v1/audio/speech").is_ok());
    }
}
