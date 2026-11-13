//! Desktop output adapter: explicit global action, opt-in clipboard and bounded speech.
use crate::{commands::AppState, session_manager::SessionManager, settings_store::SettingsStore};
use mimi_runtime::{
    clients::{provider_network::ProviderNetwork, service_catalog},
    core::translation_output::{ShortcutAction, SpeechProvider, TranslationOutput},
};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, LazyLock,
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

static MUTATION: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));
static REVISION: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSnapshot {
    settings: TranslationOutput,
    key_present: bool,
}
fn snapshot(settings: &SettingsStore) -> Result<OutputSnapshot, String> {
    let value = settings.preferences().translation_output;
    let key_present = settings
        .speech_output_key(&value.speech_provider)?
        .is_some_and(|key| !key.is_empty());
    Ok(OutputSnapshot {
        settings: value,
        key_present,
    })
}
#[tauri::command]
pub fn translation_output_get(state: State<'_, AppState>) -> Result<OutputSnapshot, String> {
    snapshot(&state.settings)
}

fn shortcut(value: &str) -> Result<Option<Shortcut>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let parsed: Shortcut = value.parse().map_err(|_| "output_invalid_shortcut")?;
    // F keys are allowed without modifiers; text keys require a modifier.
    let upper = value.to_uppercase();
    let function = upper
        .strip_prefix('F')
        .and_then(|v| v.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    if !function && !value.contains('+') {
        return Err("output_invalid_shortcut".into());
    }
    Ok(Some(parsed))
}
fn register(app: &tauri::AppHandle, value: &str) -> Result<(), String> {
    let Some(key) = shortcut(value)? else {
        return Ok(());
    };
    app.global_shortcut()
        .on_shortcut(key, move |app, _, event| {
            if event.state() != ShortcutState::Released {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                // Release modifiers before Unicode injection; keep the destination focused.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let state = app.state::<AppState>();
                let result = match state
                    .settings
                    .preferences()
                    .translation_output
                    .shortcut_action
                {
                    ShortcutAction::Copy => copy(&app, &state.session),
                    ShortcutAction::Insert => {
                        crate::microphone_input::insert_after(
                            state.session.clone(),
                            std::time::Duration::ZERO,
                        )
                        .await
                    }
                };
                feedback(&app, result);
            });
        })
        .map_err(|_| "output_shortcut_conflict".into())
}
#[tauri::command]
pub async fn translation_output_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: TranslationOutput,
    api_key: Option<String>,
    delete_key: bool,
) -> Result<OutputSnapshot, String> {
    if !cfg!(target_os = "windows") {
        return Err("output_platform_unsupported".into());
    }
    settings.validate()?;
    shortcut(&settings.shortcut)?;
    let _guard = MUTATION.lock().await;
    let old = state.settings.preferences().translation_output;
    let key = api_key.as_deref().map(str::trim).filter(|k| !k.is_empty());
    if (settings.speak_system || settings.speak_microphone) && delete_key {
        return Err("output_key_required".into());
    }
    if (settings.speak_system || settings.speak_microphone)
        && key.is_none()
        && state
            .settings
            .speech_output_key(&settings.speech_provider)?
            .is_none()
    {
        return Err("output_key_required".into());
    }
    if key.is_some_and(|key| key.len() > 4096 || key.chars().any(char::is_control)) {
        return Err("output_invalid_key".into());
    }
    if settings.speech_provider == SpeechProvider::OpenAICompatible
        && old.endpoint != settings.endpoint
        && key.is_none()
    {
        return Err("output_key_required".into());
    }
    let changed = old.shortcut != settings.shortcut;
    if changed {
        register(&app, &settings.shortcut)?;
    }
    let saved = (|| {
        if delete_key {
            state
                .settings
                .save_speech_output_key(&settings.speech_provider, "")?;
        } else if let Some(key) = key {
            state
                .settings
                .save_speech_output_key(&settings.speech_provider, key)?;
        }
        state
            .settings
            .save_preferences(|prefs| prefs.translation_output = settings.clone())
    })();
    if let Err(error) = saved {
        if changed {
            if let Some(key) = shortcut(&settings.shortcut)? {
                let _ = app.global_shortcut().unregister(key);
            }
        }
        return Err(error);
    }
    if changed {
        if let Some(key) = shortcut(&old.shortcut)? {
            let _ = app.global_shortcut().unregister(key);
        }
    }
    REVISION.fetch_add(1, Ordering::SeqCst);
    snapshot(&state.settings)
}
pub fn feedback(app: &tauri::AppHandle, result: Result<(), String>) {
    // Only categorical outcomes cross this event; never include the translated text.
    let code = result.err().unwrap_or_else(|| "output_done".into());
    if let Some(tray) = app.tray_by_id("mimi-tray") {
        let label = if code == "output_done" {
            "Mimi — خروجی ترجمه انجام شد"
        } else {
            "Mimi — خروجی ترجمه انجام نشد؛ تنظیمات خروجی را بررسی کنید"
        };
        let _ = tray.set_tooltip(Some(label));
    }
    let _ = app.emit("translation-output-feedback", code);
}
fn copy(app: &tauri::AppHandle, session: &SessionManager) -> Result<(), String> {
    let candidate = session
        .microphone_insertion_candidate()
        .ok_or("microphone_input_no_final")?;
    app.clipboard()
        .write_text(candidate.1.text)
        .map_err(|_| "output_clipboard_failed".into())
}
#[tauri::command]
pub fn microphone_copy_translation(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    copy(&app, &state.session)
}
#[tauri::command]
pub fn speech_output_devices() -> Result<Vec<crate::speech_playback::OutputDevice>, String> {
    crate::speech_playback::devices()
}
#[tauri::command]
pub async fn speech_output_catalog(
    state: State<'_, AppState>,
    voices: bool,
) -> Result<Vec<service_catalog::CatalogItem>, String> {
    if crate::commands::app_is_ui_test() {
        return Err("output_ui_test".into());
    }
    let prefs = state.settings.preferences();
    let settings = prefs.translation_output;
    let key = state
        .settings
        .speech_output_key(&settings.speech_provider)?
        .ok_or("output_key_required")?;
    let network = ProviderNetwork::resolve(&prefs.network_proxy)
        .map_err(|_| "catalog_network".to_string())?;
    match settings.speech_provider {
        SpeechProvider::ElevenLabs => service_catalog::eleven_catalog(&key, voices, &network).await,
        SpeechProvider::OpenAICompatible if !voices => {
            service_catalog::compatible_models(&settings.endpoint, &key, &network).await
        }
        _ => Err("catalog_manual_voice".into()),
    }
}
#[tauri::command]
pub async fn speech_output_preview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if crate::commands::app_is_ui_test() {
        return Err("output_ui_test".into());
    }
    if state.session.is_active() {
        return Err("output_preview_requires_stop".into());
    }
    let prefs = state.settings.preferences();
    let settings = prefs.translation_output;
    let key = state
        .settings
        .speech_output_key(&settings.speech_provider)?
        .ok_or("output_key_required")?;
    let network = ProviderNetwork::resolve(&prefs.network_proxy)
        .map_err(|_| "catalog_network".to_string())?;
    let revision = REVISION.load(Ordering::SeqCst);
    let interrupted = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if REVISION.load(Ordering::SeqCst) != revision || state.session.is_active() {
                break;
            }
        }
    };
    let pcm = tokio::select! {
        result = service_catalog::synthesize(&settings, &key, "Hello. This is a translation voice test.", &network) => result?,
        _ = interrupted => return Err("output_cancelled".into()),
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let watch = cancelled.clone();
    let session = state.session.clone();
    let watcher = tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if revision != REVISION.load(Ordering::SeqCst) || session.is_active() {
                watch.store(true, Ordering::SeqCst);
                break;
            }
        }
    });
    let result = tokio::task::spawn_blocking(move || {
        crate::speech_playback::play(pcm, &settings.device, cancelled)
    })
    .await
    .map_err(|_| "output_playback_failed".to_string())?;
    watcher.abort();
    feedback(&app, result.clone());
    result
}
#[tauri::command]
pub fn speech_output_cancel() {
    REVISION.fetch_add(1, Ordering::SeqCst);
    crate::microphone_input::cancel();
}

pub fn start(app: &tauri::AppHandle, session: Arc<SessionManager>, settings: Arc<SettingsStore>) {
    if !cfg!(target_os = "windows") || crate::commands::app_is_ui_test() {
        return;
    }
    if let Err(error) = register(app, &settings.preferences().translation_output.shortcut) {
        feedback(app, Err(error));
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut cursor = mimi_runtime::core::translation_output::OutputCursor::default();
        let mut pending = std::collections::VecDeque::new();
        let mut running: Option<(tokio::task::JoinHandle<()>, Arc<AtomicBool>)> = None;
        let mut revision = REVISION.load(Ordering::SeqCst);
        let mut generation = session.output_generation();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let event = session.current_state_event();
            let prefs = settings.preferences();
            let output = prefs.translation_output.clone();
            let next_revision = REVISION.load(Ordering::SeqCst);
            let next_generation = session.output_generation();
            if !matches!(
                event.status,
                crate::session_manager::StatusPayload::Listening
            ) || event.is_paused
                || next_revision != revision
                || next_generation != generation
            {
                if let Some((task, cancel)) = running.take() {
                    cancel.store(true, Ordering::SeqCst);
                    task.abort();
                }
                pending.clear();
            }
            revision = next_revision;
            generation = next_generation;
            if !matches!(
                event.status,
                crate::session_manager::StatusPayload::Listening
            ) || event.is_paused
            {
                continue;
            }
            for final_output in cursor.observe(&event.subtitles, generation) {
                let mic = final_output.source == crate::core::audio_input::AudioSource::Microphone;
                if mic && output.auto_copy {
                    feedback(
                        &app,
                        app.clipboard()
                            .write_text(final_output.text.clone())
                            .map_err(|_| "output_clipboard_failed".into()),
                    );
                }
                if (mic && output.speak_microphone) || (!mic && output.speak_system) {
                    if pending.len() < 4 {
                        pending.push_back(final_output.text);
                    } else {
                        feedback(&app, Err("output_queue_full".into()));
                    }
                }
            }
            if running.as_ref().is_some_and(|(t, _)| t.is_finished()) {
                running = None;
            }
            if running.is_some() {
                continue;
            }
            let Some(text) = pending.pop_front() else {
                continue;
            };
            let key = match settings.speech_output_key(&output.speech_provider) {
                Ok(Some(key)) => key,
                _ => {
                    feedback(&app, Err("output_key_required".into()));
                    continue;
                }
            };
            let network = match ProviderNetwork::resolve(&prefs.network_proxy) {
                Ok(n) => n,
                Err(_) => {
                    feedback(&app, Err("catalog_network".into()));
                    continue;
                }
            };
            let handle = app.clone();
            let cancelled = Arc::new(AtomicBool::new(false));
            let cancel = cancelled.clone();
            let task = tokio::spawn(async move {
                let result = async {
                    let pcm = service_catalog::synthesize(&output, &key, &text, &network).await?;
                    if cancel.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    tokio::task::spawn_blocking(move || {
                        crate::speech_playback::play(pcm, &output.device, cancel)
                    })
                    .await
                    .map_err(|_| "output_playback_failed".to_string())?
                }
                .await;
                feedback(&handle, result);
            });
            running = Some((task, cancelled));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_action_never_grabs_plain_text_keys() {
        assert!(shortcut("").unwrap().is_none());
        assert!(shortcut("F8").is_ok());
        assert!(shortcut("Ctrl+Shift+F9").is_ok());
        assert!(shortcut("A").is_err());
        assert!(shortcut("F99").is_err());
        assert!(shortcut("not a key").is_err());
    }
}
