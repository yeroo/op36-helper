//! Tray icon, its context menu and the commands they produce.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, IMAGE_ICON, LR_LOADFROMFILE, LoadImageW,
    MF_CHECKED, MF_SEPARATOR, MF_STRING, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD,
    TrackPopupMenu, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP,
};
use windows::core::PCWSTR;

use crate::win::msgwindow::WM_APP_TRAY;
use crate::win::to_wide;

pub const MENU_TOGGLE: u32 = 1001;
pub const MENU_OPTIONS: u32 = 1002;
pub const MENU_EXIT: u32 = 1003;

pub(crate) const ICON_ON: &[u8] = include_bytes!("../assets/icon_on.ico");
const ICON_OFF: &[u8] = include_bytes!("../assets/icon_off.ico");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCmd {
    Toggle,
    Options,
    Exit,
}

pub fn menu_command_from_id(id: u32) -> Option<TrayCmd> {
    match id {
        MENU_TOGGLE => Some(TrayCmd::Toggle),
        MENU_OPTIONS => Some(TrayCmd::Options),
        MENU_EXIT => Some(TrayCmd::Exit),
        _ => None,
    }
}

pub struct Tray {
    hwnd: HWND,
    icon_on: HICON,
    icon_off: HICON,
    connected: bool,
}

impl Tray {
    pub fn new(hwnd: HWND, connected: bool) -> Tray {
        let mut tray = Tray {
            hwnd,
            icon_on: load_icon("icon_on.ico", ICON_ON),
            icon_off: load_icon("icon_off.ico", ICON_OFF),
            connected,
        };
        let mut nid = tray.data(NIF_ICON | NIF_MESSAGE | NIF_TIP);
        nid.uCallbackMessage = WM_APP_TRAY;
        set_tip(&mut nid, "OP36 helper");
        // SAFETY: nid is fully initialised above.
        unsafe { let _ = Shell_NotifyIconW(NIM_ADD, &nid); }
        tray.connected = !connected; // force the first set_connected to apply
        tray.set_connected(connected);
        tray
    }

    fn data(&self, flags: windows::Win32::UI::Shell::NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: flags,
            hIcon: if self.connected { self.icon_on } else { self.icon_off },
            ..Default::default()
        }
    }

    pub fn set_connected(&mut self, connected: bool) {
        if self.connected == connected {
            return;
        }
        self.connected = connected;
        let nid = self.data(NIF_ICON);
        // SAFETY: nid is fully initialised.
        unsafe { let _ = Shell_NotifyIconW(NIM_MODIFY, &nid); }
    }

    /// Add the icon again after Explorer restarted (`TaskbarCreated`).
    pub fn readd(&mut self, tip: &str) {
        let mut nid = self.data(NIF_ICON | NIF_MESSAGE | NIF_TIP);
        nid.uCallbackMessage = WM_APP_TRAY;
        set_tip(&mut nid, if tip.is_empty() { "OP36 helper" } else { tip });
        // SAFETY: nid is fully initialised.
        unsafe { let _ = Shell_NotifyIconW(NIM_ADD, &nid); }
    }

    /// A balloon notification from the tray icon (used once per newer release).
    pub fn balloon(&mut self, title: &str, text: &str) {
        let mut nid = self.data(NIF_INFO);
        for (dst, src) in nid.szInfoTitle.iter_mut().zip(title.encode_utf16().take(63)) {
            *dst = src;
        }
        for (dst, src) in nid.szInfo.iter_mut().zip(text.encode_utf16().take(255)) {
            *dst = src;
        }
        // SAFETY: nid is fully initialised.
        unsafe { let _ = Shell_NotifyIconW(NIM_MODIFY, &nid); }
    }

    pub fn set_tooltip(&mut self, text: &str) {
        let mut nid = self.data(NIF_TIP);
        set_tip(&mut nid, text);
        // SAFETY: nid is fully initialised.
        unsafe { let _ = Shell_NotifyIconW(NIM_MODIFY, &nid); }
    }

}

/// What a `WM_APP_TRAY` callback asks for. Left click toggles, double-click opens Options,
/// right click wants the context menu (see [`show_menu`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayClick {
    Command(TrayCmd),
    Menu,
}

pub fn classify_click(lparam_low: u32) -> Option<TrayClick> {
    match lparam_low {
        WM_LBUTTONUP => Some(TrayClick::Command(TrayCmd::Toggle)),
        WM_LBUTTONDBLCLK => Some(TrayClick::Command(TrayCmd::Options)),
        WM_RBUTTONUP => Some(TrayClick::Menu),
        _ => None,
    }
}

/// Show the context menu at the cursor and return the chosen command. This runs a modal message
/// loop, so the caller must not hold any borrow the window procedure needs.
pub fn show_menu(hwnd: HWND, overlay_visible: bool) -> Option<TrayCmd> {
    {
        let toggle = to_wide(if overlay_visible { "Hide overlay" } else { "Show overlay" });
        let options = to_wide("Options…");
        let exit = to_wide("Exit");
        // SAFETY: plain menu API calls; the menu is destroyed before returning.
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            let check = if overlay_visible { MF_CHECKED } else { Default::default() };
            let _ = AppendMenuW(menu, MF_STRING | check, MENU_TOGGLE as usize, PCWSTR(toggle.as_ptr()));
            let _ = AppendMenuW(menu, MF_STRING, MENU_OPTIONS as usize, PCWSTR(options.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT as usize, PCWSTR(exit.as_ptr()));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd); // required for the menu to close when clicking elsewhere
            let id = TrackPopupMenu(
                menu,
                TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD,
                pt.x,
                pt.y,
                Some(0),
                hwnd,
                None,
            );
            let _ = DestroyMenu(menu);
            menu_command_from_id(id.0 as u32)
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let nid = self.data(Default::default());
        // SAFETY: removing our own icon.
        unsafe { let _ = Shell_NotifyIconW(NIM_DELETE, &nid); }
    }
}

fn set_tip(nid: &mut NOTIFYICONDATAW, text: &str) {
    let wide = to_wide(text);
    let n = wide.len().min(nid.szTip.len() - 1);
    nid.szTip[..n].copy_from_slice(&wide[..n]);
    nid.szTip[n] = 0;
}

/// Icons are shipped inside the exe and materialised next to the settings so `LoadImageW` can read them.
pub(crate) fn load_icon(name: &str, bytes: &[u8]) -> HICON {
    let path = crate::paths::app_dir().join(name);
    if std::fs::read(&path).ok().as_deref() != Some(bytes) {
        let _ = std::fs::write(&path, bytes);
    }
    let wide = to_wide(&path.to_string_lossy());
    // SAFETY: loading an icon from a file we just wrote.
    unsafe {
        LoadImageW(None, PCWSTR(wide.as_ptr()), IMAGE_ICON, 0, 0, LR_LOADFROMFILE)
            .map(|h| HICON(h.0))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_classify() {
        assert_eq!(classify_click(WM_LBUTTONUP), Some(TrayClick::Command(TrayCmd::Toggle)));
        assert_eq!(classify_click(WM_LBUTTONDBLCLK), Some(TrayClick::Command(TrayCmd::Options)));
        assert_eq!(classify_click(WM_RBUTTONUP), Some(TrayClick::Menu));
        assert_eq!(classify_click(0x0200 /* WM_MOUSEMOVE */), None);
    }

    #[test]
    fn menu_ids_round_trip_to_commands() {
        assert_eq!(menu_command_from_id(MENU_TOGGLE), Some(TrayCmd::Toggle));
        assert_eq!(menu_command_from_id(MENU_OPTIONS), Some(TrayCmd::Options));
        assert_eq!(menu_command_from_id(MENU_EXIT), Some(TrayCmd::Exit));
        assert_eq!(menu_command_from_id(0), None);
        assert_eq!(menu_command_from_id(1004), None);
    }
}
