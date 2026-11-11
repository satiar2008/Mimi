//! Independent microphone language selection. Platform adapters own capture.
use crate::core::models::{SourceLanguage, TargetLanguage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct MicrophoneTranslation {
    /// Off preserves existing shared-language sessions on upgrade.
    pub enabled: bool,
    pub source_language: SourceLanguage,
    /// None means the explicitly selected incoming/system speech language.
    pub target_language: Option<TargetLanguage>,
}

impl Default for MicrophoneTranslation {
    fn default() -> Self {
        Self {
            enabled: false,
            source_language: SourceLanguage::Automatic,
            target_language: None,
        }
    }
}

impl MicrophoneTranslation {
    pub fn languages(
        &self,
        incoming: SourceLanguage,
        shared_target: TargetLanguage,
    ) -> Result<(SourceLanguage, TargetLanguage), &'static str> {
        if !self.enabled {
            return Ok((incoming, shared_target));
        }
        let target = match self.target_language {
            Some(target) if target.translates_audio() => target,
            Some(_) => return Err("microphone_translation_target_required"),
            None => TargetLanguage::ALL
                .iter()
                .copied()
                .find(|target| {
                    target.translates_audio() && target.raw_value() == incoming.raw_value()
                })
                .ok_or("microphone_translation_incoming_language_required")?,
        };
        Ok((self.source_language, target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn microphone_translation_preserves_legacy_and_reverses_explicit_languages() {
        let mut choice = MicrophoneTranslation::default();
        assert_eq!(
            choice.languages(SourceLanguage::English, TargetLanguage::Persian),
            Ok((SourceLanguage::English, TargetLanguage::Persian))
        );
        choice.enabled = true;
        choice.source_language = SourceLanguage::Persian;
        assert_eq!(
            choice.languages(SourceLanguage::English, TargetLanguage::Persian),
            Ok((SourceLanguage::Persian, TargetLanguage::English))
        );
        assert_eq!(
            choice.languages(SourceLanguage::Japanese, TargetLanguage::Persian),
            Ok((SourceLanguage::Persian, TargetLanguage::Japanese))
        );
        assert!(choice
            .languages(SourceLanguage::Automatic, TargetLanguage::Persian)
            .is_err());
        choice.target_language = Some(TargetLanguage::German);
        assert_eq!(
            choice.languages(SourceLanguage::Automatic, TargetLanguage::Persian),
            Ok((SourceLanguage::Persian, TargetLanguage::German))
        );
        choice.target_language = Some(TargetLanguage::Original);
        assert!(choice
            .languages(SourceLanguage::English, TargetLanguage::Persian)
            .is_err());
    }
}
