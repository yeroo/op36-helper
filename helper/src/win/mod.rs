//! Small Win32 helpers shared by the window, tray and overlay code.

pub mod msgwindow;

use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

/// NUL-terminated UTF-16 for `PCWSTR` arguments that outlive the call.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn hinstance() -> HINSTANCE {
    // SAFETY: GetModuleHandleW(None) never fails for the running executable.
    unsafe {
        GetModuleHandleW(None)
            .map(|h| HINSTANCE(h.0))
            .unwrap_or_default()
    }
}
