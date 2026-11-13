//! Only synthesized PCM is routed here; raw microphone audio never enters this module.
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(all(test, target_os = "windows"))]
use std::sync::Arc;
pub(crate) static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
pub(crate) static ACTIVE: AtomicUsize = AtomicUsize::new(0);
#[cfg(target_os = "windows")]
pub(crate) struct PlaybackOwner;
#[cfg(target_os = "windows")]
impl Drop for PlaybackOwner {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while ACTIVE.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[derive(Serialize)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

#[cfg(target_os = "windows")]
pub fn devices() -> Result<Vec<OutputDevice>, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    Ok(cpal::default_host()
        .output_devices()
        .map_err(|_| "output_devices_unavailable")?
        .filter_map(|d| {
            Some(OutputDevice {
                id: d.id().ok()?.to_string(),
                name: d.description().ok()?.name().to_owned(),
            })
        })
        .collect())
}
#[cfg(not(target_os = "windows"))]
pub fn devices() -> Result<Vec<OutputDevice>, String> {
    Err("output_platform_unsupported".into())
}

#[cfg(all(test, target_os = "windows"))]
pub fn play(
    pcm: Vec<u8>,
    device: &str,
    cancelled: Arc<AtomicBool>,
    duck: bool,
    volume: f32,
) -> Result<(), String> {
    let (tx, rx) = tokio::sync::mpsc::channel(12);
    let producer = std::thread::spawn(move || {
        for part in pcm.chunks(4096) {
            if tx.blocking_send(part.to_vec()).is_err() {
                break;
            }
        }
    });
    let result = crate::streaming_playback::play(
        rx,
        device.to_owned(),
        cancelled,
        duck,
        volume,
        std::time::Instant::now(),
        None,
    )
    .map(|_| ());
    let _ = producer.join();
    result
}
#[cfg(target_os = "windows")]
pub(crate) fn playback_error(error: cpal::Error) -> String {
    #[cfg(test)]
    eprintln!("native speech stream: {error}");
    match error.kind() {
        cpal::ErrorKind::DeviceBusy => "output_device_busy",
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated => {
            "output_device_missing"
        }
        _ => "output_playback_failed",
    }
    .into()
}
#[cfg(all(test, target_os = "windows"))]
mod tests {
    #[test]
    fn busy_and_invalidated_outputs_remain_categorical_without_os_messages() {
        for (kind, expected) in [
            (cpal::ErrorKind::DeviceBusy, "output_device_busy"),
            (cpal::ErrorKind::StreamInvalidated, "output_device_missing"),
            (cpal::ErrorKind::BackendError, "output_playback_failed"),
        ] {
            assert_eq!(
                super::playback_error(cpal::Error::with_message(kind, "private native detail")),
                expected
            );
        }
    }
}
