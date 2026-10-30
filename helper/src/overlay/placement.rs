//! Which monitor and where on it the overlay goes.

use crate::settings::{Anchor, Settings};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// Device name such as `\\.\DISPLAY1` (stable across sessions for the same output).
    pub name: String,
    /// `(left, top, right, bottom)` of the work area (desktop minus taskbar), in screen pixels.
    pub work_area: (i32, i32, i32, i32),
    /// Full monitor rectangle.
    pub monitor_rect: (i32, i32, i32, i32),
    pub is_primary: bool,
    pub dpi: u32,
}

/// Enumerate attached monitors with their work areas and DPI.
pub fn monitors() -> Vec<MonitorInfo> {
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::core::BOOL;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
    };
    const MONITORINFOF_PRIMARY: u32 = 1;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

    unsafe extern "system" fn cb(hmon: HMONITOR, _hdc: HDC, _rect: *mut RECT, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the `*mut Vec<MonitorInfo>` passed below; info struct is sized.
        unsafe {
            let out = &mut *(lparam.0 as *mut Vec<MonitorInfo>);
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(hmon, &mut info.monitorInfo).as_bool() {
                let name_len = info.szDevice.iter().position(|c| *c == 0).unwrap_or(info.szDevice.len());
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                let r = info.monitorInfo.rcWork;
                let m = info.monitorInfo.rcMonitor;
                out.push(MonitorInfo {
                    name: String::from_utf16_lossy(&info.szDevice[..name_len]),
                    work_area: (r.left, r.top, r.right, r.bottom),
                    monitor_rect: (m.left, m.top, m.right, m.bottom),
                    is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                    dpi: dx,
                });
            }
            BOOL(1)
        }
    }

    let mut out: Vec<MonitorInfo> = Vec::new();
    // SAFETY: callback only pushes into `out`, which outlives the call.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

#[cfg(test)] // only tests use this
/// Top-left corner for an overlay of `size` on the monitor the settings ask for (primary when the
/// named one is absent), honouring anchor and offset. Never mutates `settings`.
pub fn place<'m>(settings: &Settings, size: (i32, i32), monitors: &'m [MonitorInfo]) -> (i32, i32, &'m MonitorInfo) {
    place_with_reserved(settings, size, monitors, None)
}

/// The monitor the settings select (primary when absent).
pub fn chosen_monitor<'m>(settings: &Settings, monitors: &'m [MonitorInfo]) -> &'m MonitorInfo {
    settings
        .monitor
        .as_deref()
        .and_then(|name| monitors.iter().find(|m| m.name == name))
        .or_else(|| monitors.iter().find(|m| m.is_primary))
        .or_else(|| monitors.first())
        .expect("at least one monitor")
}

/// Like [`place`], but when this app itself has an app-bar strip of `thickness` px reserved on
/// `monitor_name`, the work area Windows reports already excludes it; add it back so the overlay
/// lands inside its own strip instead of climbing above it.
pub fn place_with_reserved<'m>(
    settings: &Settings,
    size: (i32, i32),
    monitors: &'m [MonitorInfo],
    reserved: Option<(&str, i32)>,
) -> (i32, i32, &'m MonitorInfo) {
    let chosen = chosen_monitor(settings, monitors);
    let (l, mut t, r, mut b) = chosen.work_area;
    if let Some((name, thickness)) = reserved {
        if name == chosen.name {
            match settings.anchor {
                Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => b += thickness,
                Anchor::TopLeft | Anchor::Top | Anchor::TopRight => t -= thickness,
                _ => {}
            }
        }
    }
    let (w, h) = size;
    let (ox, oy) = settings.offset;
    let x = match settings.anchor {
        Anchor::TopLeft | Anchor::Left | Anchor::BottomLeft => l + ox,
        Anchor::Top | Anchor::Center | Anchor::Bottom => (l + r - w) / 2 + ox,
        Anchor::TopRight | Anchor::Right | Anchor::BottomRight => r - w - ox,
    };
    let y = match settings.anchor {
        Anchor::TopLeft | Anchor::Top | Anchor::TopRight => t + oy,
        Anchor::Left | Anchor::Center | Anchor::Right => (t + b - h) / 2 + oy,
        Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => b - h - oy,
    };
    (x, y, chosen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Anchor, Settings};

    fn monitor(name: &str, w: i32, h: i32, primary: bool) -> MonitorInfo {
        MonitorInfo {
            name: name.to_string(),
            work_area: (0, 0, w, h),
            monitor_rect: (0, 0, w, h + 40),
            is_primary: primary,
            dpi: 96,
        }
    }

    #[test]
    fn placement_bottom_center_default() {
        let mons = vec![monitor(r"\\.\DISPLAY1", 1920, 1040, true)];
        let (x, y, m) = place(&Settings::default(), (600, 300), &mons);
        assert_eq!((x, y), (660, 1040 - 300 - 8));
        assert!(m.is_primary);
    }

    #[test]
    fn placement_falls_back_to_primary() {
        let mons = vec![monitor(r"\\.\DISPLAY1", 1920, 1040, true)];
        let mut s = Settings::default();
        s.monitor = Some(r"\\.\DISPLAY9".to_string());
        let (_, _, m) = place(&s, (600, 300), &mons);
        assert_eq!(m.name, r"\\.\DISPLAY1");
        assert_eq!(s.monitor.as_deref(), Some(r"\\.\DISPLAY9"), "settings untouched");
    }

    #[test]
    fn placement_uses_named_monitor_and_anchor() {
        let mut second = monitor(r"\\.\DISPLAY2", 1000, 500, false);
        second.work_area = (1920, 100, 2920, 600);
        let mons = vec![monitor(r"\\.\DISPLAY1", 1920, 1040, true), second];
        let mut s = Settings::default();
        s.monitor = Some(r"\\.\DISPLAY2".to_string());
        s.anchor = Anchor::TopRight;
        s.offset = (10, 20);
        let (x, y, m) = place(&s, (200, 100), &mons);
        assert_eq!(m.name, r"\\.\DISPLAY2");
        assert_eq!((x, y), (2920 - 200 - 10, 100 + 20));
    }

    #[test]
    fn placement_compensates_for_own_reserved_strip() {
        // With an app bar of 300 px registered at the bottom, Windows reports a work area that
        // already excludes it; placement must add it back so the overlay sits in its own strip.
        let mut m = monitor("p", 1920, 1040, true);
        m.work_area = (0, 0, 1920, 740);
        let mons = vec![m];
        let s = Settings::default();
        let (_, y, _) = place_with_reserved(&s, (600, 300), &mons, Some(("p", 300)));
        assert_eq!(y, 1040 - 300 - 8);
        let (_, y2, _) = place(&s, (600, 300), &mons);
        assert_eq!(y2, 740 - 300 - 8);
    }

    #[test]
    fn placement_center() {
        let mons = vec![monitor("p", 1000, 1000, true)];
        let mut s = Settings::default();
        s.anchor = Anchor::Center;
        s.offset = (0, 0);
        assert_eq!(place(&s, (200, 100), &mons).0, 400);
        assert_eq!(place(&s, (200, 100), &mons).1, 450);
    }
}
