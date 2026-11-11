//! Direction of the existing Win32 tray menu, independent of Windows' UI locale.
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetMenuItemCount, GetMenuItemInfoW, SetMenuItemInfoW, HMENU, MENUITEMINFOW, MFT_RIGHTJUSTIFY,
    MFT_RIGHTORDER, MFT_SEPARATOR, MIIM_FTYPE, MIIM_STRING, MIIM_SUBMENU,
};

pub fn apply(menu: &tauri::menu::Menu<tauri::Wry>, rtl: bool) -> Result<(), &'static str> {
    use tauri::menu::ContextMenu;
    let handle = menu
        .hpopupmenu()
        .map_err(|_| "tray_menu_handle_unavailable")?;
    // Called on the UI thread while Tauri retains ownership of the entire menu.
    unsafe { apply_handle(handle as HMENU, rtl) }
}

unsafe fn apply_handle(menu: HMENU, rtl: bool) -> Result<(), &'static str> {
    let count = GetMenuItemCount(menu);
    if count < 0 {
        return Err("tray_menu_items_unavailable");
    }
    for position in 0..count as u32 {
        let mut item = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STRING | MIIM_SUBMENU,
            ..Default::default()
        };
        if GetMenuItemInfoW(menu, position, 1, &mut item) == 0 {
            return Err("tray_menu_item_unavailable");
        }
        if !item.hSubMenu.is_null() {
            apply_handle(item.hSubMenu, rtl)?;
        }
        if item.fType & MFT_SEPARATOR != 0 {
            continue;
        }
        let mut text = vec![0u16; item.cch as usize + 1];
        item.dwTypeData = text.as_mut_ptr();
        item.cch = text.len() as u32;
        if GetMenuItemInfoW(menu, position, 1, &mut item) == 0 {
            return Err("tray_menu_text_unavailable");
        }
        let text = String::from_utf16_lossy(&text[..item.cch as usize]);
        let plain = text
            .strip_prefix('\u{202b}')
            .and_then(|s| s.strip_suffix('\u{202c}'))
            .unwrap_or(&text);
        // GDI menus displayed the newer RLI/PDI isolates as missing glyphs in
        // native QA. Legacy RLE/PDF embeddings preserve mixed labels invisibly.
        let display = if rtl {
            format!("\u{202b}{plain}\u{202c}")
        } else {
            plain.to_string()
        };
        let mut display: Vec<u16> = display.encode_utf16().chain(Some(0)).collect();
        item.fMask = MIIM_FTYPE | MIIM_STRING;
        let direction = MFT_RIGHTJUSTIFY | MFT_RIGHTORDER;
        item.fType = if rtl {
            item.fType | direction
        } else {
            item.fType & !direction
        };
        item.dwTypeData = display.as_mut_ptr();
        item.cch = (display.len() - 1) as u32;
        if SetMenuItemInfoW(menu, position, 1, &item) == 0 {
            return Err("tray_menu_direction_unavailable");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, MFS_CHECKED, MF_CHECKED, MF_POPUP, MF_STRING,
        MIIM_ID, MIIM_STATE,
    };

    #[test]
    fn native_menu_rtl_round_trip_preserves_text_ids_submenu_and_checkmark() {
        unsafe {
            let menu = CreatePopupMenu();
            let submenu = CreatePopupMenu();
            assert!(!menu.is_null() && !submenu.is_null());
            let original: Vec<u16> = "خروج از Mimi".encode_utf16().chain(Some(0)).collect();
            assert_ne!(
                AppendMenuW(submenu, MF_STRING | MF_CHECKED, 42, original.as_ptr()),
                0
            );
            assert_ne!(
                AppendMenuW(menu, MF_POPUP, submenu as usize, original.as_ptr()),
                0
            );
            for rtl in [true, true, false, true, false] {
                apply_handle(menu, rtl).unwrap();
                for handle in [menu, submenu] {
                    let mut text = [0u16; 128];
                    let mut item = MENUITEMINFOW {
                        cbSize: size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_FTYPE | MIIM_STRING | MIIM_SUBMENU | MIIM_STATE | MIIM_ID,
                        dwTypeData: text.as_mut_ptr(),
                        cch: text.len() as u32,
                        ..Default::default()
                    };
                    assert_ne!(GetMenuItemInfoW(handle, 0, 1, &mut item), 0);
                    assert_eq!(item.fType & MFT_RIGHTORDER != 0, rtl);
                    let text = String::from_utf16_lossy(&text[..item.cch as usize]);
                    assert_eq!(
                        text,
                        if rtl {
                            "\u{202b}خروج از Mimi\u{202c}"
                        } else {
                            "خروج از Mimi"
                        }
                    );
                    if handle == menu {
                        assert_eq!(item.hSubMenu, submenu);
                    } else {
                        assert_eq!(item.wID, 42);
                        assert_ne!(item.fState & MFS_CHECKED, 0);
                    }
                }
            }
            assert_ne!(DestroyMenu(menu), 0);
        }
    }
}
