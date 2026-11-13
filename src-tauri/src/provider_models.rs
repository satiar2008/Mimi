//! Model identity and discovery always use the requested saved profile's own credentials.
use crate::commands::AppState;
use mimi_runtime::{
    clients::{
        provider_network::ProviderNetwork,
        service_catalog::{self, CatalogItem},
    },
    core::{
        configuration::TextTranslationProbeCredentials,
        credentials::{ProviderCredentials, TextTranslationCredentials},
        provider::{ProviderKind, ServiceProfile, TextTranslation},
    },
};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    pub current: Vec<String>,
    pub models: Vec<CatalogItem>,
    pub editable: bool,
}

struct TextModelDestination {
    endpoint: String,
    model: String,
    key: String,
    route: TextTranslation,
    network: ProviderNetwork,
}
fn text_destination(
    store: &crate::settings_store::SettingsStore,
    profile: &ServiceProfile,
) -> Result<TextModelDestination, String> {
    let config = store.configuration_for_text_probe(profile)?;
    let (endpoint, model, key, route) = match config.credentials {
        TextTranslationProbeCredentials::Independent(
            TextTranslationCredentials::OpenAICompatible {
                endpoint,
                model,
                api_key,
            },
        ) => (endpoint, model, api_key, TextTranslation::OpenAICompatible),
        TextTranslationProbeCredentials::Independent(TextTranslationCredentials::ChatMock {
            endpoint,
            model,
            api_key,
        }) => (endpoint, model, api_key, TextTranslation::ChatMock),
        TextTranslationProbeCredentials::Independent(TextTranslationCredentials::XAI {
            model,
            api_key,
        }) => (
            "https://api.x.ai/v1/chat/completions".into(),
            model,
            api_key,
            TextTranslation::XAI,
        ),
        _ => return Err("catalog_not_supported".into()),
    };
    let network = ProviderNetwork::resolve(&config.network_proxy)
        .map_err(|_| "catalog_network".to_string())?;
    Ok(TextModelDestination {
        endpoint,
        model,
        key,
        route,
        network,
    })
}

#[tauri::command]
pub async fn provider_models_get(
    state: State<'_, AppState>,
    profile_id: String,
    fetch: bool,
    text: bool,
) -> Result<ModelView, String> {
    let (_, profiles) = state.settings.profile_catalog()?;
    let profile = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or("profile_not_found")?;
    if text {
        let destination = text_destination(&state.settings, profile)?;
        let mut view = ModelView {
            current: vec![String::new(), destination.model],
            models: Vec::new(),
            editable: true,
        };
        if fetch {
            if crate::commands::app_is_ui_test() {
                return Err("output_ui_test".into());
            }
            view.models = service_catalog::compatible_models(
                &destination.endpoint,
                &destination.key,
                &destination.network,
            )
            .await?;
        }
        return Ok(view);
    }
    let config = state
        .settings
        .configuration_for_profile_probe(profile)
        .or_else(|_| state.settings.configuration_for_speech_probe(profile))?;
    let mut view = ModelView {
        current: config.model_names(),
        models: Vec::new(),
        editable: profile.provider == ProviderKind::GoogleGeminiLive
            && !profile.uses_independent_asr()
            || profile.provider.is_custom_speech()
            || text,
    };
    if !fetch {
        return Ok(view);
    }
    if crate::commands::app_is_ui_test() {
        return Err("output_ui_test".into());
    }
    let network = ProviderNetwork::resolve(&config.network_proxy)
        .map_err(|_| "catalog_network".to_string())?;
    if profile.provider == ProviderKind::GoogleGeminiLive {
        let ProviderCredentials::ApiKey { api_key } = &config.credentials else {
            return Err("catalog_key_required".into());
        };
        view.models = service_catalog::gemini_models(api_key, &network).await?;
        if profile.uses_independent_asr() {
            for m in &mut view.models {
                m.compatible = m.id == mimi_runtime::core::protocols::gemini_transcribe::MODEL;
            }
        }
    } else {
        view.models = match &config.credentials {
            ProviderCredentials::CustomSpeech {
                endpoint, api_key, ..
            } => service_catalog::compatible_models(endpoint, api_key, &network).await?,
            ProviderCredentials::ApiKey { api_key }
                if profile.provider == ProviderKind::OpenAIRealtime =>
            {
                service_catalog::compatible_models(
                    "https://api.openai.com/v1/realtime",
                    api_key,
                    &network,
                )
                .await?
                .into_iter()
                .map(|mut m| {
                    m.compatible = view.current.contains(&m.id);
                    m
                })
                .collect()
            }
            _ => return Err("catalog_not_supported".into()),
        };
    }
    Ok(view)
}

#[tauri::command]
pub async fn provider_model_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    model: String,
    text: bool,
) -> Result<(), String> {
    if !mimi_runtime::core::translation_output::valid_id(&model) {
        return Err("catalog_invalid_model".into());
    }
    let _guard = state.session.settings_mutation_guard(true).await?;
    let (_, profiles) = state.settings.profile_catalog()?;
    let profile = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or("profile_not_found")?;
    if text {
        let destination = text_destination(&state.settings, profile)?;
        let kind = destination.route;
        let endpoint = if kind == TextTranslation::XAI {
            String::new()
        } else {
            destination.endpoint
        };
        state.settings.save_credentials(
            &profile_id,
            &ProviderCredentials::AlibabaTranslation {
                api_key: String::new(),
                text_translation: kind,
                endpoint,
                token: String::new(),
                model,
                clear_token: false,
            },
        )?;
    } else if profile.provider == ProviderKind::GoogleGeminiLive && !profile.uses_independent_asr()
    {
        if !service_catalog::gemini_translation_model(&model) {
            return Err("catalog_incompatible_model".into());
        }
        state.settings.save_preferences(|prefs| {
            prefs.speech_models.insert(profile_id.clone(), model);
        })?;
    } else if let ProviderCredentials::CustomSpeech {
        endpoint, api_key, ..
    } = state
        .settings
        .configuration_for_speech_probe(profile)?
        .credentials
    {
        state.settings.save_credentials(
            &profile_id,
            &ProviderCredentials::CustomSpeech {
                endpoint,
                api_key,
                model,
            },
        )?;
    } else {
        return Err("catalog_not_supported".into());
    }
    crate::commands::emit_settings_snapshot(&app, &state.settings)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_store::{SecretStore, SecretStoreError, SettingsStore};
    use std::{collections::HashMap, sync::Mutex};
    #[derive(Default)]
    struct MemorySecrets(Mutex<HashMap<String, String>>);
    impl SecretStore for MemorySecrets {
        fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&format!("{service}:{account}"))
                .cloned())
        }
        fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .unwrap()
                .insert(format!("{service}:{account}"), value.into());
            Ok(())
        }
        fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .unwrap()
                .remove(&format!("{service}:{account}"));
            Ok(())
        }
    }
    #[test]
    fn text_model_catalog_does_not_require_or_borrow_speech_credentials() {
        let store = SettingsStore::in_memory(Box::<MemorySecrets>::default(), false);
        let profile = store
            .create_profile(ProviderKind::CustomOpenAIASR, "synthetic")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &ProviderCredentials::AlibabaTranslation {
                    api_key: String::new(),
                    text_translation: TextTranslation::OpenAICompatible,
                    endpoint: "https://example.com/v1/chat/completions".into(),
                    token: "synthetic-text-only-key".into(),
                    model: "selected-text-model".into(),
                    clear_token: false,
                },
            )
            .unwrap();
        let (_, profiles) = store.profile_catalog().unwrap();
        let profile = profiles.iter().find(|p| p.id == profile.id).unwrap();
        assert!(store.configuration_for_speech_probe(profile).is_err());
        let destination = text_destination(&store, profile).unwrap();
        assert_eq!(destination.model, "selected-text-model");
        assert_eq!(destination.key, "synthetic-text-only-key");
        assert_eq!(
            destination.endpoint,
            "https://example.com/v1/chat/completions"
        );
    }
}
