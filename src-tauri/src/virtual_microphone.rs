//! Optional, explicit setup of a signed virtual cable. No capture or default-device changes.
use crate::commands::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualMicrophoneState {
    installed: bool,
    output_device_id: Option<String>,
    input_name: Option<String>,
}
#[tauri::command]
pub fn virtual_microphone_state() -> Result<VirtualMicrophoneState, String> {
    if crate::commands::app_is_ui_test() {
        return Ok(VirtualMicrophoneState::default());
    }
    #[cfg(target_os = "windows")]
    {
        platform::device_state()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("virtual_microphone_unsupported".into())
    }
}
#[tauri::command]
pub async fn virtual_microphone_prepare(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("virtual_microphone_settings_only".into());
    }
    if crate::commands::app_is_ui_test() {
        return Err("virtual_microphone_ui_test".into());
    }
    if state.session.is_active() {
        return Err("output_settings_require_stop".into());
    }
    #[cfg(target_os = "windows")]
    {
        platform::prepare(&state.settings.preferences().network_proxy).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("virtual_microphone_unsupported".into())
    }
}
#[tauri::command]
pub async fn virtual_microphone_install(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<VirtualMicrophoneState, String> {
    if window.label() != "settings" {
        return Err("virtual_microphone_settings_only".into());
    }
    if crate::commands::app_is_ui_test() {
        return Err("virtual_microphone_ui_test".into());
    }
    if state.session.is_active() {
        return Err("output_settings_require_stop".into());
    }
    #[cfg(target_os = "windows")]
    {
        platform::install().await
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("virtual_microphone_unsupported".into())
    }
}
#[tauri::command]
pub fn virtual_microphone_use(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<VirtualMicrophoneState, String> {
    if window.label() != "settings" {
        return Err("virtual_microphone_settings_only".into());
    }
    if crate::commands::app_is_ui_test() {
        return Err("virtual_microphone_ui_test".into());
    }
    if state.session.is_active() {
        return Err("output_settings_require_stop".into());
    }
    let device = virtual_microphone_state()?;
    let id = device
        .output_device_id
        .clone()
        .filter(|_| device.installed)
        .ok_or("virtual_microphone_not_installed")?;
    state
        .settings
        .save_preferences(|prefs| prefs.translation_output.device = id)?;
    crate::translation_output::speech_output_cancel();
    Ok(device)
}
pub fn shutdown() {
    #[cfg(target_os = "windows")]
    {
        platform::shutdown();
    }
}

#[cfg(target_os = "windows")]
pub fn translation_output_is_default_input(device: &str) -> bool {
    platform::translation_output_is_default_input(device)
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait};
    use mimi_runtime::clients::provider_network::ProviderNetwork;
    use serde::Deserialize;
    use sha2::{Digest, Sha256};
    use std::{
        collections::BTreeMap,
        io::{Cursor, Read, Write},
        path::Path,
        sync::{
            atomic::{AtomicBool, Ordering},
            LazyLock, Mutex,
        },
    };
    static SETUP: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));
    static PREPARED: Mutex<Option<tempfile::TempDir>> = Mutex::new(None);
    static EXITING: AtomicBool = AtomicBool::new(false);
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Package {
        version: String,
        url: String,
        archive_sha256: String,
        installer: String,
        files: BTreeMap<String, String>,
    }
    fn package() -> Package {
        serde_json::from_str(include_str!("../virtual-microphone-package.json"))
            .expect("reviewed package manifest")
    }
    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    fn cable(name: &str) -> bool {
        name.contains("VB-Audio Virtual Cable")
            && !name.contains("Cable A")
            && !name.contains("Cable B")
            && !name.contains("Cable C")
            && !name.contains("Cable D")
    }
    fn standard_output(name: &str) -> bool {
        cable(name) && name.starts_with("CABLE Input (")
    }
    fn standard_input(name: &str) -> bool {
        cable(name) && name.starts_with("CABLE Output (")
    }
    pub fn translation_output_is_default_input(device: &str) -> bool {
        let host = cpal::default_host();
        let input_is_cable = host
            .default_input_device()
            .and_then(|d| d.description().ok())
            .is_some_and(|d| cable(d.name()));
        input_is_cable
            && host.output_devices().ok().is_some_and(|mut devices| {
                devices.any(|d| {
                    d.id().is_ok_and(|id| id.to_string() == device)
                        && d.description().is_ok_and(|v| cable(v.name()))
                })
            })
    }
    pub fn device_state() -> Result<VirtualMicrophoneState, String> {
        let host = cpal::default_host();
        let output_device_id = host
            .output_devices()
            .map_err(|_| "output_devices_unavailable")?
            .find(|d| d.description().is_ok_and(|v| standard_output(v.name())))
            .and_then(|d| d.id().ok())
            .map(|id| id.to_string());
        let input_name = host
            .input_devices()
            .map_err(|_| "output_devices_unavailable")?
            .filter_map(|d| d.description().ok())
            .find(|d| standard_input(d.name()))
            .map(|d| d.name().to_owned());
        Ok(VirtualMicrophoneState {
            installed: output_device_id.is_some() && input_name.is_some(),
            output_device_id,
            input_name,
        })
    }
    fn verify_files(root: &Path, package: &Package) -> Result<(), String> {
        if std::fs::read_dir(root)
            .map_err(|_| "virtual_microphone_integrity")?
            .count()
            != package.files.len()
        {
            return Err("virtual_microphone_integrity".into());
        }
        for (name, expected) in &package.files {
            if Path::new(name).components().count() != 1 || name.contains(['/', '\\', ':']) {
                return Err("virtual_microphone_integrity".into());
            }
            let path = root.join(name);
            let info =
                std::fs::symlink_metadata(&path).map_err(|_| "virtual_microphone_integrity")?;
            use std::os::windows::fs::MetadataExt;
            if !info.is_file()
                || info.file_attributes() & 0x400 != 0
                || info.len() > 8 * 1024 * 1024
            {
                return Err("virtual_microphone_integrity".into());
            }
            if digest(&std::fs::read(path).map_err(|_| "virtual_microphone_integrity")?)
                != *expected
            {
                return Err("virtual_microphone_integrity".into());
            }
        }
        Ok(())
    }
    fn extract(bytes: Vec<u8>, package: &Package) -> Result<tempfile::TempDir, String> {
        if digest(&bytes) != package.archive_sha256 {
            return Err("virtual_microphone_integrity".into());
        }
        let root = tempfile::Builder::new()
            .prefix(&format!("mimi-virtual-microphone-{}-", package.version))
            .tempdir()
            .map_err(|_| "virtual_microphone_storage")?;
        let mut zip =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "virtual_microphone_integrity")?;
        if zip.len() != package.files.len() || zip.len() > 64 {
            return Err("virtual_microphone_integrity".into());
        }
        let mut total = 0u64;
        for index in 0..zip.len() {
            let mut file = zip
                .by_index(index)
                .map_err(|_| "virtual_microphone_integrity")?;
            let name = file.name().to_owned();
            if !package.files.contains_key(&name)
                || file.enclosed_name().is_none()
                || file.is_dir()
                || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
                || file.size() > 8 * 1024 * 1024
            {
                return Err("virtual_microphone_integrity".into());
            }
            total += file.size();
            if total > 32 * 1024 * 1024 {
                return Err("virtual_microphone_integrity".into());
            }
            let mut data = Vec::new();
            (&mut file)
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut data)
                .map_err(|_| "virtual_microphone_integrity")?;
            if data.len() > 8 * 1024 * 1024 || digest(&data) != package.files[&name] {
                return Err("virtual_microphone_integrity".into());
            }
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.path().join(name))
                .map_err(|_| "virtual_microphone_storage")?;
            output
                .write_all(&data)
                .map_err(|_| "virtual_microphone_storage")?;
        }
        verify_files(root.path(), package)?;
        verify_signature(&root.path().join(&package.installer))?;
        verify_signature(&root.path().join("vbaudio_cable64_win10.cat"))?;
        Ok(root)
    }
    pub async fn prepare(proxy: &crate::core::network_proxy::ProxyConfig) -> Result<(), String> {
        let _guard = SETUP.lock().await;
        let manifest = package();
        let url = url::Url::parse(&manifest.url).map_err(|_| "virtual_microphone_integrity")?;
        let network = ProviderNetwork::resolve(proxy).map_err(|_| "catalog_network")?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = network
            .http_client_builder(&url)
            .map_err(|_| "catalog_network")?
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| "catalog_network")?;
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|_| "catalog_network")?;
        if !response.status().is_success() {
            return Err("virtual_microphone_download".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "catalog_network")? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err("virtual_microphone_integrity".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let root = tokio::task::spawn_blocking(move || extract(bytes, &manifest))
            .await
            .map_err(|_| "virtual_microphone_storage")??;
        if EXITING.load(Ordering::SeqCst) {
            return Err("virtual_microphone_cancelled".into());
        }
        *PREPARED.lock().unwrap() = Some(root);
        Ok(())
    }
    fn verify_signature(path: &Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::{Foundation::HWND, Security::WinTrust::*},
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut file = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(wide.as_ptr()),
            ..Default::default()
        };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let status = unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            )
        };
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                (&mut data as *mut WINTRUST_DATA).cast(),
            );
        }
        if status == 0 {
            Ok(())
        } else {
            Err("virtual_microphone_signature".into())
        }
    }
    pub async fn install() -> Result<VirtualMicrophoneState, String> {
        let _guard = SETUP.lock().await;
        let root = PREPARED
            .lock()
            .unwrap()
            .take()
            .ok_or("virtual_microphone_prepare_required")?;
        tokio::task::spawn_blocking(move || {
            let package = package();
            verify_files(root.path(), &package)?;
            let installer = root.path().join(&package.installer);
            verify_signature(&installer)?;
            launch_installer(&installer)?;
            device_state()
        })
        .await
        .map_err(|_| "virtual_microphone_install_failed".to_string())?
    }
    fn launch_installer(path: &Path) -> Result<(), String> {
        let path = path.to_owned();
        std::thread::Builder::new()
            .name("mimi-virtual-device-setup".into())
            .spawn(move || launch_installer_sta(&path))
            .map_err(|_| "virtual_microphone_install_failed".to_string())?
            .join()
            .map_err(|_| "virtual_microphone_install_failed".to_string())?
    }
    fn launch_installer_sta(path: &Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::{w, PCWSTR},
            Win32::{
                Foundation::CloseHandle,
                System::{
                    Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
                    Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE},
                },
                UI::{Shell::*, WindowsAndMessaging::SW_SHOWNORMAL},
            },
        };
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|_| "virtual_microphone_install_failed")?;
        struct Apartment(bool);
        impl Drop for Apartment {
            fn drop(&mut self) {
                if self.0 {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }
        let _apartment = Apartment(true);
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let directory: Vec<u16> = path
            .parent()
            .ok_or("virtual_microphone_integrity")?
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(wide.as_ptr()),
            lpDirectory: PCWSTR(directory.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
            if e.code().0 as u32 == 0x800704c7 {
                "virtual_microphone_cancelled"
            } else {
                "virtual_microphone_install_failed"
            }
        })?;
        if info.hProcess.is_invalid() {
            return Err("virtual_microphone_install_failed".into());
        }
        let mut code = 0;
        let exit_result = unsafe {
            WaitForSingleObject(info.hProcess, INFINITE);
            let result = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            result
        };
        exit_result.map_err(|_| "virtual_microphone_install_failed")?;
        if code == 0 || code == 3010 {
            Ok(())
        } else {
            Err("virtual_microphone_install_failed".into())
        }
    }
    pub fn shutdown() {
        EXITING.store(true, Ordering::SeqCst);
        PREPARED.lock().unwrap().take();
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn selects_the_standard_pair_instead_of_the_additional_16_channel_endpoint() {
            assert!(standard_output("CABLE Input (VB-Audio Virtual Cable)"));
            assert!(!standard_output("CABLE In 16ch (VB-Audio Virtual Cable)"));
            assert!(standard_input("CABLE Output (VB-Audio Virtual Cable)"));
            assert!(!standard_input("Microphone (Realtek Audio)"));
        }
        #[test]
        #[ignore = "requires an explicitly installed VB-CABLE; captures only its virtual input"]
        fn native_virtual_pair_carries_synthesized_pcm_without_a_physical_microphone() {
            use cpal::traits::StreamTrait;
            use std::sync::atomic::{AtomicBool, AtomicU32};
            use std::sync::Arc;
            let state = device_state().unwrap();
            assert!(
                state.installed,
                "Install VB-CABLE explicitly before this probe"
            );
            crate::speech_playback::play(
                vec![0; 2400],
                state.output_device_id.as_ref().unwrap(),
                Arc::new(AtomicBool::new(false)),
                false,
                0.2,
            )
            .expect("virtual output without an open capture stream");
            let host = cpal::default_host();
            let input = host
                .input_devices()
                .unwrap()
                .find(|d| d.description().is_ok_and(|v| standard_input(v.name())))
                .unwrap();
            let config = input.default_input_config().unwrap();
            assert_eq!(config.sample_format(), cpal::SampleFormat::F32);
            let frames = Arc::new(AtomicU32::new(0));
            let peak = Arc::new(AtomicU32::new(0));
            let failed = Arc::new(AtomicBool::new(false));
            let count = frames.clone();
            let max = peak.clone();
            let error = failed.clone();
            let stream = input
                .build_input_stream::<f32, _, _>(
                    config.into(),
                    move |data, _| {
                        count.fetch_add(data.len() as u32, Ordering::SeqCst);
                        let value = data.iter().fold(0.0f32, |a, b| a.max(b.abs()));
                        max.fetch_max(value.to_bits(), Ordering::SeqCst);
                    },
                    move |_| {
                        error.store(true, Ordering::SeqCst);
                    },
                    None,
                )
                .unwrap();
            stream.play().unwrap();
            let pcm: Vec<u8> = (0..24000)
                .flat_map(|i| {
                    let sample = ((i as f32 * 440.0 * std::f32::consts::TAU / 24000.0).sin()
                        * 3276.0) as i16;
                    sample.to_le_bytes()
                })
                .collect();
            crate::speech_playback::play(
                pcm,
                state.output_device_id.as_ref().unwrap(),
                Arc::new(AtomicBool::new(false)),
                false,
                0.2,
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(150));
            drop(stream);
            let received = f32::from_bits(peak.load(Ordering::SeqCst));
            assert!(!failed.load(Ordering::SeqCst));
            assert!(frames.load(Ordering::SeqCst) > 24000);
            assert!(
                (0.05..0.2).contains(&received),
                "Expected owned 10% tone, peak={received}"
            );
            println!(
                "virtual_pair=true physical_microphone=false samples={} peak={received}",
                frames.load(Ordering::SeqCst)
            );
        }
        #[test]
        fn pinned_package_has_one_official_origin_and_complete_manifest() {
            let package = package();
            assert_eq!(package.version, "46");
            assert_eq!(
                url::Url::parse(&package.url).unwrap().host_str(),
                Some("download.vb-audio.com")
            );
            assert_eq!(package.files.len(), 31);
            assert!(package.files.contains_key(&package.installer));
            assert!(package.files.contains_key("vbaudio_cable64_win10.cat"));
        }
        #[test]
        fn mutated_archive_is_rejected_before_any_extraction_or_execution() {
            assert_eq!(
                extract(vec![0, 1, 2], &package()).unwrap_err(),
                "virtual_microphone_integrity"
            );
        }
        #[test]
        fn modified_payload_and_additional_files_are_rejected() {
            let root = tempfile::tempdir().unwrap();
            let manifest = Package {
                version: "test".into(),
                url: String::new(),
                archive_sha256: String::new(),
                installer: "setup.exe".into(),
                files: BTreeMap::from([("setup.exe".into(), digest(b"expected"))]),
            };
            std::fs::write(root.path().join("setup.exe"), b"different").unwrap();
            assert!(verify_files(root.path(), &manifest).is_err());
            std::fs::write(root.path().join("setup.exe"), b"expected").unwrap();
            assert!(verify_files(root.path(), &manifest).is_ok());
            std::fs::write(root.path().join("injected.dll"), b"extra").unwrap();
            assert!(verify_files(root.path(), &manifest).is_err());
        }
    }
}
