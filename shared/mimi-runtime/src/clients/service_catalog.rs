//! Bounded catalogs and PCM synthesis. No provider error bodies or credentials in errors.
use super::provider_network::ProviderNetwork;
use crate::core::translation_output::{speech_endpoint, valid_id, SpeechProvider, SpeechSettings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogItem {
    pub id: String,
    pub name: String,
    pub compatible: bool,
}

pub fn gemini_translation_model(model: &str) -> bool {
    valid_id(model) && model.starts_with("gemini-") && model.contains("live-translate")
}

fn client(network: &ProviderNetwork, endpoint: &url::Url) -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    network
        .http_client_builder(endpoint)
        .map_err(|_| "catalog_network".to_string())?
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "catalog_network".into())
}
async fn body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err(format!("catalog_http_{}", response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "catalog_network".to_string())?
    {
        if bytes.len() + chunk.len() > limit {
            return Err("catalog_response_too_large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn gemini_models(
    key: &str,
    network: &ProviderNetwork,
) -> Result<Vec<CatalogItem>, String> {
    gemini_catalog(key, network, false).await
}
pub async fn gemini_speech_models(
    key: &str,
    network: &ProviderNetwork,
) -> Result<Vec<CatalogItem>, String> {
    gemini_catalog(key, network, true).await
}
async fn gemini_catalog(
    key: &str,
    network: &ProviderNetwork,
    speech: bool,
) -> Result<Vec<CatalogItem>, String> {
    let mut endpoint =
        url::Url::parse("https://generativelanguage.googleapis.com/v1beta/models").unwrap();
    let http = client(network, &endpoint)?;
    let mut result = Vec::new();
    let mut token = String::new();
    for _ in 0..10 {
        endpoint
            .query_pairs_mut()
            .clear()
            .append_pair("pageSize", "1000");
        if !token.is_empty() {
            endpoint.query_pairs_mut().append_pair("pageToken", &token);
        }
        let response = http
            .get(endpoint.clone())
            .header("x-goog-api-key", key)
            .send()
            .await
            .map_err(|_| "catalog_network".to_string())?;
        let value: Value = serde_json::from_slice(&body(response, 2 * 1024 * 1024).await?)
            .map_err(|_| "catalog_invalid_response".to_string())?;
        let models = value["models"]
            .as_array()
            .ok_or("catalog_invalid_response")?;
        for m in models {
            let id = m["name"]
                .as_str()
                .unwrap_or_default()
                .trim_start_matches("models/");
            if !valid_id(id) {
                continue;
            }
            let live = m["supportedGenerationMethods"]
                .as_array()
                .is_some_and(|methods| methods.iter().any(|v| v == "bidiGenerateContent"));
            let tts = id.starts_with("gemini-")
                && id.contains("tts")
                && m["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|methods| methods.iter().any(|v| v == "generateContent"));
            if (speech && tts) || (!speech && live) {
                result.push(CatalogItem {
                    id: id.into(),
                    name: id.into(),
                    compatible: speech || gemini_translation_model(id),
                });
            }
        }
        token = value["nextPageToken"].as_str().unwrap_or_default().into();
        if token.is_empty() {
            result.sort_by(|a, b| a.id.cmp(&b.id));
            return Ok(result);
        }
    }
    Err("catalog_pagination_limit".into())
}
/// Derive a catalog from the configured service origin, never from a UI-supplied foreign host.
pub fn models_endpoint(endpoint: &str) -> Result<url::Url, String> {
    let mut url = speech_endpoint(
        &endpoint
            .replacen("wss://", "https://", 1)
            .replacen("ws://", "http://", 1),
    )?;
    let path = url.path().to_owned();
    let prefix = path
        .strip_suffix("/chat/completions")
        .or_else(|| path.strip_suffix("/audio/speech"))
        .or_else(|| path.strip_suffix("/realtime"))
        .ok_or("catalog_not_supported")?;
    url.set_path(&format!("{prefix}/models"));
    Ok(url)
}
pub fn compatible_speech_endpoint(endpoint: &str) -> Result<String, String> {
    let mut url = models_endpoint(endpoint)?;
    let prefix = url
        .path()
        .strip_suffix("/models")
        .ok_or("catalog_not_supported")?
        .to_owned();
    url.set_path(&format!("{prefix}/audio/speech"));
    Ok(url.into())
}
pub async fn compatible_models(
    endpoint: &str,
    key: &str,
    network: &ProviderNetwork,
) -> Result<Vec<CatalogItem>, String> {
    let endpoint = models_endpoint(endpoint)?;
    let http = client(network, &endpoint)?;
    let mut request = http.get(endpoint);
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "catalog_network".to_string())?;
    let value: Value = serde_json::from_slice(&body(response, 2 * 1024 * 1024).await?)
        .map_err(|_| "catalog_invalid_response".to_string())?;
    let data = value["data"].as_array().ok_or("catalog_invalid_response")?;
    Ok(data
        .iter()
        .take(2000)
        .filter_map(|m| m["id"].as_str())
        .filter(|id| valid_id(id))
        .map(|id| CatalogItem {
            id: id.into(),
            name: id.into(),
            compatible: true,
        })
        .collect())
}
pub async fn eleven_catalog(
    key: &str,
    voices: bool,
    network: &ProviderNetwork,
) -> Result<Vec<CatalogItem>, String> {
    let mut endpoint = url::Url::parse(if voices {
        "https://api.elevenlabs.io/v2/voices"
    } else {
        "https://api.elevenlabs.io/v1/models"
    })
    .unwrap();
    let http = client(network, &endpoint)?;
    let mut result = Vec::new();
    let mut token = String::new();
    for _ in 0..20 {
        if voices {
            endpoint
                .query_pairs_mut()
                .clear()
                .append_pair("page_size", "100");
            if !token.is_empty() {
                endpoint
                    .query_pairs_mut()
                    .append_pair("next_page_token", &token);
            }
        }
        let response = http
            .get(endpoint.clone())
            .header("xi-api-key", key)
            .send()
            .await
            .map_err(|_| "catalog_network".to_string())?;
        let value: Value = serde_json::from_slice(&body(response, 2 * 1024 * 1024).await?)
            .map_err(|_| "catalog_invalid_response".to_string())?;
        let list = if voices {
            value["voices"].as_array()
        } else {
            value.as_array()
        }
        .ok_or("catalog_invalid_response")?;
        for m in list {
            let id = m[if voices { "voice_id" } else { "model_id" }]
                .as_str()
                .unwrap_or_default();
            if valid_id(id) && (voices || m["can_do_text_to_speech"].as_bool() == Some(true)) {
                result.push(CatalogItem {
                    id: id.into(),
                    name: m["name"].as_str().unwrap_or(id).chars().take(128).collect(),
                    compatible: true,
                });
            }
        }
        if !voices || value["has_more"].as_bool() != Some(true) {
            return Ok(result);
        }
        token = value["next_page_token"]
            .as_str()
            .ok_or("catalog_invalid_response")?
            .into();
    }
    Err("catalog_pagination_limit".into())
}
/// Returns mono 24kHz little-endian signed PCM16, bounded to 60 seconds.
pub async fn synthesize(
    settings: &SpeechSettings,
    key: &str,
    text: &str,
    network: &ProviderNetwork,
) -> Result<Vec<u8>, String> {
    settings.validate(false)?;
    if !valid_id(&settings.model)
        || !valid_id(&settings.voice)
        || settings.voice.contains('/')
        || text.is_empty()
        || text.chars().count() > 4000
    {
        return Err("output_invalid_text".into());
    }
    let (endpoint, payload) = match settings.speech_provider {
        SpeechProvider::ElevenLabs => (
            url::Url::parse(&format!(
                "https://api.elevenlabs.io/v1/text-to-speech/{}?output_format=pcm_24000",
                settings.voice
            ))
            .map_err(|_| "output_invalid_voice".to_string())?,
            json!({"text":text,"model_id":settings.model,"voice_settings":{"stability":settings.stability,"similarity_boost":settings.similarity,"speed":settings.speed}}),
        ),
        SpeechProvider::OpenAICompatible => (
            speech_endpoint(&settings.endpoint)?,
            json!({"input":text,"model":settings.model,"voice":settings.voice,"speed":settings.speed,"response_format":"pcm"}),
        ),
        SpeechProvider::Gemini => (
            url::Url::parse(&format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                settings.model
            ))
            .map_err(|_| "output_invalid_settings".to_string())?,
            gemini_speech_payload(settings, text)?,
        ),
        SpeechProvider::MainService => return Err("output_main_service_unsupported".into()),
    };
    let http = client(network, &endpoint)?;
    let request = http.post(endpoint).json(&payload);
    let request = if settings.speech_provider == SpeechProvider::ElevenLabs {
        request.header("xi-api-key", key)
    } else if settings.speech_provider == SpeechProvider::Gemini {
        request.header("x-goog-api-key", key)
    } else if key.is_empty() {
        request
    } else {
        request.bearer_auth(key)
    };
    let response = request
        .send()
        .await
        .map_err(|_| "catalog_network".to_string())?;
    if settings.speech_provider == SpeechProvider::Gemini {
        let value: Value = serde_json::from_slice(&body(response, 4 * 1024 * 1024).await?)
            .map_err(|_| "output_invalid_audio".to_string())?;
        return gemini_pcm(&value);
    }
    if !response.status().is_success() {
        return Err(format!("catalog_http_{}", response.status().as_u16()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.contains("json") || content_type.starts_with("text/") {
        return Err("output_invalid_audio".into());
    }
    let pcm = body(response, 24_000 * 2 * 60).await?;
    if pcm.is_empty() || pcm.len() % 2 != 0 {
        return Err("output_invalid_audio".into());
    }
    Ok(pcm)
}

fn gemini_speech_payload(settings: &SpeechSettings, text: &str) -> Result<Value, String> {
    // Model IDs are path components, never a provider-controlled URL.
    if !settings.model.starts_with("gemini-")
        || !settings.model.contains("tts")
        || settings.model.contains('/')
    {
        return Err("output_invalid_settings".into());
    }
    let mut config = json!({"responseModalities":["AUDIO"],"speechConfig":{"voiceConfig":{"prebuiltVoiceConfig":{"voiceName":settings.voice}}}});
    if settings.model.starts_with("gemini-3.8-") {
        config["speechConfig"]["voiceConfig"] = json!({"voice":settings.voice});
        config["responseFormat"] = json!({"audio":{"mimeType":"AUDIO_L16","sampleRate":24000}});
    }
    Ok(json!({"contents":[{"role":"user","parts":[{"text":text}]}],"generationConfig":config}))
}
fn gemini_pcm(value: &Value) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let parts = value
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .ok_or("output_invalid_audio")?;
    let mut pcm = Vec::new();
    for part in parts.iter().take(32) {
        let Some(audio) = part.get("inlineData") else {
            continue;
        };
        let mime = audio["mimeType"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        // A different rate, codec or channel layout must never play as 24kHz PCM.
        if !mime.starts_with("audio/l16") || !mime.split(';').any(|v| v.trim() == "rate=24000") {
            return Err("output_invalid_audio".into());
        }
        let data = audio["data"].as_str().ok_or("output_invalid_audio")?;
        if data.len() > 4_000_000 {
            return Err("output_invalid_audio".into());
        }
        let chunk = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| "output_invalid_audio")?;
        if chunk.len() % 2 != 0 || pcm.len() + chunk.len() > 24_000 * 2 * 60 {
            return Err("output_invalid_audio".into());
        }
        pcm.extend(chunk);
    }
    if pcm.is_empty() {
        return Err("output_invalid_audio".into());
    }
    Ok(pcm)
}
#[cfg(test)]
mod tests {
    #[test]
    fn gemini_speech_keeps_the_selected_voice_and_verbatim_text() {
        let mut settings = super::SpeechSettings {
            speech_provider: super::SpeechProvider::Gemini,
            model: "gemini-2.5-flash-preview-tts".into(),
            voice: "Kore".into(),
            ..Default::default()
        };
        let payload = super::gemini_speech_payload(&settings, "سلام Mimi 12").unwrap();
        assert_eq!(payload["contents"][0]["parts"][0]["text"], "سلام Mimi 12");
        assert_eq!(
            payload["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]
                ["voiceName"],
            "Kore"
        );
        settings.model = "gemini-3.8-flash-tts".into();
        let payload = super::gemini_speech_payload(&settings, "Hello").unwrap();
        assert_eq!(
            payload["generationConfig"]["responseFormat"]["audio"]["mimeType"],
            "AUDIO_L16"
        );
        assert_eq!(
            payload["generationConfig"]["speechConfig"]["voiceConfig"]["voice"],
            "Kore"
        );
        settings.model = "gemini-live-translate".into();
        assert!(super::gemini_speech_payload(&settings, "Hello").is_err());
    }
    #[test]
    fn gemini_audio_checks_format_rate_and_frame_boundaries() {
        let audio = |mime: &str, data: &str| serde_json::json!({"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":mime,"data":data}}]}}]});
        assert_eq!(
            super::gemini_pcm(&audio("audio/L16;codec=pcm;rate=24000", "AAABAA==")).unwrap(),
            vec![0, 0, 1, 0]
        );
        for (mime, data) in [
            ("audio/l16;rate=48000", "AAABAA=="),
            ("audio/mpeg", "AAABAA=="),
            ("audio/l16;rate=24000", "AA=="),
            ("audio/l16;rate=24000", "invalid!"),
        ] {
            assert!(super::gemini_pcm(&audio(mime, data)).is_err());
        }
    }
    async fn fixture(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
                if let Some(boundary) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers =
                        String::from_utf8_lossy(&request[..boundary]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= boundary + 4 + length {
                        break;
                    }
                }
            }
            socket.write_all(&response).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}/v1/audio/speech"), task)
    }
    fn direct() -> super::ProviderNetwork {
        super::ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
            mode: crate::core::network_proxy::ProxyMode::Direct,
            ..Default::default()
        })
        .unwrap()
    }
    #[tokio::test]
    async fn compatible_catalog_uses_same_origin_and_own_authorization() {
        let payload = br#"{"data":[{"id":"model-a"},{"id":"model-b"}]}"#;
        let response=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",payload.len()).into_bytes().into_iter().chain(payload.iter().copied()).collect();
        let (endpoint, task) = fixture(response).await;
        let models = super::compatible_models(&endpoint, "synthetic-key", &direct())
            .await
            .unwrap();
        assert_eq!(models.len(), 2);
        let request = task.await.unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /v1/models "));
        assert!(request.contains("authorization: bearer synthetic-key"));
    }
    #[tokio::test]
    async fn synthesis_uses_selected_model_voice_and_pcm_without_credentials_in_body() {
        let payload = [0u8, 0, 1, 0, 2, 0, 0, 0];
        let response=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: audio/pcm\r\nConnection: close\r\n\r\n",payload.len()).into_bytes().into_iter().chain(payload).collect();
        let (endpoint, task) = fixture(response).await;
        let settings = crate::core::translation_output::SpeechSettings {
            speech_provider: crate::core::translation_output::SpeechProvider::OpenAICompatible,
            endpoint,
            model: "model-a".into(),
            voice: "voice-a".into(),
            ..Default::default()
        };
        assert_eq!(
            super::synthesize(&settings, "synthetic-key", "Hello sample", &direct())
                .await
                .unwrap(),
            payload
        );
        let request = task.await.unwrap();
        let body = request.split("\r\n\r\n").nth(1).unwrap();
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["model"], "model-a");
        assert_eq!(body["voice"], "voice-a");
        assert_eq!(body["response_format"], "pcm");
        assert!(!body.to_string().contains("synthetic-key"));
    }
    #[tokio::test]
    async fn response_errors_do_not_expose_provider_body() {
        let payload = b"private provider content";
        let response = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .into_bytes()
        .into_iter()
        .chain(payload.iter().copied())
        .collect();
        let (endpoint, task) = fixture(response).await;
        assert_eq!(
            super::compatible_models(&endpoint, "key", &direct())
                .await
                .unwrap_err(),
            "catalog_http_401"
        );
        task.await.unwrap();
    }

    use super::*;
    #[test]
    fn catalog_stays_on_configured_service() {
        assert_eq!(
            models_endpoint("https://example.com/custom/v1/chat/completions")
                .unwrap()
                .as_str(),
            "https://example.com/custom/v1/models"
        );
        assert!(models_endpoint("https://example.com/unknown").is_err());
        assert!(models_endpoint("https://key@example.com/v1/realtime").is_err());
    }
    #[test]
    fn native_audio_is_not_translation_protocol() {
        assert!(gemini_translation_model(
            "gemini-3.5-live-translate-preview"
        ));
        assert!(!gemini_translation_model("gemini-3.8-live"));
        assert!(!gemini_translation_model(
            "gemini-3.5-live-translate-preview?key=secret"
        ));
    }
}
