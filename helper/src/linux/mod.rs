//! The Linux tray application: an X11 overlay window (XWayland on Wayland desktops), a
//! StatusNotifierItem tray icon, evdev keyboard input, and the Options window in a child process.
//!
//! Everything that happens arrives on one channel as an [`AppEvent`]; timers are deadlines the
//! loop waits for. The behaviour (offline grace, auto-hide, "only while held", find-a-key,
//! flashing, updates) follows the Windows app in `win_app.rs` step for step.

pub mod desktop;
pub mod instance;
pub mod keys;
pub mod options_ui;
pub mod tray;
pub mod x11;
pub mod xkb;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use crate::app_common::{init_logging, uptime_ms};
use crate::input::modifiers::{ModTracker, is_modifier_vk};
use crate::input::rawkey::RawKey;
use crate::legend::hid_table::scancode_to_hid;
use crate::model::{self, KeyPress, Model, ModelEvent};
use crate::overlay::frame::Frame;
use crate::overlay::placement::{MonitorInfo, chosen_monitor, place_with_reserved};
use crate::overlay::render_skia::Renderer;
use crate::settings::{Anchor, NUDGE_PX, Settings};
use crate::sink::Sink;
use crate::update_thread::{UpdateCmd, UpdateEvent};
use crate::{
    auto_hide, firmware, firmware_thread, hid_thread, offline_grace, paths, theme, update,
    update_thread, version, vial,
};
use options_ui::{FirmwareView, FromChild, OptionsProcess, ToChild};
use tray::{Tray, TrayCmd};

const OFFLINE_GRACE: Duration = Duration::from_millis(15_000);
const SWITCH_BLINK: Duration = Duration::from_millis(500);
const HIDE_DELAY: Duration = Duration::from_millis(1000);
const TOPMOST_EVERY: Duration = Duration::from_secs(2);
/// A modifier seen held this long without any key event is re-read from the kernel (a release can
/// be missed across a VT switch or a screen lock).
const MODS_RESYNC_AFTER: Duration = Duration::from_secs(3);
/// Env var a freshly updated binary is started with: wait for the old process to let go.
const AFTER_UPDATE_ENV: &str = "OP36_AFTER_UPDATE";

/// Everything the main loop reacts to.
pub enum AppEvent {
    Hid(model::HidEvent),
    Flash(firmware::FlashStep),
    Update(UpdateEvent),
    Key(RawKey),
    KeyAccess(keys::Access),
    Language(model::Hkl),
    ThemeChanged,
    DisplayChanged,
    Tray(TrayCmd),
    /// A line on the control socket (`toggle`, `options`, `quit`).
    Control(String),
    Options(FromChild),
    OptionsClosed,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Timer {
    Find,
    Hide,
    Offline,
    Switch,
    OffHide,
    Topmost,
    ModsResync,
}

#[derive(Default)]
struct Timers {
    due: HashMap<Timer, Instant>,
}

impl Timers {
    fn set(&mut self, t: Timer, after: Duration) {
        self.due.insert(t, Instant::now() + after);
    }
    fn kill(&mut self, t: Timer) {
        self.due.remove(&t);
    }
    fn wait(&self) -> Duration {
        self.due
            .values()
            .min()
            .map(|d| d.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(3600))
    }
    fn take_due(&mut self) -> Vec<Timer> {
        let now = Instant::now();
        let due: Vec<Timer> = self
            .due
            .iter()
            .filter(|(_, d)| **d <= now)
            .map(|(t, _)| *t)
            .collect();
        for t in &due {
            self.due.remove(t);
        }
        due
    }
}

/// The edge a reservation goes on for an anchor (top or bottom rows only, as on Windows).
fn edge_for(anchor: Anchor) -> Option<x11::Edge> {
    match anchor {
        Anchor::TopLeft | Anchor::Top | Anchor::TopRight => Some(x11::Edge::Top),
        Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => Some(x11::Edge::Bottom),
        _ => None,
    }
}

struct App {
    events: Sink<AppEvent>,
    timers: Timers,
    settings: Settings,
    model: Model,
    overlay: x11::Overlay,
    tray: Option<Tray>,
    hid: Option<std::sync::mpsc::Sender<hid_thread::HidCmd>>,
    mods: ModTracker,
    renderer: Renderer,
    chars: xkb::XkbCharSource,
    last_frame: Option<Frame>,
    last_scale: f32,
    options: Option<OptionsProcess>,
    /// The strip reserved now: `(monitor name, thickness)`.
    reserved: Option<(String, i32)>,
    last_tooltip: String,
    overlay_shown: bool,
    hide_pending: bool,
    offline_grace: offline_grace::OfflineGrace,
    switch_timer_on: bool,
    auto_hide: auto_hide::AutoHide,
    flash: Option<(
        vial::protocol::Side,
        std::sync::mpsc::Sender<()>,
        firmware::FlashStep,
    )>,
    update_busy: bool,
    update: Option<std::sync::mpsc::Sender<UpdateCmd>>,
    update_notified: Option<String>,
    update_line: String,
    key_access: keys::Access,
    monitors: Vec<MonitorInfo>,
    /// Notifications already shown this run.
    notified: std::collections::HashSet<&'static str>,
    quit: bool,
}

impl App {
    fn set_visible(&mut self, visible: bool) {
        self.settings.visible = visible;
        self.auto_hide.user_toggled();
        self.timers.kill(Timer::OffHide);
        if visible && !self.model.connected {
            // Shown by hand with the keyboard off: it goes away again after the minute.
            let action = self.keyboard_off_action();
            self.apply_auto_hide(action);
        }
        let show = visible && self.model.wants_visible();
        self.overlay.show(show);
        self.overlay_shown = show;
        self.hide_pending = false;
        if let Some(t) = self.tray.as_mut() {
            t.set_visible(visible);
        }
        self.redraw(true);
        if let Err(e) = self.settings.save(&paths::settings_file()) {
            log::warn!("could not save settings: {e}");
        }
    }

    /// Keep the strut reservation in line with settings, visibility and overlay size.
    fn update_reservation(&mut self, size: (i32, i32)) {
        let edge = edge_for(self.settings.anchor);
        // Like the Windows app bar: held while the overlay is enabled, also through the "only while
        // held" hide (maximized windows must not resize on every modifier press).
        let wanted =
            self.settings.reserve_space && self.settings.visible && !self.auto_hide.is_hidden();
        match edge.filter(|_| wanted) {
            None => {
                self.overlay.reserve(None);
                self.reserved = None;
            }
            Some(edge) => {
                let mon = chosen_monitor(&self.settings, &self.monitors).clone();
                let thickness = size.1 + self.settings.offset.1.max(0);
                self.overlay
                    .reserve(Some((edge, mon.monitor_rect, thickness)));
                self.reserved = Some((mon.name, thickness));
            }
        }
    }

    /// Recompute the frame; render, place and present only when something changed.
    fn redraw(&mut self, force: bool) {
        let dark = theme::dark_mode(self.settings.theme);
        let force = force || dark != self.renderer.dark;
        self.renderer.dark = dark;
        let frame = self.model.frame_at(&self.chars, uptime_ms());
        if frame.switching != self.switch_timer_on {
            self.switch_timer_on = frame.switching;
            if frame.switching {
                self.timers.set(Timer::Switch, SWITCH_BLINK);
            } else {
                self.timers.kill(Timer::Switch);
            }
        }
        if force || self.monitors.is_empty() {
            self.monitors = self.overlay.monitors();
        }
        if self.monitors.is_empty() {
            return;
        }
        let mon = chosen_monitor(&self.settings, &self.monitors);
        let scale = self.settings.scale * mon.dpi as f32 / 96.0;
        if !force && self.last_frame.as_ref() == Some(&frame) && self.last_scale == scale {
            return;
        }
        let t0 = Instant::now();
        let bitmap = self.renderer.render(&frame, scale, self.settings.opacity);
        let t_render = t0.elapsed();
        // Work-area changes (our own strip included) arrive as `DisplayChanged`, which re-reads the
        // monitors; a layer flip must not pay for X round trips.
        self.update_reservation((bitmap.width, bitmap.height));
        let reserved = self.reserved.clone();
        let (x, y, _) = place_with_reserved(
            &self.settings,
            (bitmap.width, bitmap.height),
            &self.monitors,
            reserved.as_ref().map(|(n, t)| (n.as_str(), *t)),
        );
        self.overlay.set_position(x, y);
        if let Err(e) = self.overlay.present(&bitmap) {
            log::warn!("present failed: {e}");
        }
        log::debug!(
            "redraw: render {:.1} ms, total {:.1} ms",
            t_render.as_secs_f64() * 1000.0,
            t0.elapsed().as_secs_f64() * 1000.0
        );
        self.last_frame = Some(frame);
        self.last_scale = scale;
    }

    fn on_key(&mut self, key: RawKey) {
        let before = self.mods.current();
        let after = self.mods.apply(&key);
        if after != model::Modifiers::default() {
            self.timers.set(Timer::ModsResync, MODS_RESYNC_AFTER);
        } else {
            self.timers.kill(Timer::ModsResync);
        }
        if after != before {
            self.on_model_event(ModelEvent::Mods(after));
        }
        if !key.down || is_modifier_vk(key.vk) {
            return;
        }
        // Global hotkeys (Wayland has no RegisterHotKey for us; the key still reaches the app).
        let bits = keys::mod_bits(after);
        let hk = self.settings.hotkey;
        if bits == hk.modifiers && keys::vk_scancode(hk.vk) == Some(key.scancode) {
            let v = !self.settings.visible;
            self.set_visible(v);
            return;
        }
        if bits == self.settings.move_hotkey_mods && bits != 0 {
            let nudge = [
                (0x25u32, (-NUDGE_PX, 0)),
                (0x26, (0, -NUDGE_PX)),
                (0x27, (NUDGE_PX, 0)),
                (0x28, (0, NUDGE_PX)),
            ]
            .into_iter()
            .find(|(vk, _)| keys::vk_scancode(*vk) == Some(key.scancode));
            if let Some((_, (dx, dy))) = nudge {
                self.nudge(dx, dy);
                return;
            }
        }
        self.on_model_event(ModelEvent::Key(KeyPress {
            hid: scancode_to_hid(key.scancode),
            scancode: key.scancode.code,
            extended: key.scancode.extended,
            mods_at_press: after,
            device: key.device,
        }));
    }

    fn on_mods_resync(&mut self) {
        if let Some((l, r)) = keys::held_modifiers() {
            let m = self.mods.set(l, r);
            if m != self.model.mods {
                self.on_model_event(ModelEvent::Mods(m));
            }
        }
    }

    fn on_model_event(&mut self, event: ModelEvent) {
        if let ModelEvent::Hid(model::HidEvent::AccessDenied(true)) = &event {
            self.model.access_denied = true;
            let action = self.auto_hide.on_connected();
            self.apply_auto_hide(action);
            self.notify_once(
                "hid",
                "OP36 Helper can't open the keyboard",
                "No permission for its HID device, so no layer, battery or link state. Fix: \
                 sudo cp linux/70-op36.rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger \
                 (or ./install.sh --udev from the release tarball).",
            );
        }
        // Offline grace: a Bluetooth link that re-negotiates comes back within seconds; keep the
        // last state and only flag offline when the link stays down.
        match &event {
            ModelEvent::Hid(model::HidEvent::Disconnected) => {
                match self.offline_grace.on_disconnected(self.model.connected) {
                    offline_grace::Gate::Hold => {
                        self.timers.set(Timer::Offline, OFFLINE_GRACE);
                        return;
                    }
                    offline_grace::Gate::Ignore => return,
                    offline_grace::Gate::Apply => {}
                }
            }
            ModelEvent::Hid(model::HidEvent::Connected(_)) => {
                if self.offline_grace.on_connected() {
                    self.timers.kill(Timer::Offline);
                }
            }
            _ => {}
        }
        self.apply_model_event(event);
    }

    /// Past the offline gate: update the model and everything that follows from it.
    fn apply_model_event(&mut self, event: ModelEvent) {
        if let ModelEvent::Key(k) = &event {
            if self.settings.find_key && k.hid.is_some() {
                let ms = (self.settings.find_key_secs.clamp(0.5, 5.0) * 1000.0) as u64;
                self.timers.set(Timer::Find, Duration::from_millis(ms));
            }
            log::debug!(
                "key: hid={:?} sc={:#x}{} mods={:?} from {:?}",
                k.hid,
                k.scancode,
                if k.extended { "e" } else { "" },
                k.mods_at_press,
                k.device
            );
        }
        let was_connected = self.model.connected;
        self.model.apply_at(event, uptime_ms());
        let action = match (was_connected, self.model.connected) {
            (true, false) => self.keyboard_off_action(),
            (false, true) => self.auto_hide.on_connected(),
            _ => None,
        };
        self.apply_auto_hide(action);
        if let Some(bytes) = self.model.take_pending_cache_write() {
            if let Err(e) = std::fs::write(paths::keymap_cache_file(), bytes) {
                log::warn!("could not cache keymap: {e}");
            } else {
                log::info!("keymap: live ({} layers), cached", self.model.keymap.layers);
            }
        }
        if let Some(tray) = self.tray.as_mut() {
            tray.set_connected(self.model.connected);
        }
        self.refresh_tooltip();
        self.apply_display_mode();
        self.redraw(false);
        self.push_options_state();
        log::debug!(
            "model: connected={} layer={} notify={:?} battery={:?} transport={:?} source={:?}",
            self.model.connected,
            self.model.layer.active,
            self.model.layer_notify,
            self.model.battery,
            self.model.transport,
            self.model.keymap_source
        );
    }

    /// One desktop notification per `key` per run.
    fn notify_once(&mut self, key: &'static str, title: &str, body: &str) {
        if !self.notified.insert(key) {
            return;
        }
        log::warn!("{title}: {body}");
        if let Some(tray) = self.tray.as_mut() {
            tray.balloon(title, body);
        }
    }

    fn notify_key_access(&mut self) {
        if self.key_access == keys::Access::Denied && self.settings.find_key {
            self.notify_once(
                "keys",
                "OP36 Helper can't read key presses",
                "Find-a-key, Shift/Ctrl/Alt variants and the hotkeys need /dev/input access. Fix: \
                 sudo cp linux/71-op36-keyboards.rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger \
                 (any program you run can then read keystrokes).",
            );
        }
    }

    fn refresh_tooltip(&mut self) {
        let tip = self.model.status_line_at(uptime_ms());
        if self.last_tooltip != tip {
            if let Some(tray) = self.tray.as_mut() {
                tray.set_tooltip(&tip);
            }
            self.last_tooltip = tip;
        }
    }

    /// Apply new settings everywhere they matter; `save` persists them.
    fn apply_settings(&mut self, new: Settings, save: bool) -> Result<(), String> {
        let old = std::mem::replace(&mut self.settings, new.clone());
        let mut errors = Vec::new();
        if old.hide_when_offline != new.hide_when_offline {
            if new.hide_when_offline {
                if !self.model.connected {
                    let action = self.keyboard_off_action();
                    self.apply_auto_hide(action);
                }
            } else {
                let action = self.auto_hide.on_connected();
                self.apply_auto_hide(action);
                self.auto_hide.user_toggled();
            }
        }
        if old.update_check != new.update_check
            && let Some(tx) = &self.update
        {
            let _ = tx.send(UpdateCmd::AutoCheck(new.update_check));
        }
        if old.autostart != new.autostart {
            match std::env::current_exe() {
                Ok(exe) => {
                    if let Err(e) = desktop::set_autostart(new.autostart, &exe) {
                        errors.push(format!("autostart: {e}"));
                    }
                }
                Err(e) => errors.push(format!("autostart: {e}")),
            }
        }
        self.model.apply(ModelEvent::Settings(new.clone()));
        let show = new.visible && self.model.wants_visible() && !self.auto_hide.is_hidden();
        self.overlay.show(show);
        self.overlay_shown = show;
        self.hide_pending = false;
        self.redraw(true);
        if save && let Err(e) = self.settings.save(&paths::settings_file()) {
            errors.push(format!("save: {e}"));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }

    /// "Only while held": show at once when wanted, hide one second after nothing is held.
    fn apply_display_mode(&mut self) {
        if !self.settings.visible || self.auto_hide.is_hidden() {
            return;
        }
        if self.model.wants_visible() {
            self.timers.kill(Timer::Hide);
            if !self.overlay_shown {
                self.overlay.show(true);
                self.overlay_shown = true;
            }
        } else if self.overlay_shown && !self.hide_pending {
            self.timers.set(Timer::Hide, HIDE_DELAY);
            self.hide_pending = true;
        }
    }

    fn on_switch_tick(&mut self) {
        self.renderer.switch_phase = !self.renderer.switch_phase;
        self.redraw(true);
        if self.switch_timer_on {
            self.timers.set(Timer::Switch, SWITCH_BLINK);
        }
        self.refresh_tooltip();
    }

    fn on_offline_timer(&mut self) {
        if self.offline_grace.on_timer() == offline_grace::Gate::Apply {
            // Straight to the model: going back through the gate would hold it again.
            self.apply_model_event(ModelEvent::Hid(model::HidEvent::Disconnected));
        }
    }

    fn keyboard_off_action(&mut self) -> Option<auto_hide::Action> {
        // Present but refused: not "off", and hiding would only hide the explanation.
        if self.settings.hide_when_offline && !self.model.access_denied {
            self.auto_hide.on_disconnected()
        } else {
            None
        }
    }

    fn apply_auto_hide(&mut self, action: Option<auto_hide::Action>) {
        use auto_hide::Action;
        let Some(action) = action else { return };
        match action {
            Action::ArmTimer => {
                self.timers.set(
                    Timer::OffHide,
                    Duration::from_millis(auto_hide::OFF_HIDE_DELAY_MS as u64),
                );
                log::info!(
                    "keyboard off: overlay hides in {} s",
                    auto_hide::OFF_HIDE_DELAY_MS / 1000
                );
            }
            Action::CancelTimer => self.timers.kill(Timer::OffHide),
            Action::Hide => {
                self.timers.kill(Timer::OffHide);
                log::info!("keyboard still off: overlay hidden until it is back");
                self.overlay.show(false);
                self.overlay_shown = false;
                self.hide_pending = false;
                self.redraw(true); // releases the reserved screen space
            }
            Action::Restore => {
                log::info!("keyboard back: overlay restored");
                let show = self.settings.visible && self.model.wants_visible();
                self.overlay.show(show);
                self.overlay_shown = show;
                self.redraw(true);
            }
        }
    }

    fn on_hide_timer(&mut self) {
        self.hide_pending = false;
        if self.settings.visible && !self.model.wants_visible() {
            self.overlay.show(false);
            self.overlay_shown = false;
        }
    }

    fn nudge(&mut self, dx: i32, dy: i32) {
        let mut s = self.settings.clone();
        s.nudge(dx, dy);
        if let Err(e) = self.apply_settings(s, true) {
            log::warn!("{e}");
        }
        let offset = self.settings.offset;
        if let Some(o) = self.options.as_mut() {
            o.send(&ToChild::Offset(offset));
        }
    }

    /// The firmware bundle next to the binary, or in `~/.local/share/op36-helper`.
    fn bundle(&self) -> Option<firmware::Bundle> {
        let exe = std::env::current_exe().ok()?;
        firmware::bundle_at(exe.parent()?).or_else(|| firmware::bundle_at(&update::linux_firmware_home()))
    }

    fn firmware_view(&self) -> FirmwareView {
        let bundle = self.bundle();
        let [left, right] = &self.model.firmware;
        let words = firmware::describe(
            self.model.connected,
            left.as_deref(),
            right.as_deref(),
            bundle.as_ref().map(|b| b.build.as_str()),
        );
        let (flashing, status) = match &self.flash {
            Some((side, _, step)) => (true, step.text(*side)),
            None => (false, String::new()),
        };
        FirmwareView {
            left: words.left,
            right: words.right,
            bundled: match &bundle {
                Some(b) => format!("Bundled: {}", b.build),
                None => "Bundled: no firmware files (firmware/ next to the binary or in ~/.local/share/op36-helper)".to_string(),
            },
            note: words.note,
            can_flash: bundle.is_some() && self.flash.is_none(),
            flashing,
            status,
            update_line: self.update_line.clone(),
            can_update: !self.update_busy,
        }
    }

    fn status_text(&self) -> String {
        format!(
            "Helper: {}\nKeyboard: {}\nFirmware: {}\nLayer data: {}\nKeymap: {:?}\nKey input: {}\n{}",
            version::VERSION,
            match (self.model.connected, self.model.transport) {
                (true, Some(t)) => format!("connected ({})", t.label()),
                (true, None) => "connected".to_string(),
                _ if self.model.access_denied => {
                    "found, but access denied: install linux/70-op36.rules (./install.sh --udev)"
                        .to_string()
                }
                _ => "not found (USB cable in, or Bluetooth paired and connected?)".to_string(),
            },
            self.model.firmware_line(),
            match self.model.layer_notify {
                Some(true) => "live",
                Some(false) => "not supported by firmware",
                None => "waiting",
            },
            self.model.keymap_source,
            self.key_access.describe(),
            self.model.last_error.clone().unwrap_or_default()
        )
    }

    /// Keep an open Options window current (firmware words, status block).
    fn push_options_state(&mut self) {
        if self.options.is_none() {
            return;
        }
        let fw = self.firmware_view();
        let status = self.status_text();
        if let Some(o) = self.options.as_mut() {
            o.send(&ToChild::Firmware(fw));
            o.send(&ToChild::Status(status));
        }
    }

    fn on_update_event(&mut self, event: UpdateEvent) {
        let line = match &event {
            UpdateEvent::Available(v) => format!("Helper {} — {v} available", version::VERSION),
            UpdateEvent::UpToDate => format!("Helper {} — up to date", version::VERSION),
            UpdateEvent::CheckFailed => format!(
                "Helper {} — update check failed (offline?)",
                version::VERSION
            ),
            UpdateEvent::Downloading(v) => format!("Downloading {v}…"),
            UpdateEvent::Verified { .. } => "Installed; restarting into the new version…".to_string(),
            UpdateEvent::Failed(e) => format!("Update failed: {e}"),
        };
        log::info!("update: {line}");
        self.update_line = line;
        self.update_busy = update_thread::update_in_progress_after(&event);
        self.push_options_state();
        match event {
            UpdateEvent::Available(v) => {
                if self.update_notified.as_deref() != Some(v.as_str()) {
                    self.update_notified = Some(v.clone());
                    if let Some(tray) = self.tray.as_mut() {
                        tray.balloon(
                            "OP36 Helper",
                            &format!(
                                "{v} is out (you have {}). Options → Update now.",
                                version::VERSION
                            ),
                        );
                    }
                }
            }
            UpdateEvent::Verified { payload, .. } => self.exit_for_update(payload),
            _ => {}
        }
    }

    /// The new binary is in place: start it (it waits for our socket) and leave.
    fn exit_for_update(&mut self, start: std::path::PathBuf) {
        match std::process::Command::new(&start)
            .env(AFTER_UPDATE_ENV, "1")
            .spawn()
        {
            Ok(_) => {
                log::info!("update: started {}; exiting", start.display());
                self.quit = true;
            }
            Err(e) => {
                self.update_line = format!("Update installed, restart failed: {e}");
                self.update_busy = false;
                self.push_options_state();
            }
        }
    }

    fn start_flash(&mut self, side: vial::protocol::Side) -> Result<(), String> {
        if self.flash.is_some() {
            return Err("a flash is already running".into());
        }
        let bundle = self.bundle().ok_or("no bundled firmware found")?;
        let ev = self.events.clone();
        let cancel = firmware_thread::spawn(Arc::new(move |s| ev(AppEvent::Flash(s))), side, bundle);
        self.flash = Some((side, cancel, firmware::FlashStep::WaitingForDrive));
        log::info!("flash: started for the {side:?} half");
        self.push_options_state();
        Ok(())
    }

    fn on_flash_step(&mut self, step: firmware::FlashStep) {
        let Some((side, _, last)) = self.flash.as_mut() else {
            return;
        };
        log::info!("flash: {:?} half: {:?}", side, step);
        if step.is_final() {
            let side = *side;
            self.flash = None;
            // Keep the final word visible in the dialog until the next action.
            let mut fw = self.firmware_view();
            fw.status = step.text(side);
            if let Some(o) = self.options.as_mut() {
                o.send(&ToChild::Firmware(fw));
            }
        } else {
            *last = step;
            self.push_options_state();
        }
    }

    fn open_options(&mut self) {
        if let Some(o) = self.options.as_mut() {
            o.send(&ToChild::Raise);
            return;
        }
        self.monitors = self.overlay.monitors();
        let init = ToChild::Init {
            settings: self.settings.clone(),
            monitors: self.monitors.clone(),
            status: self.status_text(),
            firmware: self.firmware_view(),
        };
        match OptionsProcess::spawn(self.events.clone(), init) {
            Ok(p) => {
                self.options = Some(p);
                if let Some(tx) = &self.update {
                    let _ = tx.send(UpdateCmd::OptionsOpened);
                }
            }
            Err(e) => log::warn!("{e}"),
        }
    }

    fn on_options(&mut self, msg: FromChild) {
        let result = match msg {
            FromChild::Settings { mut settings, save } => {
                // The window owns only its controls: the hotkey may have toggled the overlay
                // meanwhile, and a live preview must not touch the autostart entry.
                settings.visible = self.settings.visible;
                if !save {
                    settings.autostart = self.settings.autostart;
                }
                self.apply_settings(settings, save)
            }
            FromChild::ReloadKeymap => {
                if let Some(hid) = &self.hid {
                    let _ = hid.send(hid_thread::HidCmd::ReloadKeymap);
                }
                Ok(())
            }
            FromChild::FlashHalf(h) => self.start_flash(match h {
                options_ui::Half::Left => vial::protocol::Side::Left,
                options_ui::Half::Right => vial::protocol::Side::Right,
            }),
            FromChild::CancelFlash => {
                if let Some((_, cancel, _)) = &self.flash {
                    let _ = cancel.send(());
                }
                Ok(())
            }
            FromChild::UpdateNow => {
                self.update_busy = true;
                if let Some(tx) = &self.update {
                    let _ = tx.send(UpdateCmd::UpdateNow);
                }
                self.push_options_state();
                Ok(())
            }
        };
        if let Err(e) = result {
            log::warn!("options: {e}");
            if let Some(o) = self.options.as_mut() {
                o.send(&ToChild::Error(e));
            }
        }
    }

    fn handle_tray(&mut self, cmd: TrayCmd) {
        match cmd {
            TrayCmd::Toggle => {
                let v = !self.settings.visible;
                self.set_visible(v);
            }
            TrayCmd::Options => self.open_options(),
            TrayCmd::Exit => self.quit = true,
        }
    }

    fn on_timer(&mut self, t: Timer) {
        match t {
            Timer::Find => self.on_model_event(ModelEvent::FindExpired),
            Timer::Hide => self.on_hide_timer(),
            Timer::Offline => self.on_offline_timer(),
            Timer::Switch => self.on_switch_tick(),
            Timer::OffHide => {
                let action = self.auto_hide.on_timer();
                self.apply_auto_hide(action);
            }
            Timer::Topmost => {
                if self.overlay_shown {
                    self.overlay.reassert_topmost();
                }
                self.timers.set(Timer::Topmost, TOPMOST_EVERY);
            }
            Timer::ModsResync => self.on_mods_resync(),
        }
    }

    fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Hid(e) => self.on_model_event(ModelEvent::Hid(e)),
            AppEvent::Flash(s) => self.on_flash_step(s),
            AppEvent::Update(e) => self.on_update_event(e),
            AppEvent::Key(k) => self.on_key(k),
            AppEvent::KeyAccess(a) => {
                log::info!("key input: {}", a.describe());
                self.key_access = a;
                self.notify_key_access();
                self.push_options_state();
            }
            AppEvent::Language(hkl) => {
                log::info!(
                    "language: {} ({:?})",
                    crate::input::primary_lang_tag(hkl),
                    xkb::layout_for(hkl)
                );
                self.on_model_event(ModelEvent::Language(hkl));
            }
            AppEvent::ThemeChanged => self.redraw(true),
            AppEvent::DisplayChanged => {
                self.monitors = self.overlay.monitors();
                self.redraw(true);
            }
            AppEvent::Tray(cmd) => self.handle_tray(cmd),
            AppEvent::Control(word) => match word.as_str() {
                "toggle" => self.handle_tray(TrayCmd::Toggle),
                "show" => self.set_visible(true),
                "hide" => self.set_visible(false),
                "options" => self.open_options(),
                "quit" => self.quit = true,
                // What Options → Update now does (scriptable, and the hermetic update test).
                "update-now" => self.on_options(FromChild::UpdateNow),
                other => log::info!("control: unknown command {other:?}"),
            },
            AppEvent::Options(msg) => self.on_options(msg),
            AppEvent::OptionsClosed => self.options = None,
            AppEvent::Quit => self.quit = true,
        }
    }

    fn shutdown(&mut self) {
        if let Some(hid) = self.hid.take() {
            let _ = hid.send(hid_thread::HidCmd::Quit);
        }
        self.options = None;
        self.overlay.reserve(None);
        self.overlay.show(false);
        self.tray = None;
        instance::release();
    }
}

/// Block SIGINT / SIGTERM / SIGHUP in every thread and turn them into `AppEvent::Quit` on one
/// (so the strut and the tray icon are removed on `kill` and at logout).
fn route_signals(events: Sink<AppEvent>) {
    // SAFETY: plain libc signal-mask calls on a locally owned set, before other threads exist.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGHUP);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        let set_copy = set;
        std::thread::Builder::new()
            .name("signals".into())
            .spawn(move || {
                let mut sig: libc::c_int = 0;
                // SAFETY: waits on the set blocked above.
                if libc::sigwait(&set_copy, &mut sig) == 0 {
                    log::info!("signal {sig}: exiting");
                    events(AppEvent::Quit);
                }
            })
            .expect("spawn signal thread");
    }
}

fn usage() -> &'static str {
    "op36-helper [--toggle | --show | --hide | --options | --quit | --version | --snapshot … | --dump-keymap <file> | --probe-layers [secs] | --write-keymap-zmk | --write-taphold]"
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--options-ui") => {
            if let Err(e) = options_ui::run_child() {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            return;
        }
        Some(cmd @ ("--toggle" | "--show" | "--hide" | "--options" | "--quit")) => {
            if let Err(e) = instance::send(&cmd[2..]) {
                eprintln!("op36-helper is not running ({e})");
                std::process::exit(1);
            }
            if cmd == "--quit" && !instance::wait_gone(Duration::from_secs(10)) {
                eprintln!("op36-helper did not exit within 10 s");
                std::process::exit(1);
            }
            return;
        }
        Some("--bench-render") => {
            // Per-frame render cost at the default size, without a display: a cold frame (cache
            // miss: a new layer) and a repeat (a layer seen before).
            let mut model =
                Model::new(Settings::default(), std::fs::read(paths::keymap_cache_file()).ok());
            model.apply(ModelEvent::Hid(model::HidEvent::Connected(
                vial::transport::Transport::Usb,
            )));
            let chars = xkb::XkbCharSource::new();
            let mut r = Renderer::new();
            let n = 30u32;
            let mut cold = Duration::ZERO;
            let mut warm = Duration::ZERO;
            for i in 0..n {
                // A new battery level each time defeats the frame cache: a fresh render of a layer.
                model.apply(ModelEvent::Hid(model::HidEvent::Layer(model::LayerState {
                    active: (i % 3) as u8,
                    mask: if i % 3 == 0 { 0 } else { 1 << (i % 3) },
                    ..Default::default()
                })));
                model.apply(ModelEvent::Hid(model::HidEvent::Battery(
                    vial::protocol::BatteryHalves {
                        left: Some(i as u8),
                        right: Some(90),
                        left_charging: false,
                        right_charging: false,
                    },
                )));
                let frame = model.frame(&chars);
                let t = Instant::now();
                let _ = r.render(&frame, 0.6, 1.0);
                cold += t.elapsed();
                let t = Instant::now();
                let _ = r.render(&frame, 0.6, 1.0);
                warm += t.elapsed();
            }
            println!(
                "cold render {:.2} ms/frame (first one includes the static layers), repeat {:.3} ms",
                cold.as_secs_f64() * 1000.0 / n as f64,
                warm.as_secs_f64() * 1000.0 / n as f64
            );
            return;
        }
        Some("--help" | "-h") => {
            println!("{}", usage());
            return;
        }
        _ => {}
    }
    if let Some(result) = crate::cli::run_cli(&args) {
        if let Err(e) = result {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    if !args.is_empty() {
        eprintln!("unknown arguments: {}\n{}", args.join(" "), usage());
        std::process::exit(2);
    }
    init_logging();
    let first = if std::env::var_os(AFTER_UPDATE_ENV).is_some() {
        instance::claim_waiting(Duration::from_secs(15))
    } else {
        instance::claim()
    };
    if !first {
        log::info!("another instance is running; asking it to toggle");
        let _ = instance::send("toggle");
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel::<AppEvent>();
    let events: Sink<AppEvent> = Arc::new(move |e| {
        let _ = tx.send(e);
    });
    route_signals(events.clone());

    let mut settings = Settings::load(&paths::settings_file());
    settings.autostart = desktop::autostart_enabled();
    log::info!("op36-helper {} starting; settings: {settings:?}", version::VERSION);

    let overlay = match x11::Overlay::create() {
        Ok(o) => o,
        Err(e) => {
            log::error!("{e}");
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let cached = std::fs::read(paths::keymap_cache_file()).ok();
    let model = Model::new(settings.clone(), cached);
    log::info!("keymap source at start: {:?}", model.keymap_source);

    let mut app = App {
        events: events.clone(),
        timers: Timers::default(),
        settings,
        model,
        overlay,
        tray: None,
        hid: None,
        mods: ModTracker::default(),
        renderer: Renderer::new(),
        chars: xkb::XkbCharSource::new(),
        last_frame: None,
        last_scale: 0.0,
        options: None,
        reserved: None,
        last_tooltip: String::new(),
        overlay_shown: false,
        hide_pending: false,
        offline_grace: Default::default(),
        switch_timer_on: false,
        auto_hide: Default::default(),
        flash: None,
        update_busy: false,
        update: None,
        update_notified: None,
        update_line: format!("Helper {}", version::VERSION),
        key_access: keys::Access::NoKeyboards,
        monitors: Vec::new(),
        notified: Default::default(),
        quit: false,
    };

    {
        let ev = events.clone();
        instance::serve(move |w| ev(AppEvent::Control(w)));
    }
    {
        let ev = events.clone();
        app.overlay.watch_display(move || ev(AppEvent::DisplayChanged));
    }
    {
        let ev = events.clone();
        desktop::watch_theme(move || ev(AppEvent::ThemeChanged));
    }
    {
        let ev = events.clone();
        desktop::watch_language(move |hkl| ev(AppEvent::Language(hkl)));
    }
    app.tray = Some(Tray::new(events.clone(), app.settings.visible));
    {
        let ev = events.clone();
        app.hid = Some(hid_thread::spawn(Arc::new(move |e| ev(AppEvent::Hid(e)))));
    }
    {
        let ev = events.clone();
        app.update = Some(update_thread::spawn(
            Arc::new(move |e| ev(AppEvent::Update(e))),
            app.settings.update_check,
        ));
    }
    if let Ok(exe) = std::env::current_exe() {
        update::cleanup_leftovers(&exe);
    }
    app.key_access = keys::spawn(events.clone());
    log::info!("key input: {}", app.key_access.describe());
    app.notify_key_access();

    // Started with the keyboard off: same minute, same hide; a connect within it cancels.
    let action = app.keyboard_off_action();
    app.apply_auto_hide(action);
    app.redraw(true);
    let show = app.settings.visible && app.model.wants_visible();
    app.overlay.show(show);
    app.overlay_shown = show;
    app.redraw(true);
    app.refresh_tooltip();
    app.timers.set(Timer::Topmost, TOPMOST_EVERY);

    while !app.quit {
        match rx.recv_timeout(app.timers.wait()) {
            Ok(event) => app.handle(event),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        for t in app.timers.take_due() {
            app.on_timer(t);
        }
    }
    app.shutdown();
    log::info!("op36-helper exiting");
}
