//! The overlay window on X11 (and on Wayland desktops through XWayland, which GNOME starts on
//! demand): a 32-bit ARGB dock window that is never focused, takes no input (empty input shape, so
//! clicks fall through), sits on every workspace above normal windows, and can reserve a strip of
//! the screen with `_NET_WM_STRUT_PARTIAL`. Monitors come from RandR, work areas from
//! `_NET_WORKAREA`, the display scale from `Xft.dpi`.

use std::sync::Arc;

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _, CreateGCAux,
    CreateWindowAux, EventMask, ImageFormat, PropMode, VisualClass, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::overlay::BgraBitmap;
use crate::overlay::placement::MonitorInfo;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DOCK,
        _NET_WM_STATE,
        _NET_WM_STATE_ABOVE,
        _NET_WM_STATE_STICKY,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_DESKTOP,
        _NET_WM_STRUT,
        _NET_WM_STRUT_PARTIAL,
        _NET_WM_NAME,
        _NET_WM_PID,
        _NET_WORKAREA,
        _NET_WM_BYPASS_COMPOSITOR,
        WM_PROTOCOLS,
        UTF8_STRING,
        RESOURCE_MANAGER,
    }
}

/// Which screen edge a reservation sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
}

pub struct Overlay {
    conn: Arc<RustConnection>,
    atoms: Atoms,
    root: u32,
    window: u32,
    pixmap: Option<(u32, u16, u16)>,
    gc: u32,
    depth: u8,
    shown: bool,
    pos: (i32, i32),
    size: (u16, u16),
    root_size: (u16, u16),
    /// The strip currently reserved: `(edge, start x, end x, thickness)` in root coordinates.
    strut: Option<(Edge, i32, i32, i32)>,
}

impl Overlay {
    /// Connect to the X server (`$DISPLAY`) and create the hidden overlay window.
    pub fn create() -> Result<Overlay, String> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|e| {
            format!("cannot open the X display ({e}); on Wayland the overlay needs XWayland")
        })?;
        let conn = Arc::new(conn);
        let screen = conn.setup().roots[screen_num].clone();
        let atoms = Atoms::new(&*conn)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        // A 32-bit TrueColor visual gives per-pixel alpha with a compositor (every Wayland desktop,
        // and GNOME / KDE on X11).
        let visual = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| d.visuals.iter())
            .find(|v| v.class == VisualClass::TRUE_COLOR)
            .map(|v| v.visual_id)
            .ok_or("the X server offers no 32-bit ARGB visual")?;
        let colormap = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_colormap(
            x11rb::protocol::xproto::ColormapAlloc::NONE,
            colormap,
            screen.root,
            visual,
        )
        .map_err(|e| e.to_string())?;
        let window = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            32,
            window,
            screen.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(0)
                .border_pixel(0)
                .colormap(colormap)
                .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY),
        )
        .map_err(|e| e.to_string())?;
        let gc = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_gc(gc, window, &CreateGCAux::new())
            .map_err(|e| e.to_string())?;

        let set32 = |prop: u32, ty: AtomEnum, data: &[u32]| {
            let _ = conn.change_property32(PropMode::REPLACE, window, prop, ty, data);
        };
        set32(
            atoms._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            &[atoms._NET_WM_WINDOW_TYPE_DOCK],
        );
        set32(
            atoms._NET_WM_STATE,
            AtomEnum::ATOM,
            &[
                atoms._NET_WM_STATE_ABOVE,
                atoms._NET_WM_STATE_STICKY,
                atoms._NET_WM_STATE_SKIP_TASKBAR,
                atoms._NET_WM_STATE_SKIP_PAGER,
            ],
        );
        set32(atoms._NET_WM_DESKTOP, AtomEnum::CARDINAL, &[0xFFFF_FFFF]);
        set32(atoms._NET_WM_PID, AtomEnum::CARDINAL, &[std::process::id()]);
        // WM_HINTS: input = False (never take the keyboard focus).
        set32(AtomEnum::WM_HINTS.into(), AtomEnum::WM_HINTS, &[1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let _ = conn.change_property8(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_NAME,
            atoms.UTF8_STRING,
            b"OP36 Helper overlay",
        );
        let _ = conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"op36-helper\0OP36 Helper\0",
        );
        // Click-through: an empty input region.
        if conn
            .extension_information(shape::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some()
        {
            let _ = conn.shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                x11rb::protocol::xproto::ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &[],
            );
        } else {
            log::warn!("X SHAPE extension missing: the overlay will catch clicks");
        }
        // Monitor and work-area changes wake the main loop.
        let _ = conn.change_window_attributes(
            screen.root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        );
        let _ = conn.randr_select_input(
            screen.root,
            randr::NotifyMask::SCREEN_CHANGE | randr::NotifyMask::CRTC_CHANGE | randr::NotifyMask::OUTPUT_CHANGE,
        );
        conn.flush().map_err(|e| e.to_string())?;
        Ok(Overlay {
            conn,
            atoms,
            root: screen.root,
            window,
            pixmap: None,
            gc,
            depth: 32,
            shown: false,
            pos: (0, 0),
            size: (1, 1),
            root_size: (screen.width_in_pixels, screen.height_in_pixels),
            strut: None,
        })
    }

    /// Run `on_change` on a thread whenever monitors, the work area or X resources change.
    pub fn watch_display(&self, on_change: impl Fn() + Send + 'static) {
        let conn = Arc::clone(&self.conn);
        let interesting = [self.atoms._NET_WORKAREA, self.atoms.RESOURCE_MANAGER];
        std::thread::Builder::new()
            .name("x11-events".into())
            .spawn(move || {
                loop {
                    match conn.wait_for_event() {
                        Ok(Event::RandrScreenChangeNotify(_)) | Ok(Event::RandrNotify(_)) => {
                            on_change()
                        }
                        Ok(Event::PropertyNotify(p)) if interesting.contains(&p.atom) => {
                            on_change()
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::warn!("X connection lost: {e}");
                            return;
                        }
                    }
                }
            })
            .expect("spawn x11 event thread");
    }

    pub fn show(&mut self, show: bool) {
        if show == self.shown {
            return;
        }
        self.shown = show;
        if show {
            let _ = self.conn.map_window(self.window);
            // Window managers may place a newly mapped window themselves; insist on ours.
            let _ = self.conn.configure_window(
                self.window,
                &ConfigureWindowAux::new().x(self.pos.0).y(self.pos.1),
            );
        } else {
            let _ = self.conn.unmap_window(self.window);
        }
        let _ = self.conn.flush();
    }

    pub fn set_position(&mut self, x: i32, y: i32) {
        if (x, y) == self.pos {
            return;
        }
        self.pos = (x, y);
        let _ = self
            .conn
            .configure_window(self.window, &ConfigureWindowAux::new().x(x).y(y));
        self.set_size_hints();
        let _ = self.conn.flush();
    }

    /// WM_NORMAL_HINTS with a user-set position and a fixed size, so the WM keeps both.
    fn set_size_hints(&self) {
        const US_POSITION: u32 = 1;
        const P_POSITION: u32 = 4;
        const P_MIN_SIZE: u32 = 16;
        const P_MAX_SIZE: u32 = 32;
        let (w, h) = (self.size.0 as u32, self.size.1 as u32);
        let hints = [
            US_POSITION | P_POSITION | P_MIN_SIZE | P_MAX_SIZE,
            self.pos.0 as u32,
            self.pos.1 as u32,
            w,
            h,
            w,
            h,
            w,
            h,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            self.window,
            AtomEnum::WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS,
            &hints,
        );
    }

    /// Show `bitmap`: it becomes the window's background pixmap, so the server repaints exposures
    /// on its own.
    pub fn present(&mut self, bitmap: &BgraBitmap) -> Result<(), String> {
        let (w, h) = (bitmap.width.max(1) as u16, bitmap.height.max(1) as u16);
        let pixmap = match self.pixmap {
            Some((p, pw, ph)) if (pw, ph) == (w, h) => p,
            other => {
                if let Some((old, _, _)) = other {
                    let _ = self.conn.free_pixmap(old);
                }
                let p = self.conn.generate_id().map_err(|e| e.to_string())?;
                self.conn
                    .create_pixmap(self.depth, p, self.window, w, h)
                    .map_err(|e| e.to_string())?;
                self.pixmap = Some((p, w, h));
                p
            }
        };
        if self.size != (w, h) {
            self.size = (w, h);
            let _ = self.conn.configure_window(
                self.window,
                &ConfigureWindowAux::new()
                    .width(w as u32)
                    .height(h as u32)
                    .x(self.pos.0)
                    .y(self.pos.1),
            );
            self.set_size_hints();
        }
        // 0xAARRGGBB words are B, G, R, A bytes on a little-endian host: the ARGB32 visual's order.
        let bytes: Vec<u8> = bitmap
            .premultiplied
            .iter()
            .flat_map(|px| px.to_le_bytes())
            .collect();
        let row = w as usize * 4;
        // Stay under the request size limit: send bands of rows.
        let max = self.conn.maximum_request_bytes().saturating_sub(64).max(row);
        let rows_per = (max / row).clamp(1, h as usize);
        let mut y = 0usize;
        while y < h as usize {
            let n = rows_per.min(h as usize - y);
            self.conn
                .put_image(
                    ImageFormat::Z_PIXMAP,
                    pixmap,
                    self.gc,
                    w,
                    n as u16,
                    0,
                    y as i16,
                    0,
                    self.depth,
                    &bytes[y * row..(y + n) * row],
                )
                .map_err(|e| e.to_string())?;
            y += n;
        }
        let _ = self.conn.change_window_attributes(
            self.window,
            &ChangeWindowAttributesAux::new().background_pixmap(pixmap),
        );
        let _ = self.conn.clear_area(false, self.window, 0, 0, 0, 0);
        self.conn.flush().map_err(|e| e.to_string())
    }

    /// Reserve `thickness` px along `edge` of `monitor_rect` (root coordinates), or release it.
    pub fn reserve(&mut self, want: Option<(Edge, (i32, i32, i32, i32), i32)>) {
        let strut = want.map(|(edge, (l, t, r, b), thickness)| {
            let thick = match edge {
                // Struts count from the root window's edge, not the monitor's.
                Edge::Bottom => (self.root_size.1 as i32 - b) + thickness,
                Edge::Top => t + thickness,
            };
            (edge, l, r - 1, thick.max(0))
        });
        if strut == self.strut {
            return;
        }
        self.strut = strut;
        match strut {
            None => {
                let _ = self.conn.delete_property(self.window, self.atoms._NET_WM_STRUT_PARTIAL);
                let _ = self.conn.delete_property(self.window, self.atoms._NET_WM_STRUT);
            }
            Some((edge, x0, x1, thick)) => {
                let (top, bottom) = match edge {
                    Edge::Top => (thick as u32, 0),
                    Edge::Bottom => (0, thick as u32),
                };
                let (tx0, tx1, bx0, bx1) = match edge {
                    Edge::Top => (x0 as u32, x1 as u32, 0, 0),
                    Edge::Bottom => (0, 0, x0 as u32, x1 as u32),
                };
                let partial = [0, 0, top, bottom, 0, 0, 0, 0, tx0, tx1, bx0, bx1];
                let _ = self.conn.change_property32(
                    PropMode::REPLACE,
                    self.window,
                    self.atoms._NET_WM_STRUT_PARTIAL,
                    AtomEnum::CARDINAL,
                    &partial,
                );
                let _ = self.conn.change_property32(
                    PropMode::REPLACE,
                    self.window,
                    self.atoms._NET_WM_STRUT,
                    AtomEnum::CARDINAL,
                    &[0, 0, top, bottom],
                );
            }
        }
        let _ = self.conn.flush();
    }

    /// Restack above other docks and normal windows (some window managers lower docks on focus
    /// changes).
    pub fn reassert_topmost(&self) {
        if self.shown {
            let _ = self.conn.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
            );
            let _ = self.conn.flush();
        }
    }

    /// Attached monitors with their work areas (monitor ∩ `_NET_WORKAREA`) and scale as a DPI.
    pub fn monitors(&mut self) -> Vec<MonitorInfo> {
        if let Ok(Ok(geo)) = self.conn.get_geometry(self.root).map(|c| c.reply()) {
            self.root_size = (geo.width, geo.height);
        }
        let dpi = self.xft_dpi().unwrap_or(96);
        let workarea = self.workarea();
        let mut out = Vec::new();
        if let Ok(Ok(reply)) = self.conn.randr_get_monitors(self.root, true).map(|c| c.reply()) {
            for m in reply.monitors {
                let name = self
                    .conn
                    .get_atom_name(m.name)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                    .unwrap_or_else(|| format!("monitor-{}", m.name));
                let rect = (
                    m.x as i32,
                    m.y as i32,
                    m.x as i32 + m.width as i32,
                    m.y as i32 + m.height as i32,
                );
                out.push(MonitorInfo {
                    name,
                    work_area: intersect(rect, workarea.unwrap_or(rect)),
                    monitor_rect: rect,
                    is_primary: m.primary,
                    dpi,
                });
            }
        }
        if out.is_empty() {
            let rect = (0, 0, self.root_size.0 as i32, self.root_size.1 as i32);
            out.push(MonitorInfo {
                name: "screen".into(),
                work_area: intersect(rect, workarea.unwrap_or(rect)),
                monitor_rect: rect,
                is_primary: true,
                dpi,
            });
        }
        if !out.iter().any(|m| m.is_primary) {
            out[0].is_primary = true;
        }
        out
    }

    fn workarea(&self) -> Option<(i32, i32, i32, i32)> {
        let reply = self
            .conn
            .get_property(false, self.root, self.atoms._NET_WORKAREA, AtomEnum::CARDINAL, 0, 4)
            .ok()?
            .reply()
            .ok()?;
        let v: Vec<u32> = reply.value32()?.collect();
        (v.len() >= 4).then(|| {
            (
                v[0] as i32,
                v[1] as i32,
                (v[0] + v[2]) as i32,
                (v[1] + v[3]) as i32,
            )
        })
    }

    /// `Xft.dpi` from the root window's resources (GNOME sets it from the display scale).
    fn xft_dpi(&self) -> Option<u32> {
        let reply = self
            .conn
            .get_property(
                false,
                self.root,
                self.atoms.RESOURCE_MANAGER,
                AtomEnum::STRING,
                0,
                1 << 16,
            )
            .ok()?
            .reply()
            .ok()?;
        parse_xft_dpi(&String::from_utf8_lossy(&reply.value))
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}

fn intersect(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let r = (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3));
    if r.0 < r.2 && r.1 < r.3 { r } else { a }
}

pub fn parse_xft_dpi(resources: &str) -> Option<u32> {
    resources.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "Xft.dpi")
            .then(|| v.trim().parse::<f32>().ok())
            .flatten()
            .map(|d| d.round() as u32)
            .filter(|d| (48..=480).contains(d))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xft_dpi_is_read_from_resources() {
        assert_eq!(
            parse_xft_dpi("Xcursor.size:\t24\nXft.dpi:\t192\nXft.hinting:\t1\n"),
            Some(192)
        );
        assert_eq!(parse_xft_dpi("Xcursor.size: 24\n"), None);
    }

    #[test]
    fn workarea_intersection_keeps_monitor_when_disjoint() {
        let mon = (1920, 0, 3840, 1080);
        assert_eq!(intersect(mon, (0, 27, 3840, 1080)), (1920, 27, 3840, 1080));
        assert_eq!(intersect(mon, (0, 0, 100, 100)), mon);
    }
}
