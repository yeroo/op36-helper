//! Keyboard layout (language) of the foreground window.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_FOCUS, EVENT_SYSTEM_FOREGROUND, GUITHREADINFO, GetForegroundWindow,
    GetGUIThreadInfo, GetWindowThreadProcessId, PostMessageW, SetTimer, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};

use crate::model::Hkl;
use crate::win::msgwindow::WM_APP_LANG;

/// Timer id for the 250 ms language poll.
pub const TIMER_LANG: usize = 1;
const POLL_MS: u32 = 250;

/// `"EN"`, `"RU"`, or the 4-hex language id for anything else.
pub fn primary_lang_tag(hkl: Hkl) -> String {
    match (hkl as usize) & 0x3FF {
        0x09 => "EN".to_string(),
        0x19 => "RU".to_string(),
        _ => format!("{:04X}", (hkl as usize) & 0xFFFF),
    }
}

/// Keyboard layout of the thread owning the focused control of the foreground window; `0` if none.
pub fn foreground_hkl() -> Hkl {
    // SAFETY: plain window/thread queries with stack structs of the documented sizes.
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return 0;
        }
        let mut tid = GetWindowThreadProcessId(fg, None);
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(tid, &mut info).is_ok()
            && !info.hwndFocus.is_invalid()
            && info.hwndFocus != fg
        {
            let focus_tid = GetWindowThreadProcessId(info.hwndFocus, None);
            if focus_tid != 0 {
                tid = focus_tid;
            }
        }
        GetKeyboardLayout(tid).0 as Hkl
    }
}

static WATCH_HWND: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let target = WATCH_HWND.load(Ordering::Relaxed);
    if target != 0 {
        // SAFETY: posting to our own message window.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(target as *mut _)),
                WM_APP_LANG,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

/// Posts `WM_APP_LANG` on foreground/focus changes and starts a 250 ms poll timer as a safety net.
pub struct LanguageWatcher {
    _hooks: [HWINEVENTHOOK; 2],
    last: Hkl,
}

impl LanguageWatcher {
    pub fn install(hwnd: HWND) -> LanguageWatcher {
        WATCH_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        // SAFETY: out-of-context hooks need no DLL; the callback only posts a message.
        let hooks = unsafe {
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            [
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(on_win_event),
                    0,
                    0,
                    flags,
                ),
                SetWinEventHook(
                    EVENT_OBJECT_FOCUS,
                    EVENT_OBJECT_FOCUS,
                    None,
                    Some(on_win_event),
                    0,
                    0,
                    flags,
                ),
            ]
        };
        // SAFETY: timer on our own window.
        unsafe {
            SetTimer(Some(hwnd), TIMER_LANG, POLL_MS, None);
        }
        LanguageWatcher {
            _hooks: hooks,
            last: 0,
        }
    }

    /// Re-read the foreground layout; `Some(hkl)` when it changed.
    pub fn poll(&mut self) -> Option<Hkl> {
        let now = foreground_hkl();
        if now != 0 && now != self.last {
            self.last = now;
            Some(now)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_lang_tag_maps_known_ids() {
        assert_eq!(primary_lang_tag(0x04090409), "EN");
        assert_eq!(primary_lang_tag(0x04190419), "RU");
        assert_eq!(primary_lang_tag(0x04070407), "0407");
        assert_eq!(primary_lang_tag(0xF0C00409), "EN"); // layout-id form still carries the language
    }
}
