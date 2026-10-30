//! Layered, click-through, always-on-top overlay window.

use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HMENU, HWND_TOPMOST, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::PCWSTR;

use super::BgraBitmap;
use crate::win::{hinstance, to_wide};

const CLASS_NAME: &str = "Op36HelperOverlay";

unsafe extern "system" fn overlay_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: forwarding to the default procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub struct OverlayWindow {
    hwnd: HWND,
    x: i32,
    y: i32,
}

impl OverlayWindow {
    pub fn create() -> windows::core::Result<OverlayWindow> {
        let class = to_wide(CLASS_NAME);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(overlay_wndproc),
            hInstance: hinstance(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        // SAFETY: plain class registration; a duplicate registration returns 0 harmlessly.
        unsafe { RegisterClassW(&wc) };
        // SAFETY: standard window creation with the spec's styles.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                PCWSTR(class.as_ptr()),
                PCWSTR(class.as_ptr()),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None::<HMENU>,
                Some(hinstance()),
                None,
            )?
        };
        Ok(OverlayWindow { hwnd, x: 0, y: 0 })
    }

    pub fn show(&self, visible: bool) {
        // SAFETY: own window handle.
        unsafe {
            let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
        if visible {
            self.reassert_topmost();
        }
    }

    /// Screen position of the top-left corner; the size always follows the presented bitmap.
    pub fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        // SAFETY: own window handle.
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    pub fn reassert_topmost(&self) {
        // SAFETY: own window handle.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    /// Push a premultiplied bitmap as the window's content (and size).
    pub fn present(&self, frame: &BgraBitmap) -> windows::core::Result<()> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: frame.width,
                biHeight: -frame.height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: GDI resource lifecycle is fully contained in this function; every created object is
        // released before returning.
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            std::ptr::copy_nonoverlapping(
                frame.premultiplied.as_ptr(),
                bits as *mut u32,
                frame.premultiplied.len(),
            );
            let old = SelectObject(mem, bitmap.into());
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let dst = POINT { x: self.x, y: self.y };
            let size = SIZE {
                cx: frame.width,
                cy: frame.height,
            };
            let src = POINT::default();
            let result = UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                Some(&dst),
                Some(&size),
                Some(mem),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            SelectObject(mem, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            result
        }
    }
}
