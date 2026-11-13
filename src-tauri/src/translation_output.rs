//! Desktop output adapter: explicit global action, opt-in clipboard and bounded speech.
use crate::{commands::AppState, session_manager::SessionManager, settings_store::SettingsStore};
use mimi_runtime::{
    clients::{
        eleven_realtime_speech, provider_network::ProviderNetwork, service_catalog, speech_stream,
    },
    core::{
        credentials::ProviderCredentials,
        provider::ProviderKind,
        translation_output::{ShortcutAction, SpeechProvider, SpeechSettings, TranslationOutput},
    },
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
    system_key_present: bool,
    main_service: Option<String>,
}
fn snapshot(settings: &SettingsStore) -> Result<OutputSnapshot, String> {
    let mut value = settings.preferences().translation_output;
    value.system_speech = Some(value.incoming_speech());
    let key_present = destination(settings, value.microphone_speech()).is_ok();
    let system_key_present = destination(settings, value.incoming_speech()).is_ok();
    let main_service = settings
        .active_profile()
        .ok()
        .and_then(|p| match p.provider {
            ProviderKind::GoogleGeminiLive => Some("Gemini".into()),
            ProviderKind::OpenAIRealtime => Some("OpenAI".into()),
            ProviderKind::CustomOpenAIASR => Some("OpenAI-compatible API".into()),
            _ => None,
        });
    Ok(OutputSnapshot {
        settings: value,
        key_present,
        system_key_present,
        main_service,
    })
}

struct SpeechDestination {
    speech: SpeechSettings,
    key: String,
    network: ProviderNetwork,
}
fn destination(
    store: &SettingsStore,
    mut speech: SpeechSettings,
) -> Result<SpeechDestination, String> {
    let (key, proxy) = if speech.speech_provider == SpeechProvider::MainService {
        let profile = store.active_profile()?;
        if !matches!(
            profile.provider,
            ProviderKind::GoogleGeminiLive
                | ProviderKind::OpenAIRealtime
                | ProviderKind::CustomOpenAIASR
        ) {
            return Err("output_main_service_unsupported".into());
        }
        let config = store.configuration_for_speech_probe(&profile)?;
        let api_key = match config.credentials {
            ProviderCredentials::ApiKey { api_key }
                if profile.provider == ProviderKind::GoogleGeminiLive =>
            {
                speech.speech_provider = SpeechProvider::Gemini;
                api_key
            }
            ProviderCredentials::ApiKey { api_key }
                if profile.provider == ProviderKind::OpenAIRealtime =>
            {
                speech.speech_provider = SpeechProvider::OpenAICompatible;
                speech.endpoint = "https://api.openai.com/v1/audio/speech".into();
                api_key
            }
            ProviderCredentials::CustomSpeech {
                endpoint, api_key, ..
            } if profile.provider == ProviderKind::CustomOpenAIASR => {
                speech.speech_provider = SpeechProvider::OpenAICompatible;
                speech.endpoint = service_catalog::compatible_speech_endpoint(&endpoint)?;
                api_key
            }
            _ => return Err("output_key_required".into()),
        };
        (api_key, config.network_proxy)
    } else {
        if speech.speech_provider == SpeechProvider::Gemini {
            return Err("output_main_service_unsupported".into());
        }
        let key = store
            .reader_key(&speech)?
            .filter(|s| !s.is_empty())
            .ok_or("output_key_required")?;
        (key, store.preferences().network_proxy)
    };
    let network = ProviderNetwork::resolve(&proxy).map_err(|_| "catalog_network".to_string())?;
    Ok(SpeechDestination {
        speech,
        key,
        network,
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
    incoming: Option<bool>,
) -> Result<OutputSnapshot, String> {
    if !cfg!(target_os = "windows") {
        return Err("output_platform_unsupported".into());
    }
    settings.validate()?;
    shortcut(&settings.shortcut)?;
    let _guard = MUTATION.lock().await;
    let old = state.settings.preferences().translation_output;
    if state.session.is_active()
        && (old.microphone_speech() != settings.microphone_speech()
            || old.incoming_speech() != settings.incoming_speech()
            || old.speak_system != settings.speak_system
            || old.speak_microphone != settings.speak_microphone
            || old.duck_system_audio != settings.duck_system_audio
            || old.original_volume != settings.original_volume)
    {
        return Err("output_settings_require_stop".into());
    }
    let selected = if incoming.unwrap_or(false) {
        settings.incoming_speech()
    } else {
        settings.microphone_speech()
    };
    let previous = if incoming.unwrap_or(false) {
        old.incoming_speech()
    } else {
        old.microphone_speech()
    };
    let key = api_key.as_deref().map(str::trim).filter(|k| !k.is_empty());
    if selected.speech_provider == SpeechProvider::Gemini
        || (selected.speech_provider == SpeechProvider::MainService
            && (key.is_some() || delete_key))
    {
        return Err("output_invalid_settings".into());
    }
    if (settings.speak_system || settings.speak_microphone) && delete_key {
        return Err("output_key_required".into());
    }
    for (enabled, speech) in [
        (settings.speak_system, settings.incoming_speech()),
        (settings.speak_microphone, settings.microphone_speech()),
    ] {
        if enabled
            && !(key.is_some()
                && speech.speech_provider == selected.speech_provider
                && (speech.speech_provider != SpeechProvider::OpenAICompatible
                    || speech.endpoint == selected.endpoint))
        {
            destination(&state.settings, speech)?;
        }
    }
    if key.is_some_and(|key| key.len() > 4096 || key.chars().any(char::is_control)) {
        return Err("output_invalid_key".into());
    }
    if selected.speech_provider == SpeechProvider::OpenAICompatible
        && previous.endpoint != selected.endpoint
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
            state.settings.save_reader_key(&selected, "")?;
        } else if let Some(key) = key {
            state.settings.save_reader_key(&selected, key)?;
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
    incoming: Option<bool>,
) -> Result<Vec<service_catalog::CatalogItem>, String> {
    if crate::commands::app_is_ui_test() {
        return Err("output_ui_test".into());
    }
    let output = state.settings.preferences().translation_output;
    let SpeechDestination {
        speech: settings,
        key,
        network,
    } = destination(
        &state.settings,
        if incoming.unwrap_or(false) {
            output.incoming_speech()
        } else {
            output.microphone_speech()
        },
    )?;
    match settings.speech_provider {
        SpeechProvider::ElevenLabs => service_catalog::eleven_catalog(&key, voices, &network).await,
        SpeechProvider::OpenAICompatible if !voices => {
            service_catalog::compatible_models(&settings.endpoint, &key, &network).await
        }
        SpeechProvider::Gemini if !voices => {
            service_catalog::gemini_speech_models(&key, &network).await
        }
        SpeechProvider::Gemini => Ok([
            "Kore", "Puck", "Charon", "Aoede", "Fenrir", "Leda", "Orus", "Zephyr",
        ]
        .into_iter()
        .map(|v| service_catalog::CatalogItem {
            id: v.into(),
            name: v.into(),
            compatible: true,
        })
        .collect()),
        _ => Err("catalog_manual_voice".into()),
    }
}
#[tauri::command]
pub async fn speech_output_preview(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    incoming: Option<bool>,
) -> Result<crate::streaming_playback::PlaybackMetrics, String> {
    if crate::commands::app_is_ui_test() {
        return Err("output_ui_test".into());
    }
    if state.session.is_active() {
        return Err("output_preview_requires_stop".into());
    }
    let output = state.settings.preferences().translation_output;
    let incoming = incoming.unwrap_or(false);
    let SpeechDestination {
        speech: settings,
        key,
        network,
    } = destination(
        &state.settings,
        if incoming {
            output.incoming_speech()
        } else {
            output.microphone_speech()
        },
    )?;
    let revision = REVISION.load(Ordering::SeqCst);
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
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tx.send("سلام. این آزمایش خواندن تدریجی ترجمه است. صدا هم‌زمان با دریافت پخش می‌شود و منتظر پایان فایل نمی‌ماند.".into()).await.map_err(|_|"output_cancelled")?;
    drop(tx);
    let result = stream_reader(
        app.clone(),
        SpeechDestination {
            speech: settings,
            key,
            network,
        },
        rx,
        cancelled,
        incoming && output.duck_system_audio,
        output.original_volume as f32,
        incoming,
    )
    .await;
    watcher.abort();
    feedback(&app, result.as_ref().map(|_| ()).map_err(Clone::clone));
    result
}
#[tauri::command]
pub fn speech_output_cancel() {
    REVISION.fetch_add(1, Ordering::SeqCst);
    crate::microphone_input::cancel();
}

async fn stream_reader(
    app: tauri::AppHandle,
    destination: SpeechDestination,
    mut phrases: tokio::sync::mpsc::Receiver<String>,
    cancel: Arc<AtomicBool>,
    duck: bool,
    volume: f32,
    incoming: bool,
) -> Result<crate::streaming_playback::PlaybackMetrics, String> {
    let started = std::time::Instant::now();
    let (pcm_tx, mut raw_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(12);
    let (tempo_tx, pcm_rx) = tokio::sync::mpsc::channel(12);
    let tempo_cancel = cancel.clone();
    let tempo_app = app.clone();
    let local_speed = destination.speech.local_speed();
    let tempo = tokio::spawn(async move {
        let mut processor = mimi_runtime::audio::speech_tempo::SpeechTempo::new(local_speed)?;
        let mut received = 0usize;
        let mut emitted = 0usize;
        let mut reported = std::time::Instant::now();
        loop {
            let packet = tokio::select! {
                packet=raw_rx.recv()=>packet,
                _=async { while !tempo_cancel.load(Ordering::SeqCst) { tokio::time::sleep(std::time::Duration::from_millis(20)).await; } }=>return Err("output_cancelled"),
            };
            let finished = packet.is_none();
            let output = if let Some(packet) = packet {
                received += packet.len() / 2;
                processor.push(&packet)?
            } else {
                processor.finish()
            };
            for packet in output {
                emitted += packet.len() / 2;
                tokio::select! {
                    result=tempo_tx.send(packet)=>result.map_err(|_|"output_cancelled")?,
                    _=async { while !tempo_cancel.load(Ordering::SeqCst) { tokio::time::sleep(std::time::Duration::from_millis(20)).await; } }=>return Err("output_cancelled"),
                }
            }
            if finished || reported.elapsed() >= std::time::Duration::from_secs(1) {
                let _=tempo_app.emit("speech-stream-tempo",serde_json::json!({"incoming":incoming,"localSpeed":local_speed,"inputSamples":received,"outputSamples":emitted,"finished":finished}));
                reported = std::time::Instant::now();
            }
            if finished {
                return Ok(());
            }
        }
    });
    let device = destination.speech.device.clone();
    let playback_cancel = cancel.clone();
    let (first_tx, first_rx) = tokio::sync::oneshot::channel();
    let observer = app.clone();
    tokio::spawn(async move {
        if let Ok(ms) = first_rx.await {
            let _ = observer.emit(
                "speech-stream-started",
                serde_json::json!({"incoming":incoming,"firstFrameMs":ms}),
            );
        }
    });
    let mut playback = tokio::task::spawn_blocking(move || {
        crate::streaming_playback::play(
            pcm_rx,
            device,
            playback_cancel,
            duck,
            volume,
            started,
            Some(first_tx),
        )
    });
    let producer_cancel = cancel.clone();
    let mut producer = tokio::spawn(async move {
        let synthesized = async {
        if destination.speech.speech_provider == SpeechProvider::ElevenLabs {
            return eleven_realtime_speech::speak(&destination.speech,&destination.key,&destination.network,phrases,&pcm_tx,&producer_cancel,|metrics|{
                let _=app.emit("speech-stream-metrics",serde_json::json!({"incoming":incoming,"transport":"websocket","requestStartedMs":0,"firstChunkMs":metrics.first_chunk_ms,"lastChunkMs":metrics.last_chunk_ms,"networkChunks":metrics.network_chunks,"firstNetworkMs":metrics.first_network_ms,"lastNetworkMs":metrics.last_network_ms,"chunks":metrics.chunks,"bytes":metrics.bytes}));
            }).await.map(|_|());
        }
        let mut previous = String::new();
        while let Some(text) = phrases.recv().await {
            if producer_cancel.load(Ordering::SeqCst) {
                return Err("output_cancelled".to_owned());
            }
            let request_started = started.elapsed().as_millis() as u64;
            let metrics = speech_stream::synthesize_stream(
                &destination.speech,
                &destination.key,
                &text,
                &previous,
                &destination.network,
                &pcm_tx,
                &producer_cancel,
            )
            .await?;
            // Timing and counts only: no text, voice identifiers or credentials.
            let _=app.emit("speech-stream-metrics",serde_json::json!({"incoming":incoming,"requestStartedMs":request_started,"firstChunkMs":metrics.first_chunk_ms,"lastChunkMs":metrics.last_chunk_ms,"networkChunks":metrics.network_chunks,"firstNetworkMs":metrics.first_network_ms,"lastNetworkMs":metrics.last_network_ms,"chunks":metrics.chunks,"bytes":metrics.bytes}));
            previous = text
                .chars()
                .rev()
                .take(300)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
        }
        Ok::<(), String>(())
        }.await;
        if synthesized.is_err() {
            producer_cancel.store(true, Ordering::SeqCst);
        }
        drop(pcm_tx);
        let adjusted = tempo
            .await
            .map_err(|_| "output_playback_failed".to_owned())?;
        synthesized?;
        adjusted.map_err(str::to_owned)
    });
    tokio::select! {
        result=&mut producer=>{
            let result=result.map_err(|_|"output_stream_failed".to_owned()).and_then(|v|v);
            if result.is_err(){cancel.store(true,Ordering::SeqCst);}
            let played=playback.await.map_err(|_|"output_playback_failed".to_owned())?;
            result?;played
        },
        result=&mut playback=>{
            let played=result.map_err(|_|"output_playback_failed".to_owned())?;
            if played.is_err(){cancel.store(true,Ordering::SeqCst);producer.abort();return played;}
            if cancel.load(Ordering::SeqCst){producer.abort();return Err("output_cancelled".into());}
            producer.await.map_err(|_|"output_stream_failed".to_owned())??;
            played
        }
    }
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
        let mut speech_cursor = mimi_runtime::core::streaming_speech::SpeechCursor::default();
        let clock = std::time::Instant::now();
        type Running = Option<(
            tokio::task::JoinHandle<()>,
            Arc<AtomicBool>,
            tokio::sync::mpsc::Sender<String>,
        )>;
        let mut running: [Running; 2] = [None, None];
        let mut blocked = [false; 2];
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
                for slot in &mut running {
                    if let Some((task, cancel, _)) = slot.take() {
                        cancel.store(true, Ordering::SeqCst);
                        task.abort();
                    }
                }
                blocked = [false; 2];
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
            }
            for lane in 0..2 {
                if running[lane]
                    .as_ref()
                    .is_some_and(|(t, _, _)| t.is_finished())
                {
                    running[lane] = None;
                    blocked[lane] = true;
                }
            }
            for phrase in speech_cursor.observe(
                &event.subtitles,
                generation,
                clock.elapsed().as_millis() as u64,
                [output.speak_system, output.speak_microphone],
            ) {
                let lane =
                    usize::from(phrase.source == crate::core::audio_input::AudioSource::Microphone);
                if blocked[lane] {
                    continue;
                }
                if running[lane].is_none() {
                    let SpeechDestination {
                        speech,
                        key,
                        network,
                    } = match destination(
                        &settings,
                        if lane == 0 {
                            output.incoming_speech()
                        } else {
                            output.microphone_speech()
                        },
                    ) {
                        Ok(d) => d,
                        Err(error) => {
                            feedback(&app, Err(error));
                            blocked[lane] = true;
                            continue;
                        }
                    };
                    let handle = app.clone();
                    let cancelled = Arc::new(AtomicBool::new(false));
                    let cancel = cancelled.clone();
                    let duck = lane == 0 && output.duck_system_audio;
                    let volume = output.original_volume as f32;
                    let (tx, rx) = tokio::sync::mpsc::channel(8);
                    let task = tokio::spawn(async move {
                        let result = stream_reader(
                            handle.clone(),
                            SpeechDestination {
                                speech,
                                key,
                                network,
                            },
                            rx,
                            cancel.clone(),
                            duck,
                            volume,
                            lane == 0,
                        )
                        .await;
                        if let Err(error) = result {
                            if error != "output_cancelled" {
                                feedback(&handle, Err(error));
                            }
                        }
                    });
                    running[lane] = Some((task, cancelled, tx));
                }
                let preview = phrase.preview;
                if running[lane]
                    .as_ref()
                    .is_some_and(|(_, _, tx)| tx.try_send(phrase.text).is_err())
                {
                    feedback(&app, Err("output_queue_full".into()));
                    blocked[lane] = true;
                    if let Some((task, cancel, _)) = running[lane].take() {
                        cancel.store(true, Ordering::SeqCst);
                        task.abort();
                    }
                } else {
                    let _ = app.emit(
                        "speech-phrase-queued",
                        serde_json::json!({"incoming":lane==0,"preview":preview}),
                    );
                }
            }
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
