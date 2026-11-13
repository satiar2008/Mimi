//! Only synthesized PCM is routed here; raw microphone audio never enters this module.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
#[cfg(target_os = "windows")]
struct PlaybackOwner;
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

#[cfg(target_os = "windows")]
pub fn play(
    pcm: Vec<u8>,
    device_id: &str,
    cancelled: Arc<AtomicBool>,
    duck: bool,
    volume: f32,
) -> Result<(), String> {
    let device = device_id.to_owned();
    // CPAL initializes an STA on first use. Volume-session enumeration needs an
    // MTA owner; a reused Tokio blocking worker may already belong to an STA.
    // Own both operations on one fresh MTA thread, including native destruction.
    std::thread::Builder::new()
        .name("mimi-speech-output".into())
        .spawn(move || {
            crate::audio::windows::with_com(|| play_inner(pcm, &device, cancelled, duck, volume))
        })
        .map_err(|_| "output_playback_failed".to_string())?
        .join()
        .map_err(|_| "output_playback_failed".to_string())?
}

#[cfg(target_os = "windows")]
fn play_inner(
    pcm: Vec<u8>,
    device_id: &str,
    cancelled: Arc<AtomicBool>,
    duck: bool,
    volume: f32,
) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let _owner = PlaybackOwner;
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        return Err("output_cancelled".into());
    }
    let host = cpal::default_host();
    let device = host
        .output_devices()
        .map_err(|_| "output_devices_unavailable")?
        .find(|d| d.id().is_ok_and(|id| id.to_string() == device_id))
        .ok_or("output_device_missing")?;
    let config = device
        .default_output_config()
        .map_err(|_| "output_device_unsupported")?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate() as f64;
    let samples: Arc<Vec<f32>> = Arc::new(
        pcm.as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect(),
    );
    let done = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    // Bounded interpolation adapts the API's mono 24kHz to the selected device.
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(
            &device,
            config.into(),
            channels,
            rate,
            samples,
            done.clone(),
            failed.clone(),
            cancelled.clone(),
        ),
        cpal::SampleFormat::I16 => build::<i16>(
            &device,
            config.into(),
            channels,
            rate,
            samples,
            done.clone(),
            failed.clone(),
            cancelled.clone(),
        ),
        cpal::SampleFormat::U16 => build::<u16>(
            &device,
            config.into(),
            channels,
            rate,
            samples,
            done.clone(),
            failed.clone(),
            cancelled.clone(),
        ),
        _ => return Err("output_device_unsupported".into()),
    }?;
    if cancelled.load(Ordering::SeqCst) {
        return Ok(());
    }
    let _ducking = if duck {
        Some(crate::speech_ducking::Ducking::begin(volume)?)
    } else {
        None
    };
    stream.play().map_err(playback_error)?;
    let started = std::time::Instant::now();
    while !done.load(Ordering::SeqCst)
        && !failed.load(Ordering::SeqCst)
        && !cancelled.load(Ordering::SeqCst)
        && !SHUTTING_DOWN.load(Ordering::SeqCst)
    {
        if started.elapsed() > std::time::Duration::from_secs(65) {
            return Err("output_playback_timeout".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    drop(stream);
    if failed.load(Ordering::SeqCst) {
        Err("output_playback_failed".into())
    } else {
        Ok(())
    }
}
#[cfg(target_os = "windows")]
#[allow(clippy::too_many_arguments)]
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    rate: f64,
    samples: Arc<Vec<f32>>,
    done: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;
    let mut position: f64 = 0.0;
    let step = 24_000.0 / rate;
    let mut tail = 0usize;
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                for frame in output.chunks_mut(channels) {
                    let index = position as usize;
                    let sample = if cancelled.load(Ordering::SeqCst) {
                        0.0
                    } else if index < samples.len() {
                        let a = samples[index];
                        let b = samples.get(index + 1).copied().unwrap_or(a);
                        a + (b - a) * (position.fract() as f32)
                    } else {
                        tail += 1;
                        if tail > rate as usize / 5 {
                            done.store(true, Ordering::SeqCst);
                        }
                        0.0
                    };
                    frame.fill(T::from_sample(sample));
                    position += step;
                }
            },
            move |_| {
                failed.store(true, Ordering::SeqCst);
            },
            None,
        )
        .map_err(playback_error)
}
#[cfg(target_os = "windows")]
fn playback_error(error: cpal::Error) -> String {
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
#[cfg(not(target_os = "windows"))]
pub fn play(_: Vec<u8>, _: &str, _: Arc<AtomicBool>, _: bool, _: f32) -> Result<(), String> {
    Err("output_platform_unsupported".into())
}
