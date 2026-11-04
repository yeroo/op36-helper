//! Tray icon as a StatusNotifierItem (KDE, and GNOME with the AppIndicator extension that Ubuntu
//! enables by default). Left click toggles the overlay, middle click opens Options, the menu has
//! both plus Exit. Notifications go through `notify-send`.

use ksni::blocking::TrayMethods;

use super::AppEvent;
use crate::sink::Sink;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCmd {
    Toggle,
    Options,
    Exit,
}

struct Op36Tray {
    events: Sink<AppEvent>,
    connected: bool,
    visible: bool,
    tooltip: String,
    icon_on: Vec<ksni::Icon>,
    icon_off: Vec<ksni::Icon>,
}

impl ksni::Tray for Op36Tray {
    fn id(&self) -> String {
        "op36-helper".into()
    }

    fn title(&self) -> String {
        "OP36 Helper".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        if self.connected {
            self.icon_on.clone()
        } else {
            self.icon_off.clone()
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "OP36 Helper".into(),
            description: self.tooltip.clone(),
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.events)(AppEvent::Tray(TrayCmd::Toggle));
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        (self.events)(AppEvent::Tray(TrayCmd::Options));
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: if self.visible {
                    "Hide overlay".into()
                } else {
                    "Show overlay".into()
                },
                activate: Box::new(|t: &mut Self| (t.events)(AppEvent::Tray(TrayCmd::Toggle))),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Options…".into(),
                activate: Box::new(|t: &mut Self| (t.events)(AppEvent::Tray(TrayCmd::Options))),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Exit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| (t.events)(AppEvent::Tray(TrayCmd::Exit))),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Decode a bundled PNG into SNI pixmaps (the full size plus a box-filtered 32 px copy).
fn icons(png_bytes: &[u8]) -> Vec<ksni::Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let Ok(mut reader) = decoder.read_info() else {
        return Vec::new();
    };
    let Some(size) = reader.output_buffer_size() else {
        return Vec::new();
    };
    let mut buf = vec![0; size];
    let Ok(info) = reader.next_frame(&mut buf) else {
        return Vec::new();
    };
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Vec::new();
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let argb = |rgba: &[u8]| -> Vec<u8> {
        rgba.chunks_exact(4)
            .flat_map(|p| [p[3], p[0], p[1], p[2]])
            .collect()
    };
    let mut out = vec![ksni::Icon {
        width: w as i32,
        height: h as i32,
        data: argb(&buf[..w * h * 4]),
    }];
    let small = 32usize;
    if w >= small && w % small == 0 && h == w {
        let f = w / small;
        let mut rgba = Vec::with_capacity(small * small * 4);
        for y in 0..small {
            for x in 0..small {
                let mut acc = [0u32; 4];
                for dy in 0..f {
                    for dx in 0..f {
                        let i = ((y * f + dy) * w + x * f + dx) * 4;
                        for c in 0..4 {
                            acc[c] += buf[i + c] as u32;
                        }
                    }
                }
                rgba.extend(acc.iter().map(|v| (v / (f * f) as u32) as u8));
            }
        }
        out.push(ksni::Icon {
            width: small as i32,
            height: small as i32,
            data: argb(&rgba),
        });
    }
    out
}

pub struct Tray {
    handle: Option<ksni::blocking::Handle<Op36Tray>>,
}

impl Tray {
    pub fn new(events: Sink<AppEvent>, visible: bool) -> Tray {
        let tray = Op36Tray {
            events,
            connected: false,
            visible,
            tooltip: String::new(),
            icon_on: icons(include_bytes!("../../assets/icon_on_preview.png")),
            icon_off: icons(include_bytes!("../../assets/icon_off_preview.png")),
        };
        let handle = match tray.spawn() {
            Ok(h) => Some(h),
            Err(e) => {
                log::warn!(
                    "tray icon unavailable ({e}); use `op36-helper --toggle` / `--options` instead"
                );
                None
            }
        };
        Tray { handle }
    }

    pub fn set_connected(&mut self, connected: bool) {
        if let Some(h) = &self.handle {
            h.update(|t| t.connected = connected);
        }
    }

    pub fn set_visible(&mut self, visible: bool) {
        if let Some(h) = &self.handle {
            h.update(|t| t.visible = visible);
        }
    }

    pub fn set_tooltip(&mut self, tip: &str) {
        if let Some(h) = &self.handle {
            let tip = tip.to_string();
            h.update(move |t| t.tooltip = tip);
        }
    }

    /// A desktop notification (the Windows build's tray balloon).
    pub fn balloon(&mut self, title: &str, text: &str) {
        let spawned = std::process::Command::new("notify-send")
            .args(["--app-name=OP36 Helper", "--icon=input-keyboard", title, text])
            .spawn();
        if let Err(e) = spawned {
            log::info!("notification not shown ({e}): {title}: {text}");
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.shutdown();
        }
    }
}
