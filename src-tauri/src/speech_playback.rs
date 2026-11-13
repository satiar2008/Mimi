//! Only synthesized PCM is routed here; raw microphone audio never enters this module.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

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
pub fn play(pcm: Vec<u8>, device_id: &str, cancelled: Arc<AtomicBool>) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
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
    stream.play().map_err(|_| "output_playback_failed")?;
    let started = std::time::Instant::now();
    while !done.load(Ordering::SeqCst)
        && !failed.load(Ordering::SeqCst)
        && !cancelled.load(Ordering::SeqCst)
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
        .map_err(|_| "output_playback_failed".into())
}
#[cfg(not(target_os = "windows"))]
pub fn play(_: Vec<u8>, _: &str, _: Arc<AtomicBool>) -> Result<(), String> {
    Err("output_platform_unsupported".into())
}
