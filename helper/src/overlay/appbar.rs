//! Reserve screen space like the taskbar does, so maximized windows stop at the overlay's edge.

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::WM_APP;

use crate::settings::Anchor;

/// Notification message the shell sends to the app bar window.
pub const WM_APP_APPBAR: u32 = WM_APP + 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

/// The edge an anchor reserves, or `None` for anchors in the middle of the screen.
pub fn edge_for(anchor: Anchor) -> Option<Edge> {
    match anchor {
        Anchor::TopLeft | Anchor::Top | Anchor::TopRight => Some(Edge::Top),
        Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => Some(Edge::Bottom),
        _ => None,
    }
}

/// Strip to reserve on `monitor` (full monitor rect): full width, `thickness` px at `edge`.
pub fn strip_rect(monitor: (i32, i32, i32, i32), edge: Edge, thickness: i32) -> RECT {
    let (l, t, r, b) = monitor;
    match edge {
        Edge::Top => RECT { left: l, top: t, right: r, bottom: t + thickness },
        Edge::Bottom => RECT { left: l, top: b - thickness, right: r, bottom: b },
    }
}

pub struct AppBar {
    hwnd: HWND,
    registered: bool,
    /// What is currently reserved: `(monitor name, edge, thickness)`.
    pub current: Option<(String, Edge, i32)>,
}

impl AppBar {
    pub fn new(hwnd: HWND) -> AppBar {
        AppBar { hwnd, registered: false, current: None }
    }

    fn data(&self) -> APPBARDATA {
        APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd,
            uCallbackMessage: WM_APP_APPBAR,
            ..Default::default()
        }
    }

    /// Reserve `thickness` px at `edge` of the monitor; re-registers as needed.
    pub fn reserve(&mut self, monitor_name: &str, monitor: (i32, i32, i32, i32), edge: Edge, thickness: i32) {
        if self.current.as_ref().is_some_and(|(n, e, t)| n == monitor_name && *e == edge && *t == thickness) {
            return;
        }
        let mut abd = self.data();
        // SAFETY: APPBARDATA is fully initialised; the shell copies what it needs.
        unsafe {
            if !self.registered {
                self.registered = SHAppBarMessage(ABM_NEW, &mut abd) != 0;
                if !self.registered {
                    log::warn!("app bar registration failed");
                    return;
                }
            }
            abd.uEdge = if edge == Edge::Top { ABE_TOP } else { ABE_BOTTOM };
            abd.rc = strip_rect(monitor, edge, thickness);
            SHAppBarMessage(ABM_QUERYPOS, &mut abd);
            // Keep our thickness after the shell adjusted the rect for other bars.
            match edge {
                Edge::Top => abd.rc.bottom = abd.rc.top + thickness,
                Edge::Bottom => abd.rc.top = abd.rc.bottom - thickness,
            }
            SHAppBarMessage(ABM_SETPOS, &mut abd);
        }
        self.current = Some((monitor_name.to_string(), edge, thickness));
        log::info!("app bar reserved {thickness} px at {edge:?} of {monitor_name}");
    }

    /// Give the space back.
    pub fn release(&mut self) {
        if self.registered {
            let mut abd = self.data();
            // SAFETY: removing our own app bar.
            unsafe {
                SHAppBarMessage(ABM_REMOVE, &mut abd);
            }
            self.registered = false;
            log::info!("app bar released");
        }
        self.current = None;
    }

    /// Shell notification (position changed, full-screen app, …): re-apply the current reservation.
    pub fn on_notification(&mut self, _wparam: usize, _lparam: LPARAM, monitor: Option<(i32, i32, i32, i32)>) {
        if let (Some((name, edge, thickness)), Some(rect)) = (self.current.clone(), monitor) {
            self.current = None;
            self.reserve(&name, rect, edge, thickness);
        }
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_is_full_width_at_the_edge() {
        let r = strip_rect((100, 0, 2020, 1200), Edge::Bottom, 270);
        assert_eq!((r.left, r.top, r.right, r.bottom), (100, 930, 2020, 1200));
        let r = strip_rect((0, 0, 1920, 1200), Edge::Top, 50);
        assert_eq!((r.left, r.top, r.right, r.bottom), (0, 0, 1920, 50));
    }

    #[test]
    fn only_edge_anchors_reserve_space() {
        assert_eq!(edge_for(Anchor::Bottom), Some(Edge::Bottom));
        assert_eq!(edge_for(Anchor::TopRight), Some(Edge::Top));
        assert_eq!(edge_for(Anchor::Center), None);
        assert_eq!(edge_for(Anchor::Left), None);
    }
}
