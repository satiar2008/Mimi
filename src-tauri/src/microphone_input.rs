//! Explicit, cancelable insertion; no clipboard, target text reads or focus changes.
use crate::session_manager::SessionManager;
use std::hash::{Hash, Hasher};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

static BUSY: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(0);
static CONSUMED: Mutex<Option<u64>> = Mutex::new(None);
struct Pending;
impl Drop for Pending {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

pub fn cancel() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
}

pub async fn insert(session: Arc<SessionManager>) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err("microphone_input_unsupported".into());
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("microphone_input_busy".into());
    }
    let pending = Pending;
    let epoch = EPOCH.load(Ordering::SeqCst);
    let candidate = session
        .microphone_insertion_candidate()
        .ok_or("microphone_input_no_final")?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    candidate.1.hash(&mut hash);
    let key = hash.finish();
    if *CONSUMED.lock().unwrap() == Some(key) {
        return Err("microphone_input_duplicate".into());
    }
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    tokio::task::spawn_blocking(move || {
        let _pending = pending;
        let check = || -> Result<(), String> {
            if EPOCH.load(Ordering::SeqCst) != epoch {
                return Err("microphone_input_cancelled".into());
            }
            if session.microphone_insertion_candidate().as_ref() != Some(&candidate) {
                return Err("microphone_input_changed".into());
            }
            Ok(())
        };
        check()?;
        native_insert(&candidate.1.text, || {
            check()?;
            // Once injection is attempted, never automatically replay partial input.
            *CONSUMED.lock().unwrap() = Some(key);
            Ok(())
        })
    })
    .await
    .map_err(|_| "microphone_input_failed".to_string())?
}

#[cfg(not(target_os = "windows"))]
fn native_insert(_: &str, _: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    Err("microphone_input_unsupported".into())
}

#[cfg(target_os = "windows")]
fn native_insert(text: &str, commit: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    use windows::Win32::{
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_MULTITHREADED,
            },
            Variant::{VariantClear, VariantToBoolean, VT_BOOL},
        },
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationTextPattern, IUIAutomationValuePattern,
                UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_IsReadOnlyAttributeId,
                UIA_TextPatternId, UIA_ValuePatternId,
            },
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN,
                VK_SHIFT,
            },
            WindowsAndMessaging::GetForegroundWindow,
        },
    };
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let unavailable = |_| "microphone_input_target_unavailable".to_string();
    // This dedicated blocking thread owns its COM apartment and all UIA objects.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(unavailable)?;
        let _com = Com;
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(unavailable)?;
        let foreground = GetForegroundWindow();
        let element = automation.GetFocusedElement().map_err(unavailable)?;
        if foreground.is_invalid()
            || element.CurrentProcessId().map_err(unavailable)? as u32 == std::process::id()
            || element.CurrentIsPassword().map_err(unavailable)?.as_bool()
            || !element.CurrentIsEnabled().map_err(unavailable)?.as_bool()
            || !element
                .CurrentHasKeyboardFocus()
                .map_err(unavailable)?
                .as_bool()
            || ![UIA_EditControlTypeId, UIA_DocumentControlTypeId]
                .contains(&element.CurrentControlType().map_err(unavailable)?)
        {
            return Err("microphone_input_target_unavailable".into());
        }
        let writable = if let Ok(value) =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            !value.CurrentIsReadOnly().map_err(unavailable)?.as_bool()
        } else {
            // RichEdit/Notepad can expose editable TextPattern without ValuePattern.
            // Query only the read-only attribute; never fetch target document text.
            let pattern: IUIAutomationTextPattern = element
                .GetCurrentPatternAs(UIA_TextPatternId)
                .map_err(unavailable)?;
            let mut attribute = pattern
                .DocumentRange()
                .map_err(unavailable)?
                .GetAttributeValue(UIA_IsReadOnlyAttributeId)
                .map_err(unavailable)?;
            let writable = attribute.Anonymous.Anonymous.vt == VT_BOOL
                && VariantToBoolean(&attribute).is_ok_and(|read_only| !read_only.as_bool());
            let _ = VariantClear(&mut attribute);
            writable
        };
        if !writable {
            return Err("microphone_input_target_unavailable".into());
        }
        let input: Vec<INPUT> = text
            .encode_utf16()
            .flat_map(|unit| {
                [false, true].map(|up| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wScan: unit,
                            dwFlags: KEYEVENTF_UNICODE
                                | if up {
                                    KEYEVENTF_KEYUP
                                } else {
                                    Default::default()
                                },
                            ..Default::default()
                        },
                    },
                })
            })
            .collect();
        // Recheck identity without moving focus, selecting text, or reading its value.
        let current = automation.GetFocusedElement().map_err(unavailable)?;
        if GetForegroundWindow() != foreground
            || !automation
                .CompareElements(&element, &current)
                .map_err(unavailable)?
                .as_bool()
            || [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
                .iter()
                .any(|key| GetAsyncKeyState(key.0 as i32) < 0)
        {
            return Err("microphone_input_target_changed".into());
        }
        commit()?;
        if SendInput(&input, std::mem::size_of::<INPUT>() as i32) != input.len() as u32 {
            return Err("microphone_input_incomplete".into());
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    /// Manual native acceptance against a disposable, explicitly named editor.
    /// CI must not inject input into whichever application happens to be active.
    #[test]
    #[ignore = "requires the focused Mimi microphone insertion fixture"]
    fn windows_microphone_insertion_fixture() {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};
        let case = std::env::var("MIMI_MICROPHONE_INPUT_CASE").expect("explicit fixture case");
        assert!(["writable", "readonly", "password", "cancelled"].contains(&case.as_str()));
        println!("Focus the disposable fixture field within 8 seconds.");
        std::thread::sleep(std::time::Duration::from_secs(8));
        let mut title = [0u16; 128];
        let length = unsafe { GetWindowTextW(GetForegroundWindow(), &mut title) };
        assert_eq!(
            String::from_utf16_lossy(&title[..length as usize]),
            "Mimi microphone insertion fixture"
        );
        let text = mimi_runtime::core::microphone_input::insertion_text(
            "سلام Mimi (GPT-4o) 12:34 👋\nnext",
        )
        .unwrap();
        let result = super::native_insert(&text, || {
            if case == "cancelled" {
                Err("microphone_input_cancelled".into())
            } else {
                Ok(())
            }
        });
        match case.as_str() {
            "writable" => assert_eq!(result, Ok(())),
            "cancelled" => assert_eq!(result, Err("microphone_input_cancelled".into())),
            _ => assert_eq!(result, Err("microphone_input_target_unavailable".into())),
        }
    }
}
