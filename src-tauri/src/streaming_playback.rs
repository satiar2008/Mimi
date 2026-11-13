//! One native output stream stays alive while text and PCM continue arriving.
use serde::Serialize;
#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};
use tokio::sync::mpsc;
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackMetrics {
    pub first_frame_ms: Option<u64>,
    pub underruns: u64,
}
#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires an explicitly selected playback endpoint; plays only a synthetic tone"]
    async fn native_pcm_starts_before_the_delayed_producer_finishes() {
        let device = std::env::var("MIMI_READER_OUTPUT_DEVICE_ID").expect("explicit output ID");
        let (tx, rx) = mpsc::channel(12);
        let cancel = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        let playback = tokio::task::spawn_blocking(move || {
            play(rx, device, cancel, false, 0.2, started, None)
        });
        let tone: Vec<u8> = (0..4800)
            .flat_map(|i| {
                (((i as f32 * 440.0 * std::f32::consts::TAU / 24000.0).sin() * 1600.0) as i16)
                    .to_le_bytes()
            })
            .collect();
        for part in tone.chunks(4096) {
            tx.send(part.to_vec()).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        assert!(!playback.is_finished());
        let delayed_ms = started.elapsed().as_millis() as u64;
        for part in tone.chunks(4096) {
            tx.send(part.to_vec()).await.unwrap();
        }
        drop(tx);
        let metrics = playback.await.unwrap().unwrap();
        let first = metrics.first_frame_ms.expect("actual PCM consumption");
        assert!(first < delayed_ms, "first={first}, delayed={delayed_ms}");
        println!("first_frame_ms={first} delayed_packet_ms={delayed_ms} physical_microphone=false");
    }
}
#[cfg(target_os = "windows")]
pub fn play(
    rx: mpsc::Receiver<Vec<u8>>,
    device: String,
    cancel: Arc<AtomicBool>,
    duck: bool,
    volume: f32,
    started: Instant,
    first_frame: Option<tokio::sync::oneshot::Sender<u64>>,
) -> Result<PlaybackMetrics, String> {
    std::thread::Builder::new()
        .name("mimi-streaming-reader".into())
        .spawn(move || {
            crate::audio::windows::with_com(|| {
                play_inner(rx, &device, cancel, duck, volume, started, first_frame)
            })
        })
        .map_err(|_| "output_playback_failed")?
        .join()
        .map_err(|_| "output_playback_failed")?
}
#[cfg(not(target_os = "windows"))]
pub fn play(
    _: mpsc::Receiver<Vec<u8>>,
    _: String,
    _: Arc<AtomicBool>,
    _: bool,
    _: f32,
    _: Instant,
    _: Option<tokio::sync::oneshot::Sender<u64>>,
) -> Result<PlaybackMetrics, String> {
    Err("output_platform_unsupported".into())
}
#[cfg(target_os = "windows")]
struct Signals {
    active: AtomicBool,
    done: AtomicBool,
    failed: AtomicBool,
    first: AtomicU64,
    underruns: AtomicU64,
}
#[cfg(target_os = "windows")]
fn play_inner(
    rx: mpsc::Receiver<Vec<u8>>,
    device_id: &str,
    cancel: Arc<AtomicBool>,
    duck: bool,
    volume: f32,
    started: Instant,
    mut first_frame: Option<tokio::sync::oneshot::Sender<u64>>,
) -> Result<PlaybackMetrics, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    crate::speech_playback::ACTIVE.fetch_add(1, Ordering::SeqCst);
    let _owner = crate::speech_playback::PlaybackOwner;
    if crate::speech_playback::SHUTTING_DOWN.load(Ordering::SeqCst) || cancel.load(Ordering::SeqCst)
    {
        return Err("output_cancelled".into());
    }
    let device = cpal::default_host()
        .output_devices()
        .map_err(|_| "output_devices_unavailable")?
        .find(|d| d.id().is_ok_and(|v| v.to_string() == device_id))
        .ok_or("output_device_missing")?;
    let config = device
        .default_output_config()
        .map_err(|_| "output_device_unsupported")?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate();
    let signals = Arc::new(Signals {
        active: AtomicBool::new(false),
        done: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        first: AtomicU64::new(u64::MAX),
        underruns: AtomicU64::new(0),
    });
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(
            &device,
            config.into(),
            channels,
            rate,
            rx,
            signals.clone(),
            cancel.clone(),
            started,
        ),
        cpal::SampleFormat::I16 => build::<i16>(
            &device,
            config.into(),
            channels,
            rate,
            rx,
            signals.clone(),
            cancel.clone(),
            started,
        ),
        cpal::SampleFormat::U16 => build::<u16>(
            &device,
            config.into(),
            channels,
            rate,
            rx,
            signals.clone(),
            cancel.clone(),
            started,
        ),
        _ => return Err("output_device_unsupported".into()),
    }?;
    stream
        .play()
        .map_err(crate::speech_playback::playback_error)?;
    let mut guard = None;
    let mut quiet = Instant::now();
    let mut drained = None;
    loop {
        let first = signals.first.load(Ordering::SeqCst);
        if first != u64::MAX {
            if let Some(tx) = first_frame.take() {
                let _ = tx.send(first);
            }
        }
        if cancel.load(Ordering::SeqCst)
            || crate::speech_playback::SHUTTING_DOWN.load(Ordering::SeqCst)
        {
            break;
        }
        if signals.failed.load(Ordering::SeqCst) {
            return Err("output_playback_failed".into());
        }
        if signals.active.load(Ordering::SeqCst) {
            quiet = Instant::now();
            if duck && guard.is_none() {
                guard = Some(crate::speech_ducking::Ducking::begin(volume)?);
            }
        } else if quiet.elapsed() > std::time::Duration::from_millis(200) {
            guard.take();
        }
        if signals.done.load(Ordering::SeqCst)
            && drained.get_or_insert_with(Instant::now).elapsed()
                > std::time::Duration::from_millis(200)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    drop(stream);
    drop(guard);
    let first = signals.first.load(Ordering::SeqCst);
    Ok(PlaybackMetrics {
        first_frame_ms: (first != u64::MAX).then_some(first),
        underruns: signals.underruns.load(Ordering::SeqCst),
    })
}
#[cfg(target_os = "windows")]
#[allow(clippy::too_many_arguments)]
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    rate: u32,
    mut rx: mpsc::Receiver<Vec<u8>>,
    signals: Arc<Signals>,
    cancel: Arc<AtomicBool>,
    started: Instant,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;
    let error = signals.clone();
    let mut pcm = mimi_runtime::audio::speech_pcm::SpeechPcm::default();
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                let mut active = false;
                for frame in output.chunks_mut(channels) {
                    while pcm.should_fill() {
                        match rx.try_recv() {
                            Ok(bytes) => {
                                if pcm.push(&bytes).is_err() {
                                    signals.failed.store(true, Ordering::SeqCst);
                                    break;
                                }
                            }
                            Err(mpsc::error::TryRecvError::Disconnected) => {
                                pcm.finish();
                                break;
                            }
                            Err(mpsc::error::TryRecvError::Empty) => break,
                        }
                    }
                    let sample = if cancel.load(Ordering::SeqCst) {
                        None
                    } else {
                        pcm.sample(rate)
                    };
                    if sample.is_some() {
                        active = true;
                        let _ = signals.first.compare_exchange(
                            u64::MAX,
                            started.elapsed().as_millis() as u64,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        );
                    }
                    frame.fill(T::from_sample(sample.unwrap_or(0.0)));
                }
                signals.active.store(active, Ordering::SeqCst);
                signals.done.store(pcm.drained(), Ordering::SeqCst);
                signals.underruns.store(pcm.underruns, Ordering::SeqCst);
            },
            move |_| {
                error.failed.store(true, Ordering::SeqCst);
            },
            None,
        )
        .map_err(crate::speech_playback::playback_error)
}
