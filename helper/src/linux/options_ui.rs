//! Options on Linux: an egui window in a child process (`op36-helper --options-ui`), because a
//! winit event loop can be created only once per process. The tray process sends the state as
//! JSON lines on the child's stdin; the child answers with actions on stdout, and exits when the
//! window closes (EOF tells the parent).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::AppEvent;
use crate::options::{SCALE_MAX_PCT, SCALE_MIN_PCT, hotkey_label};
use crate::overlay::placement::MonitorInfo;
use crate::settings::modbits::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
use crate::settings::{Anchor, DisplayMode, Hotkey, Settings, Theme};
use crate::sink::Sink;

/// What the Firmware group shows (the Windows dialog's `FirmwareView`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FirmwareView {
    pub left: String,
    pub right: String,
    pub bundled: String,
    pub note: String,
    pub can_flash: bool,
    pub flashing: bool,
    pub status: String,
    pub update_line: String,
    pub can_update: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToChild {
    /// Everything the window shows; sent on open.
    Init {
        settings: Settings,
        monitors: Vec<MonitorInfo>,
        status: String,
        firmware: FirmwareView,
    },
    Firmware(FirmwareView),
    Status(String),
    /// The arrow hotkeys moved the overlay.
    Offset((i32, i32)),
    /// An apply failed (hotkey, autostart, save).
    Error(String),
    /// Bring the window to the front (Options asked for again).
    Raise,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Half {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FromChild {
    Settings { settings: Settings, save: bool },
    ReloadKeymap,
    FlashHalf(Half),
    CancelFlash,
    UpdateNow,
}

// ---------------------------------------------------------------------------------------------
// Parent side

pub struct OptionsProcess {
    child: Child,
    stdin: ChildStdin,
}

impl OptionsProcess {
    /// Start the child and its reader thread; replies arrive as `AppEvent::Options`, the close as
    /// `AppEvent::OptionsClosed`.
    pub fn spawn(events: Sink<AppEvent>, init: ToChild) -> Result<OptionsProcess, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut child = Command::new(exe)
            .arg("--options-ui")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start the Options window: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        std::thread::Builder::new()
            .name("options-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    match serde_json::from_str::<FromChild>(&line) {
                        Ok(msg) => events(AppEvent::Options(msg)),
                        Err(e) => log::warn!("options: bad message ({e}): {line}"),
                    }
                }
                events(AppEvent::OptionsClosed);
            })
            .map_err(|e| e.to_string())?;
        let mut p = OptionsProcess { child, stdin };
        p.send(&init);
        Ok(p)
    }

    pub fn send(&mut self, msg: &ToChild) {
        if let Ok(line) = serde_json::to_string(msg) {
            let _ = writeln!(self.stdin, "{line}");
            let _ = self.stdin.flush();
        }
    }
}

impl Drop for OptionsProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------------------------
// Child side

const ANCHORS: [(Anchor, &str); 9] = [
    (Anchor::TopLeft, "Top left"),
    (Anchor::Top, "Top"),
    (Anchor::TopRight, "Top right"),
    (Anchor::Left, "Left"),
    (Anchor::Center, "Center"),
    (Anchor::Right, "Right"),
    (Anchor::BottomLeft, "Bottom left"),
    (Anchor::Bottom, "Bottom"),
    (Anchor::BottomRight, "Bottom right"),
];

/// Keys offered for the toggle hotkey, as Windows virtual keys (what settings store).
fn hotkey_keys() -> Vec<u32> {
    let mut v: Vec<u32> = (0x41..=0x5A).collect(); // A–Z
    v.extend(0x30..=0x39); // 0–9
    v.extend(0x70..=0x7B); // F1–F12
    v
}

fn key_name(vk: u32) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        other => format!("VK{other:#04x}"),
    }
}

struct OptionsApp {
    rx: Receiver<ToChild>,
    settings: Option<Settings>,
    original: Option<Settings>,
    monitors: Vec<MonitorInfo>,
    status: String,
    firmware: FirmwareView,
    error: String,
}

fn send(msg: &FromChild) {
    if let Ok(line) = serde_json::to_string(msg) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

impl OptionsApp {
    fn poll(&mut self, ctx: &egui::Context) {
        loop {
            match self.rx.try_recv() {
                Ok(ToChild::Init {
                    settings,
                    monitors,
                    status,
                    firmware,
                }) => {
                    self.original = Some(settings.clone());
                    self.settings = Some(settings);
                    self.monitors = monitors;
                    self.status = status;
                    self.firmware = firmware;
                }
                Ok(ToChild::Firmware(f)) => self.firmware = f,
                Ok(ToChild::Status(s)) => self.status = s,
                Ok(ToChild::Offset(o)) => {
                    if let Some(s) = self.settings.as_mut() {
                        s.offset = o;
                    }
                }
                Ok(ToChild::Error(e)) => self.error = e,
                Ok(ToChild::Raise) => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    break;
                }
            }
        }
    }

    fn settings_ui(ui: &mut egui::Ui, s: &mut Settings, monitors: &[MonitorInfo]) -> bool {
        let mut changed = false;
        ui.heading("Display");
        egui::Grid::new("display").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Monitor");
            let current = match &s.monitor {
                None => "Primary monitor".to_string(),
                Some(n) if monitors.iter().any(|m| &m.name == n) => n.clone(),
                Some(n) => format!("{n} (not connected)"),
            };
            egui::ComboBox::from_id_salt("monitor")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    changed |= ui.selectable_value(&mut s.monitor, None, "Primary monitor").changed();
                    for m in monitors {
                        let label = format!(
                            "{} ({}×{})",
                            m.name,
                            m.monitor_rect.2 - m.monitor_rect.0,
                            m.monitor_rect.3 - m.monitor_rect.1
                        );
                        changed |= ui
                            .selectable_value(&mut s.monitor, Some(m.name.clone()), label)
                            .changed();
                    }
                });
            ui.end_row();

            ui.label("Position");
            egui::ComboBox::from_id_salt("anchor")
                .selected_text(
                    ANCHORS
                        .iter()
                        .find(|(a, _)| *a == s.anchor)
                        .map(|(_, l)| *l)
                        .unwrap_or("Bottom"),
                )
                .show_ui(ui, |ui| {
                    for (a, label) in ANCHORS {
                        changed |= ui.selectable_value(&mut s.anchor, a, label).changed();
                    }
                });
            ui.end_row();

            ui.label("Offset X / Y");
            ui.horizontal(|ui| {
                changed |= ui.add(egui::DragValue::new(&mut s.offset.0).range(-4000..=4000).suffix(" px")).changed();
                changed |= ui.add(egui::DragValue::new(&mut s.offset.1).range(-4000..=4000).suffix(" px")).changed();
            });
            ui.end_row();

            ui.label("Size");
            let mut pct = crate::options::scale_to_slider(s.scale);
            if ui
                .add(egui::Slider::new(&mut pct, SCALE_MIN_PCT..=SCALE_MAX_PCT).suffix(" %"))
                .changed()
            {
                s.scale = crate::options::slider_to_scale(pct);
                changed = true;
            }
            ui.end_row();

            ui.label("Opacity");
            let mut op = (s.opacity * 100.0).round() as i32;
            if ui.add(egui::Slider::new(&mut op, 10..=100).suffix(" %")).changed() {
                s.opacity = op as f32 / 100.0;
                changed = true;
            }
            ui.end_row();

            ui.label("Theme");
            ui.horizontal(|ui| {
                changed |= ui.radio_value(&mut s.theme, Theme::Auto, "Follow system").changed();
                changed |= ui.radio_value(&mut s.theme, Theme::Light, "Light").changed();
                changed |= ui.radio_value(&mut s.theme, Theme::Dark, "Dark").changed();
            });
            ui.end_row();

            ui.label("Show");
            ui.vertical(|ui| {
                changed |= ui
                    .radio_value(&mut s.display_mode, DisplayMode::Always, "Always (while enabled)")
                    .changed();
                changed |= ui
                    .radio_value(
                        &mut s.display_mode,
                        DisplayMode::WhileHeld,
                        "Only while a layer or modifier is held",
                    )
                    .changed();
            });
            ui.end_row();
        });
        changed |= ui
            .checkbox(&mut s.reserve_space, "Reserve screen space for maximized windows")
            .changed();
        changed |= ui
            .checkbox(&mut s.hide_when_offline, "Hide the overlay while the keyboard is off")
            .changed();

        ui.separator();
        ui.heading("Keyboard");
        egui::Grid::new("keyboard").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Toggle hotkey");
            ui.horizontal(|ui| {
                changed |= mod_boxes(ui, &mut s.hotkey.modifiers);
                egui::ComboBox::from_id_salt("hotkey-key")
                    .selected_text(key_name(s.hotkey.vk))
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for vk in hotkey_keys() {
                            changed |= ui.selectable_value(&mut s.hotkey.vk, vk, key_name(vk)).changed();
                        }
                    });
            });
            ui.end_row();
            ui.label("Move with arrows");
            ui.horizontal(|ui| {
                changed |= mod_boxes(ui, &mut s.move_hotkey_mods);
                ui.label("+ arrows nudge 10 px");
            });
            ui.end_row();
        });
        ui.label(
            egui::RichText::new(format!(
                "Hotkeys need key access (input group). From a desktop shortcut: `op36-helper --toggle` ({}).",
                hotkey_label(&Hotkey {
                    modifiers: s.hotkey.modifiers,
                    vk: s.hotkey.vk
                })
            ))
            .small()
            .weak(),
        );
        changed |= ui.checkbox(&mut s.find_key, "Find a key when I press one").changed();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(s.find_key, |ui| {
                ui.label("Show for");
                changed |= ui
                    .add(egui::Slider::new(&mut s.find_key_secs, 0.5..=5.0).step_by(0.1).suffix(" s"))
                    .changed();
            });
        });
        changed |= ui
            .checkbox(&mut s.show_en_hint, "Show Latin hint on non-Latin layouts")
            .changed();

        ui.separator();
        ui.heading("Behaviour");
        changed |= ui.checkbox(&mut s.autostart, "Start with the session").changed();
        changed |= ui
            .checkbox(&mut s.update_check, "Check for updates automatically")
            .changed();
        changed
    }
}

fn mod_boxes(ui: &mut egui::Ui, bits: &mut u32) -> bool {
    let mut changed = false;
    for (bit, name) in [
        (MOD_CONTROL, "Ctrl"),
        (MOD_ALT, "Alt"),
        (MOD_SHIFT, "Shift"),
        (MOD_WIN, "Super"),
    ] {
        let mut on = *bits & bit != 0;
        if ui.checkbox(&mut on, name).changed() {
            if on {
                *bits |= bit;
            } else {
                *bits &= !bit;
            }
            changed = true;
        }
    }
    changed
}

impl eframe::App for OptionsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(mut s) = self.settings.clone() else {
                ui.label("Loading…");
                return;
            };
            egui::ScrollArea::vertical().show(ui, |ui| {
                if Self::settings_ui(ui, &mut s, &self.monitors) {
                    // Live preview; Startup entry and the file only change on Apply / OK.
                    send(&FromChild::Settings {
                        settings: s.clone(),
                        save: false,
                    });
                    self.settings = Some(s.clone());
                }

                ui.separator();
                ui.heading("Firmware and updates");
                let f = &self.firmware;
                ui.label(&f.left);
                ui.label(&f.right);
                ui.label(&f.bundled);
                if !f.note.is_empty() {
                    ui.colored_label(egui::Color32::from_rgb(0xE0, 0x9A, 0x30), &f.note);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(f.can_flash, egui::Button::new("Flash left half…"))
                        .clicked()
                    {
                        send(&FromChild::FlashHalf(Half::Left));
                    }
                    if ui
                        .add_enabled(f.can_flash, egui::Button::new("Flash right half…"))
                        .clicked()
                    {
                        send(&FromChild::FlashHalf(Half::Right));
                    }
                    if ui
                        .add_enabled(f.flashing, egui::Button::new("Cancel flash"))
                        .clicked()
                    {
                        send(&FromChild::CancelFlash);
                    }
                });
                if !f.status.is_empty() {
                    ui.label(&f.status);
                }
                ui.horizontal(|ui| {
                    ui.label(&f.update_line);
                    if ui
                        .add_enabled(f.can_update, egui::Button::new("Update now"))
                        .clicked()
                    {
                        send(&FromChild::UpdateNow);
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("Status");
                    if ui.button("Reload keymap").clicked() {
                        send(&FromChild::ReloadKeymap);
                    }
                });
                ui.label(egui::RichText::new(&self.status).monospace().small());
                if !self.error.is_empty() {
                    ui.colored_label(egui::Color32::from_rgb(0xD6, 0x3A, 0x3A), &self.error);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        send(&FromChild::Settings {
                            settings: s.clone(),
                            save: true,
                        });
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        if let Some(orig) = &self.original {
                            send(&FromChild::Settings {
                                settings: orig.clone(),
                                save: false,
                            });
                        }
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Apply").clicked() {
                        send(&FromChild::Settings {
                            settings: s.clone(),
                            save: true,
                        });
                        self.original = Some(s.clone());
                        self.error.clear();
                    }
                });
            });
        });
    }
}

/// `--options-ui`: run the window until it closes.
pub fn run_child() -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>> = Default::default();
    let slot = ctx_slot.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            if let Ok(msg) = serde_json::from_str::<ToChild>(&line) {
                if tx.send(msg).is_err() {
                    return;
                }
                if let Some(ctx) = slot.lock().ok().and_then(|c| c.clone()) {
                    ctx.request_repaint();
                }
            }
        }
        // Parent gone: dropping `tx` makes the window close itself.
        drop(tx);
        if let Some(ctx) = slot.lock().ok().and_then(|c| c.clone()) {
            ctx.request_repaint();
        }
    });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OP36 Helper — Options")
            .with_app_id("op36-helper")
            .with_inner_size([560.0, 860.0])
            .with_min_inner_size([420.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "OP36 Helper — Options",
        options,
        Box::new(move |cc| {
            if let Ok(mut c) = ctx_slot.lock() {
                *c = Some(cc.egui_ctx.clone());
            }
            Ok(Box::new(OptionsApp {
                rx,
                settings: None,
                original: None,
                monitors: Vec::new(),
                status: String::new(),
                firmware: FirmwareView::default(),
                error: String::new(),
            }))
        }),
    )
    .map_err(|e| e.to_string())
}
