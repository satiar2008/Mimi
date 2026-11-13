//! Scoped Windows session attenuation. Never changes endpoint/master volume.
#[cfg(target_os = "windows")]
mod platform {
    use windows::core::{Interface, GUID};
    use windows::Win32::{Media::Audio::*, System::Com::*};

    const CONTEXT: GUID = GUID::from_u128(0x77145878_4f8c_411d_9496_dfa4663ebfe1);
    struct Entry {
        volume: ISimpleAudioVolume,
        original: f32,
        applied: f32,
    }
    pub struct Ducking {
        entries: Vec<Entry>,
        _apartment: Apartment,
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    impl Ducking {
        pub fn begin(factor: f32) -> Result<Self, String> {
            if !(0.05..=1.0).contains(&factor) {
                return Err("output_invalid_settings".into());
            }
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .map_err(|_| "output_ducking_failed")?;
            let mut guard = Self {
                entries: Vec::new(),
                _apartment: Apartment,
            };
            // Every successful mutation is immediately owned by the guard, so an
            // enumeration or playback error restores the already changed sessions.
            unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                        .map_err(|_| "output_ducking_failed")?;
                let devices = enumerator
                    .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                    .map_err(|_| "output_ducking_failed")?;
                for i in 0..devices
                    .GetCount()
                    .map_err(|_| "output_ducking_failed")?
                    .min(64)
                {
                    let device = devices.Item(i).map_err(|_| "output_ducking_failed")?;
                    let manager: IAudioSessionManager2 = device
                        .Activate(CLSCTX_ALL, None)
                        .map_err(|_| "output_ducking_failed")?;
                    let sessions = manager
                        .GetSessionEnumerator()
                        .map_err(|_| "output_ducking_failed")?;
                    for index in 0..sessions
                        .GetCount()
                        .map_err(|_| "output_ducking_failed")?
                        .min(256)
                    {
                        let session = sessions
                            .GetSession(index)
                            .map_err(|_| "output_ducking_failed")?;
                        let control: IAudioSessionControl2 =
                            session.cast().map_err(|_| "output_ducking_failed")?;
                        if control
                            .GetProcessId()
                            .map_err(|_| "output_ducking_failed")?
                            == std::process::id()
                        {
                            continue;
                        }
                        let volume: ISimpleAudioVolume =
                            session.cast().map_err(|_| "output_ducking_failed")?;
                        let original = volume
                            .GetMasterVolume()
                            .map_err(|_| "output_ducking_failed")?;
                        let applied = original * factor;
                        volume
                            .SetMasterVolume(applied, &CONTEXT)
                            .map_err(|_| "output_ducking_failed")?;
                        guard.entries.push(Entry {
                            volume,
                            original,
                            applied,
                        });
                    }
                }
            }
            Ok(guard)
        }
    }
    impl Drop for Ducking {
        fn drop(&mut self) {
            for entry in &self.entries {
                // Preserve an ordinary user volume adjustment made during reading.
                if unsafe { entry.volume.GetMasterVolume() }
                    .is_ok_and(|current| super::still_owned(current, entry.applied))
                {
                    let _ = unsafe { entry.volume.SetMasterVolume(entry.original, &CONTEXT) };
                }
            }
        }
    }
    #[cfg(test)]
    mod native_tests {
        use super::*;
        #[test]
        #[ignore = "requires a Windows playback endpoint"]
        fn native_reader_playback_does_not_inherit_a_cpal_sta() {
            use cpal::traits::{DeviceTrait, HostTrait};
            std::thread::spawn(|| {
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                    .ok()
                    .unwrap();
                let _sta = Apartment;
                let device = cpal::default_host()
                    .default_output_device()
                    .expect("default output")
                    .id()
                    .unwrap()
                    .to_string();
                crate::speech_playback::play(
                    vec![0; 2400],
                    &device,
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    true,
                    0.2,
                )
                .unwrap();
            })
            .join()
            .unwrap();
        }
        #[test]
        #[ignore = "requires an explicitly started external synthetic audio fixture"]
        fn native_reader_ducking_restores_and_preserves_manual_volume() {
            let pid: u32 = std::env::var("MIMI_READER_FIXTURE_PID")
                .expect("external fixture PID")
                .parse()
                .unwrap();
            assert_ne!(pid, std::process::id());
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .unwrap();
            let _apartment = Apartment;
            let volume = unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
                let devices = enumerator
                    .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                    .unwrap();
                let mut found = None;
                for i in 0..devices.GetCount().unwrap() {
                    let device = devices.Item(i).unwrap();
                    let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).unwrap();
                    let sessions = manager.GetSessionEnumerator().unwrap();
                    for j in 0..sessions.GetCount().unwrap() {
                        let session = sessions.GetSession(j).unwrap();
                        let control: IAudioSessionControl2 = session.cast().unwrap();
                        if control.GetProcessId().unwrap() == pid {
                            found = Some(session.cast::<ISimpleAudioVolume>().unwrap());
                        }
                    }
                }
                found.expect("external synthetic fixture must have a live session")
            };
            let original = unsafe { volume.GetMasterVolume() }.unwrap();
            assert!(original > 0.05);
            let guard = Ducking::begin(0.2).unwrap();
            assert!(super::super::still_owned(
                unsafe { volume.GetMasterVolume() }.unwrap(),
                original * 0.2
            ));
            drop(guard);
            assert!(super::super::still_owned(
                unsafe { volume.GetMasterVolume() }.unwrap(),
                original
            ));
            let guard = Ducking::begin(0.2).unwrap();
            let manual = original * 0.7;
            unsafe { volume.SetMasterVolume(manual, &CONTEXT) }.unwrap();
            drop(guard);
            let preserved =
                super::super::still_owned(unsafe { volume.GetMasterVolume() }.unwrap(), manual);
            unsafe { volume.SetMasterVolume(original, &CONTEXT) }.unwrap();
            assert!(preserved);
            println!("Native external-session attenuation, normal restore and manual-volume preservation passed.");
        }
    }
}
#[cfg(target_os = "windows")]
pub use platform::Ducking;

#[cfg(any(target_os = "windows", test))]
fn still_owned(current: f32, applied: f32) -> bool {
    (current - applied).abs() <= 0.00001
}

#[cfg(test)]
mod tests {
    #[test]
    fn restore_does_not_override_a_user_volume_change() {
        assert!(super::still_owned(0.16, 0.8 * 0.2));
        assert!(!super::still_owned(0.3, 0.16));
        assert!(!super::still_owned(f32::NAN, 0.16));
    }
}
