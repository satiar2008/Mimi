//! HTTP audio is forwarded under backpressure, never collected into a whole utterance.
use super::{provider_network::ProviderNetwork, service_catalog};
use crate::core::translation_output::{speech_endpoint, valid_id, SpeechProvider, SpeechSettings};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

#[derive(Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamMetrics {
    pub first_network_ms: u64,
    pub last_network_ms: u64,
    pub network_chunks: usize,
    pub first_chunk_ms: u64,
    pub last_chunk_ms: u64,
    pub chunks: usize,
    pub bytes: usize,
}
#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Pcm,
    Gemini,
    Interaction,
}
fn request(
    settings: &SpeechSettings,
    text: &str,
    previous: &str,
) -> Result<(url::Url, Value, Encoding), String> {
    settings.validate(false)?;
    if !valid_id(&settings.model)
        || !valid_id(&settings.voice)
        || settings.voice.contains('/')
        || text.is_empty()
        || text.chars().count() > 4000
    {
        return Err("output_invalid_text".into());
    }
    Ok(match settings.speech_provider {
        SpeechProvider::ElevenLabs=>(url::Url::parse(&format!("https://api.elevenlabs.io/v1/text-to-speech/{}/stream?output_format=pcm_24000",settings.voice)).unwrap(),json!({"text":text,"previous_text":previous,"model_id":settings.model,"voice_settings":{"stability":settings.stability,"similarity_boost":settings.similarity,"speed":settings.speed}}),Encoding::Pcm),
        SpeechProvider::OpenAICompatible=>(speech_endpoint(&settings.endpoint)?,json!({"input":text,"model":settings.model,"voice":settings.voice,"speed":settings.speed,"response_format":"pcm"}),Encoding::Pcm),
        SpeechProvider::Gemini if settings.model.starts_with("gemini-3.8-")=>(url::Url::parse("https://generativelanguage.googleapis.com/v1beta/interactions").unwrap(),json!({"model":settings.model,"input":[{"type":"user_input","content":[{"type":"text","text":text}]}],"response_format":{"type":"audio","mime_type":"audio/l16","sample_rate":24000},"generation_config":{"speech_config":[{"voice":settings.voice}]},"stream":true,"store":false}),Encoding::Interaction),
        SpeechProvider::Gemini=>(url::Url::parse(&format!("https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",settings.model)).map_err(|_|"output_invalid_settings")?,service_catalog::gemini_speech_payload(settings,text)?,Encoding::Gemini),
        SpeechProvider::MainService=>return Err("output_main_service_unsupported".into()),
    })
}
pub async fn synthesize_stream(
    settings: &SpeechSettings,
    key: &str,
    text: &str,
    previous: &str,
    network: &ProviderNetwork,
    tx: &mpsc::Sender<Vec<u8>>,
    cancel: &Arc<AtomicBool>,
) -> Result<StreamMetrics, String> {
    let (endpoint, payload, encoding) = request(settings, text, previous)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = network
        .http_client_builder(&endpoint)
        .map_err(|_| "catalog_network")?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(75))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "catalog_network")?;
    let request = http.post(endpoint).json(&payload);
    let request = match settings.speech_provider {
        SpeechProvider::ElevenLabs => request.header("xi-api-key", key),
        SpeechProvider::Gemini => request.header("x-goog-api-key", key),
        _ if !key.is_empty() => request.bearer_auth(key),
        _ => request,
    };
    let started = Instant::now();
    let mut response = tokio::select! { r=request.send()=>r.map_err(|_|"catalog_network")?, _=cancelled(cancel)=>return Err("output_cancelled".into()) };
    if !response.status().is_success() {
        return Err(format!("catalog_http_{}", response.status().as_u16()));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if (encoding == Encoding::Pcm
        && ![
            "audio/pcm",
            "audio/l16",
            "audio/raw",
            "application/octet-stream",
        ]
        .iter()
        .any(|v| mime.starts_with(v)))
        || (encoding != Encoding::Pcm && !mime.starts_with("text/event-stream"))
    {
        return Err("output_invalid_audio".into());
    }
    let mut metrics = StreamMetrics::default();
    let mut sse = Sse::default();
    let mut odd = None;
    let mut prefix = Vec::new();
    loop {
        let chunk = tokio::select! { r=tokio::time::timeout(Duration::from_secs(10),response.chunk())=>r.map_err(|_|"output_stream_timeout")?.map_err(|_|"catalog_network")?, _=cancelled(cancel)=>return Err("output_cancelled".into()) };
        let Some(chunk) = chunk else {
            break;
        };
        if metrics.network_chunks == 0 {
            metrics.first_network_ms = started.elapsed().as_millis() as u64;
        }
        metrics.last_network_ms = started.elapsed().as_millis() as u64;
        metrics.network_chunks += 1;
        let packets = if encoding == Encoding::Pcm {
            vec![chunk.to_vec()]
        } else {
            sse.push(&chunk, encoding)?
        };
        for mut packet in packets {
            if packet.is_empty() {
                continue;
            }
            if metrics.bytes == 0 {
                prefix.extend_from_slice(&packet);
                if prefix.len() < 8 {
                    continue;
                }
                packet = std::mem::take(&mut prefix);
            }
            if metrics.bytes == 0
                && (packet.starts_with(b"RIFF")
                    || packet.starts_with(b"ID3")
                    || packet.starts_with(b"OggS")
                    || packet.starts_with(b"fLaC"))
            {
                return Err("output_invalid_audio".into());
            }
            metrics.bytes += packet.len();
            if metrics.bytes > 24_000 * 2 * 60 {
                return Err("output_audio_limit".into());
            }
            let mut framed = Vec::with_capacity(packet.len() + 1);
            if let Some(byte) = odd.take() {
                framed.push(byte);
            }
            framed.extend_from_slice(&packet);
            if framed.len() % 2 != 0 {
                odd = framed.pop();
            }
            for part in framed.chunks(4096) {
                if part.is_empty() {
                    continue;
                }
                tokio::select! { r=tx.send(part.to_vec())=>r.map_err(|_|"output_cancelled")?, _=cancelled(cancel)=>return Err("output_cancelled".into()) }
                if metrics.chunks == 0 {
                    metrics.first_chunk_ms = started.elapsed().as_millis() as u64;
                }
                metrics.chunks += 1;
                metrics.last_chunk_ms = started.elapsed().as_millis() as u64;
            }
        }
    }
    if metrics.bytes == 0
        || !prefix.is_empty()
        || odd.is_some()
        || (encoding != Encoding::Pcm && (!sse.pending.is_empty() || !sse.complete))
    {
        return Err("output_invalid_audio".into());
    }
    Ok(metrics)
}
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[derive(Default)]
struct Sse {
    pending: Vec<u8>,
    data: String,
    complete: bool,
}
impl Sse {
    fn push(&mut self, bytes: &[u8], encoding: Encoding) -> Result<Vec<Vec<u8>>, String> {
        if self.pending.len() + self.data.len() + bytes.len() > 4 * 1024 * 1024 {
            return Err("output_audio_limit".into());
        }
        self.pending.extend_from_slice(bytes);
        let mut result = Vec::new();
        while let Some(index) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=index).collect();
            let line = std::str::from_utf8(&line)
                .map_err(|_| "output_invalid_audio")?
                .trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if self.data.is_empty() {
                    continue;
                }
                let data = std::mem::take(&mut self.data);
                if data.trim() == "[DONE]" {
                    self.complete = true;
                    continue;
                }
                let event: Value =
                    serde_json::from_str(&data).map_err(|_| "output_invalid_audio")?;
                if event.get("error").is_some()
                    || matches!(
                        event["event_type"].as_str(),
                        Some("error" | "interaction.failed" | "interaction.cancelled")
                    )
                {
                    return Err("output_stream_failed".into());
                }
                if encoding == Encoding::Gemini {
                    if event["candidates"][0]["finishReason"]
                        .as_str()
                        .is_some_and(|s| s == "STOP")
                    {
                        self.complete = true;
                    }
                    if event["candidates"][0]["content"]["parts"]
                        .as_array()
                        .is_some_and(|p| p.iter().any(|v| v.get("inlineData").is_some()))
                    {
                        result.push(service_catalog::gemini_pcm(&event)?);
                    }
                } else {
                    if event["event_type"] == "interaction.complete"
                        || event["event_type"] == "interaction.completed"
                        || event["event_type"] == "interaction.stop"
                    {
                        self.complete = true;
                    }
                    if event["event_type"] == "step.stop" {
                        self.complete = true;
                    }
                    let audio = &event["delta"];
                    if event["event_type"] == "step.delta" && audio["type"] == "audio" {
                        if audio["mime_type"]
                            .as_str()
                            .is_some_and(|v| !v.starts_with("audio/l16"))
                            || audio["sample_rate"].as_u64().is_some_and(|r| r != 24000)
                            || audio["channels"].as_u64().is_some_and(|n| n != 1)
                        {
                            return Err("output_invalid_audio".into());
                        }
                        if let Some(data) = audio["data"].as_str() {
                            result.push(
                                base64::engine::general_purpose::STANDARD
                                    .decode(data)
                                    .map_err(|_| "output_invalid_audio")?,
                            );
                        }
                    }
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data.trim_start());
            }
        }
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_routes_keep_selected_models_and_disable_interaction_storage() {
        let mut settings = SpeechSettings {
            model: "eleven_v3".into(),
            voice: "voice-a".into(),
            ..Default::default()
        };
        let (url, body, _) = request(&settings, "سلام Mimi", "previous words").unwrap();
        assert!(url.path().ends_with("/stream"));
        assert_eq!(body["model_id"], "eleven_v3");
        settings.speech_provider = SpeechProvider::Gemini;
        settings.model = "gemini-3.8-flash-lite-tts".into();
        let (url, body, _) = request(&settings, "سلام Mimi", "").unwrap();
        assert_eq!(url.path(), "/v1beta/interactions");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
    }
    #[test]
    fn fragmented_sse_preserves_audio_and_rejects_wrong_formats_and_private_errors() {
        let input=b"event: step.delta\r\ndata: {\"event_type\":\"step.delta\",\"delta\":{\"type\":\"audio\",\"mime_type\":\"audio/l16\",\"data\":\"AAABAA==\"}}\r\n\r\ndata: {\"event_type\":\"step.stop\"}\n\n";
        let mut sse = Sse::default();
        let mut output = Vec::new();
        for part in input.chunks(7) {
            output.extend(sse.push(part, Encoding::Interaction).unwrap().concat());
        }
        assert_eq!(output, [0, 0, 1, 0]);
        assert!(sse.complete);
        for delta in [
            json!({"type":"audio","mime_type":"audio/wav","data":"AAABAA=="}),
            json!({"type":"audio","sample_rate":48000,"data":"AAABAA=="}),
        ] {
            let input = format!(
                "data: {}\n\n",
                json!({"event_type":"step.delta","delta":delta})
            );
            assert_eq!(
                Sse::default()
                    .push(input.as_bytes(), Encoding::Interaction)
                    .unwrap_err(),
                "output_invalid_audio"
            );
        }
        assert_eq!(
            Sse::default()
                .push(
                    b"data: {\"error\":\"private server details\"}\n\n",
                    Encoding::Interaction
                )
                .unwrap_err(),
            "output_stream_failed"
        );
    }
    #[tokio::test]
    async fn pcm_arrives_before_http_completion_and_odd_network_boundaries_are_joined() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 8192];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nContent-Length: 16\r\nConnection: close\r\n\r\n\x00\x00\x01\x00\x02\x00\x03\x00\x04").await.unwrap();
            wait.await.unwrap();
            socket.write_all(&[0, 5, 0, 6, 0, 7, 0]).await.unwrap();
        });
        let settings = SpeechSettings {
            speech_provider: SpeechProvider::OpenAICompatible,
            endpoint: format!("http://{address}/audio/speech"),
            model: "test-model".into(),
            voice: "test-voice".into(),
            ..Default::default()
        };
        let network = ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
            mode: crate::core::network_proxy::ProxyMode::Direct,
            ..Default::default()
        })
        .unwrap();
        let (tx, mut rx) = mpsc::channel(2);
        let cancel = Arc::new(AtomicBool::new(false));
        let synthesis = tokio::spawn(async move {
            synthesize_stream(
                &settings,
                "synthetic",
                "Hello test",
                "",
                &network,
                &tx,
                &cancel,
            )
            .await
        });
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap(),
            [0, 0, 1, 0, 2, 0, 3, 0]
        );
        assert!(!synthesis.is_finished());
        release.send(()).unwrap();
        let mut rest = Vec::new();
        while let Some(data) = rx.recv().await {
            rest.extend(data);
        }
        assert_eq!(rest, [4, 0, 5, 0, 6, 0, 7, 0]);
        assert_eq!(synthesis.await.unwrap().unwrap().bytes, 16);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn cancellation_stops_an_incomplete_http_body_without_waiting_for_eof() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 8192];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nContent-Length: 48000\r\n\r\n\x00\x00\x01\x00\x02\x00\x03\x00").await.unwrap();
            std::future::pending::<()>().await;
        });
        let settings = SpeechSettings {
            speech_provider: SpeechProvider::OpenAICompatible,
            endpoint: format!("http://{address}/audio/speech"),
            model: "test-model".into(),
            voice: "test-voice".into(),
            ..Default::default()
        };
        let network = ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
            mode: crate::core::network_proxy::ProxyMode::Direct,
            ..Default::default()
        })
        .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = cancel.clone();
        let (tx, mut rx) = mpsc::channel(2);
        let job = tokio::spawn(async move {
            synthesize_stream(&settings, "synthetic", "Hello", "", &network, &tx, &cancel).await
        });
        assert!(tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .is_some());
        trigger.store(true, Ordering::SeqCst);
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), job)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err(),
            "output_cancelled"
        );
        server.abort();
    }
}
