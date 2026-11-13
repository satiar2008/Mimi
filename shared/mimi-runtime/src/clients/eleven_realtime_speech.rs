//! Official model-specific ElevenLabs text-input streaming, one connection per reader.
use super::{
    provider_network::{self, ProviderNetwork},
    speech_stream::StreamMetrics,
};
use crate::core::translation_output::{valid_id, SpeechSettings};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

fn dialogue(model: &str) -> bool {
    model.starts_with("eleven_v3") || model.starts_with("eleven_v4")
}
fn setup(settings: &SpeechSettings) -> Result<(String, Value), String> {
    settings.validate(false)?;
    if !valid_id(&settings.model) || !valid_id(&settings.voice) || settings.voice.contains('/') {
        return Err("output_invalid_settings".into());
    }
    let ttd = dialogue(&settings.model);
    let mut url = url::Url::parse(if ttd {
        "wss://api.elevenlabs.io/v1/text-to-dialogue/stream-input"
    } else {
        "wss://api.elevenlabs.io"
    })
    .unwrap();
    if !ttd {
        url.set_path(&format!(
            "/v1/text-to-speech/{}/stream-input",
            settings.voice
        ));
    }
    url.query_pairs_mut()
        .append_pair("model_id", &settings.model)
        .append_pair("output_format", "pcm_24000");
    let mut voice = json!({"stability":settings.stability,"similarity_boost":settings.similarity,"speed":settings.speed});
    if settings.model.starts_with("eleven_v4") {
        voice.as_object_mut().unwrap().remove("speed");
    }
    let initial = if ttd {
        json!({"voices":[settings.voice],"voice_settings":voice})
    } else {
        json!({"text":" ","voice_settings":voice,"generation_config":{"chunk_length_schedule":[50,80,120,160]}})
    };
    Ok((url.into(), initial))
}
fn text_frame(settings: &SpeechSettings, text: &str) -> Value {
    if dialogue(&settings.model) {
        json!({"inputs":[{"text":format!("{text} "),"voice_id":settings.voice,"new_turn":false}]})
    } else {
        json!({"text":format!("{text} ")})
    }
}
fn audio_frame(value: &Value) -> Result<Option<Vec<u8>>, String> {
    if value.get("error").is_some_and(|v| !v.is_null()) {
        return Err("output_stream_failed".into());
    }
    let Some(audio) = value["audio"].as_str().filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio)
        .map_err(|_| "output_invalid_audio")?;
    if !bytes.len().is_multiple_of(2)
        || bytes.len() > 768_000
        || bytes.starts_with(b"RIFF")
        || bytes.starts_with(b"ID3")
    {
        return Err("output_invalid_audio".into());
    }
    Ok(Some(bytes))
}
pub async fn speak(
    settings: &SpeechSettings,
    key: &str,
    network: &ProviderNetwork,
    mut text: mpsc::Receiver<String>,
    pcm: &mpsc::Sender<Vec<u8>>,
    cancel: &Arc<AtomicBool>,
    mut progress: impl FnMut(&StreamMetrics) + Send,
) -> Result<StreamMetrics, String> {
    let (url, initial) = setup(settings)?;
    let mut request = url.into_client_request().map_err(|_| "catalog_network")?;
    request.headers_mut().insert(
        "xi-api-key",
        HeaderValue::from_str(key).map_err(|_| "output_invalid_key")?,
    );
    let started = Instant::now();
    let (mut socket, _) = tokio::select! {
        r=provider_network::websocket_with_message_limit(request,network,1024*1024)=>r.map_err(|error|match error {tokio_tungstenite::tungstenite::Error::Http(response)=>format!("catalog_http_{}",response.status().as_u16()),_=>"catalog_network".into()})?,
        _=cancelled(cancel)=>return Err("output_cancelled".into()),
    };
    send_frame(
        &mut socket,
        Message::Text(initial.to_string().into()),
        cancel,
    )
    .await?;
    let mut closing = false;
    let mut metrics = StreamMetrics::default();
    let mut last_progress = Instant::now();
    let mut keep_alive = tokio::time::interval(Duration::from_secs(10));
    keep_alive.tick().await;
    loop {
        tokio::select! {
            _=cancelled(cancel)=>{let _=tokio::time::timeout(Duration::from_millis(250),socket.close(None)).await;return Err("output_cancelled".into());},
            frame=socket.next()=>{
                match frame {
                    Some(Ok(Message::Text(raw)))=>{
                        let value:Value=serde_json::from_str(&raw).map_err(|_|"output_invalid_audio")?;
                        if let Some(bytes)=audio_frame(&value)? {
                            if metrics.network_chunks==0{metrics.first_network_ms=started.elapsed().as_millis() as u64;metrics.first_chunk_ms=metrics.first_network_ms;}
                            metrics.network_chunks+=1;metrics.bytes=metrics.bytes.saturating_add(bytes.len());metrics.last_network_ms=started.elapsed().as_millis() as u64;
                            for part in bytes.chunks(4096){tokio::select!{r=pcm.send(part.to_vec())=>r.map_err(|_|"output_cancelled")?,_=cancelled(cancel)=>return Err("output_cancelled".into())}metrics.chunks+=1;}
                            metrics.last_chunk_ms=started.elapsed().as_millis() as u64;
                            if metrics.network_chunks==1||last_progress.elapsed()>=Duration::from_secs(1){progress(&metrics);last_progress=Instant::now();}
                        }
                        if value["is_final"]==true||value["isFinal"]==true{
                            if !closing{return Err("output_stream_failed".into());}
                            if metrics.bytes==0{return Err("output_invalid_audio".into());}
                            progress(&metrics);return Ok(metrics);
                        }
                    },
                    Some(Ok(Message::Ping(bytes)))=>send_frame(&mut socket,Message::Pong(bytes),cancel).await?,
                    Some(Ok(Message::Pong(_)))=>{},
                    Some(Ok(Message::Close(_)))|None=>return Err("output_stream_failed".into()),
                    Some(Ok(_))=>return Err("output_invalid_audio".into()),
                    Some(Err(_))=>return Err("catalog_network".into()),
                }
            },
            next=text.recv(),if !closing=>{
                if let Some(next)=next {
                    if next.is_empty()||next.chars().count()>4000{return Err("output_invalid_text".into());}
                    send_frame(&mut socket,Message::Text(text_frame(settings,&next).to_string().into()),cancel).await?;
                    // Each input is a sentence or a bounded long-sentence fallback.
                    send_frame(&mut socket,Message::Text(json!({"flush":true}).to_string().into()),cancel).await?;
                }else{
                    let close=if dialogue(&settings.model){json!({"close_socket":true})}else{json!({"text":""})};
                    send_frame(&mut socket,Message::Text(close.to_string().into()),cancel).await?;closing=true;
                }
            },
            _=keep_alive.tick(),if !closing=>{
                let keep=if dialogue(&settings.model){json!({"keep_alive":true})}else{json!({"text":" "})};
                send_frame(&mut socket,Message::Text(keep.to_string().into()),cancel).await?;
            },
            _=tokio::time::sleep(Duration::from_secs(20)),if closing=>return Err("output_stream_timeout".into()),
        }
    }
}
async fn send_frame<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    frame: Message,
    cancel: &AtomicBool,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::select! {
        result=tokio::time::timeout(Duration::from_secs(10),socket.send(frame))=>result.map_err(|_|"output_stream_timeout")?.map_err(|_|"catalog_network".into()),
        _=cancelled(cancel)=>Err("output_cancelled".into()),
    }
}
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_selects_the_official_protocol_without_changing_model_or_voice() {
        let mut settings = SpeechSettings {
            model: "eleven_v3".into(),
            voice: "voice-a".into(),
            ..Default::default()
        };
        let (url, initial) = setup(&settings).unwrap();
        assert!(url.contains("/text-to-dialogue/stream-input?"));
        assert!(url.contains("model_id=eleven_v3"));
        assert_eq!(initial["voices"][0], "voice-a");
        assert_eq!(
            text_frame(&settings, "سلام Mimi")["inputs"][0]["text"],
            "سلام Mimi "
        );
        assert!(!initial.to_string().contains("key"));
        settings.model = "eleven_v4".into();
        settings.speed = 1.4;
        let (url, initial) = setup(&settings).unwrap();
        assert!(url.contains("model_id=eleven_v4"));
        assert!(initial["voice_settings"].get("speed").is_none());
        assert_eq!(settings.local_speed(), 1.4);
        settings.model = "eleven_flash_v2_5".into();
        settings.speed = 1.2;
        let (url, initial) = setup(&settings).unwrap();
        assert!(url.contains("/text-to-speech/voice-a/stream-input?"));
        assert_eq!(initial["text"], " ");
        assert_eq!(initial["voice_settings"]["speed"], 1.2);
        assert_eq!(settings.local_speed(), 1.0);
    }
    #[test]
    fn only_bounded_pcm_is_forwarded_and_provider_error_text_is_private() {
        assert_eq!(
            audio_frame(&json!({"audio":"AAABAA=="})).unwrap().unwrap(),
            [0, 0, 1, 0]
        );
        assert_eq!(
            audio_frame(&json!({"error":"private provider message"})).unwrap_err(),
            "output_stream_failed"
        );
        assert!(audio_frame(&json!({"audio":"AA=="})).is_err());
        assert!(audio_frame(&json!({"audio":"UklGRg=="})).is_err());
    }
}
