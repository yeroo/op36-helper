//! Hidden top-level window that receives tray, hotkey, raw-input and cross-thread messages.
//!
//! It is a real (never shown) top-level window, not a message-only one: message-only windows do
//! not receive broadcasts such as `WM_SETTINGCHANGE`, `WM_DISPLAYCHANGE` or `TaskbarCreated`, and
//! the shell's app-bar notifications need a window the shell can see too.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW, HMENU,
    RegisterClassW, SetWindowLongPtrW, WINDOW_EX_STYLE, WM_APP, WM_NCCREATE, WNDCLASSW, WS_POPUP,
};
use windows::core::PCWSTR;

use super::{hinstance, to_wide};

/// Window class name; a second instance finds the first one by it.
pub const CLASS_NAME: &str = "Op36HelperMsg";

/// Toggle overlay visibility (sent by a second instance or the hotkey path).
pub const WM_APP_TOGGLE: u32 = WM_APP + 1;
/// Tray icon callback message (`NOTIFYICONDATAW::uCallbackMessage`).
pub const WM_APP_TRAY: u32 = WM_APP + 2;
/// HID thread → UI thread: `LPARAM` is a `Box<HidEvent>` pointer.
pub const WM_APP_HID: u32 = WM_APP + 3;
/// Foreground keyboard layout changed.
pub const WM_APP_LANG: u32 = WM_APP + 5;
/// Flash thread → UI thread: `LPARAM` is a `Box<FlashStep>` pointer.
pub const WM_APP_FLASH: u32 = WM_APP + 6;
/// Update thread → UI thread: `LPARAM` is a `Box<UpdateEvent>` pointer.
pub const WM_APP_UPDATE: u32 = WM_APP + 7;

pub type Handler = Box<dyn FnMut(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>>;

pub struct MsgWindow {
    hwnd: HWND,
    // Keeps the handler alive for the window's lifetime; the window holds a raw pointer to it.
    _handler: Box<Handler>,
}

impl MsgWindow {
    pub fn create(handler: Handler) -> windows::core::Result<MsgWindow> {
        let class = to_wide(CLASS_NAME);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        // SAFETY: registering twice (second instance in the same process) just returns 0, which we ignore.
        unsafe { RegisterClassW(&wc) };

        let mut boxed: Box<Handler> = Box::new(handler);
        let param: *mut Handler = &mut *boxed;
        // SAFETY: standard window creation; `param` is stored in GWLP_USERDATA by WM_NCCREATE and
        // stays valid because `boxed` lives inside the returned struct.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR(class.as_ptr()),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None::<HMENU>,
                Some(hinstance()),
                Some(param as *const c_void),
            )?
        };
        Ok(MsgWindow { hwnd, _handler: boxed })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for MsgWindow {
    fn drop(&mut self) {
        // The handler box dies with us; the window must not outlive it.
        // SAFETY: own window on the current thread.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds the `*mut Handler` set below; it outlives the window.
    unsafe {
        if msg == WM_NCCREATE {
            let cs = lparam.0 as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
        }
        let handler = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Handler;
        if !handler.is_null() {
            if let Some(result) = (*handler)(hwnd, msg, wparam, lparam) {
                return result;
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}
