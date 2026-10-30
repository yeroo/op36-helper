//! OP36 layout helper: tray application drawing a click-through overlay of the keymap.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod find_key;
mod firmware;
mod firmware_thread;
mod hid_thread;
mod input;
mod keymap;
mod legend;
mod model;
mod options;
mod overlay;
mod paths;
mod settings;
mod theme;
mod tray;
mod update;
mod update_thread;
mod version;
mod vial;
mod win;

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, RegisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, FindWindowW, GetMessageW, MSG, PostMessageW, PostQuitMessage,
    TranslateMessage, WM_HOTKEY,
};
use windows::core::{PCWSTR, w};

use input::language::{LanguageWatcher, TIMER_LANG};
use input::modifiers::{ModTracker, is_modifier_vk};
use legend::chars::WinCharSource;
use legend::hid_table::scancode_to_hid;
use model::{KeyPress, Model, ModelEvent};
use options::dialog::{DialogAction, FirmwareView, OptionsDialog};
use overlay::appbar::{AppBar, WM_APP_APPBAR, edge_for};
use overlay::frame::Frame;
use overlay::placement::{chosen_monitor, monitors, place_with_reserved};
use overlay::render_d2d::Renderer;
use overlay::window::OverlayWindow;
use settings::NUDGE_PX;
use settings::Settings;
use tray::{Tray, TrayCmd};
use update_thread::{UpdateCmd, UpdateEvent};
use win::msgwindow::{
    self, CLASS_NAME, MsgWindow, SystemRequest, WM_APP_FLASH, WM_APP_HID, WM_APP_LANG,
    WM_APP_TOGGLE, WM_APP_TRAY, WM_APP_UPDATE,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::UI::WindowsAndMessaging::{
    IsDialogMessageW, KillTimer, RegisterWindowMessageW, SetTimer, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_INPUT, WM_SETTINGCHANGE, WM_TIMER,
};

/// Timer id for the periodic "stay on top" nudge.
const TIMER_TOPMOST: usize = 2;
/// Timer id for the find-key display duration.
const TIMER_FIND: usize = 3;
/// Timer id for the "only while held" hide delay.
const TIMER_HIDE: usize = 4;
/// Timer id for the offline grace period: a dropped link is shown as offline only after this.
const TIMER_OFFLINE: usize = 5;
const OFFLINE_GRACE_MS: u32 = 15_000;
/// Timer id for the "switching master" blink; runs only while the frame shows the switch.
const TIMER_SWITCH: usize = 6;
const SWITCH_BLINK_MS: u32 = 500;
const HIDE_DELAY_MS: u32 = 1000;
const LOG_ROTATE_BYTES: u64 = 1_000_000;
use win::to_wide;

const HOTKEY_ID: i32 = 1;
/// Milliseconds since the first call (the model's clock for the switching timeout).
fn uptime_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Arrow nudge hotkeys: Left, Up, Right, Down.
const HOTKEY_NUDGE_BASE: i32 = 2;
const NUDGE_VKS: [u32; 4] = [0x25, 0x26, 0x27, 0x28];

/// `true` when this process is the first instance; the mutex handle is intentionally leaked
/// so it lives as long as the process.
fn claim_single_instance() -> bool {
    // SAFETY: plain Win32 call with a static name; the returned handle is never closed on purpose.
    unsafe {
        let handle = CreateMutexW(None, false, w!("Local\\op36-helper-single-instance"));
        handle.is_ok() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Ask the running instance to toggle its overlay.
fn toggle_running_instance() {
    let class = to_wide(CLASS_NAME);
    // SAFETY: FindWindowW with a class name; PostMessageW to a foreign window is fine.
    unsafe {
        if let Ok(hwnd) = FindWindowW(PCWSTR(class.as_ptr()), None) {
            let _ = PostMessageW(Some(hwnd), WM_APP_TOGGLE, WPARAM(0), LPARAM(0));
        }
    }
}

fn init_logging() {
    use simplelog::{CombinedLogger, Config, LevelFilter, WriteLogger};
    // Keep one rotated backup so the log never grows past ~1 MB.
    let path = paths::log_file();
    if std::fs::metadata(&path)
        .map(|m| m.len() > LOG_ROTATE_BYTES)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&path, path.with_extension("1.txt"));
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::log_file());
    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> = Vec::new();
    if let Ok(file) = file {
        loggers.push(WriteLogger::new(LevelFilter::Info, Config::default(), file));
    }
    #[cfg(debug_assertions)]
    loggers.push(simplelog::TermLogger::new(
        LevelFilter::Debug,
        Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    ));
    let _ = CombinedLogger::init(loggers);
}

struct App {
    hwnd: HWND,
    settings: Settings,
    model: Model,
    overlay: OverlayWindow,
    tray: Option<Tray>,
    hid: Option<std::sync::mpsc::Sender<hid_thread::HidCmd>>,
    mods: ModTracker,
    language: Option<LanguageWatcher>,
    renderer: Renderer,
    chars: WinCharSource,
    last_frame: Option<Frame>,
    last_scale: f32,
    options: Option<OptionsDialog>,
    appbar: AppBar,
    last_tooltip: String,
    /// Whether the overlay window is currently shown (differs from `settings.visible` in
    /// "only while held" mode).
    overlay_shown: bool,
    hide_pending: bool,
    offline_pending: bool,
    switch_timer_on: bool,
    /// A firmware flash in progress: the half and the way to cancel the worker.
    flash: Option<(vial::protocol::Side, std::sync::mpsc::Sender<()>)>,
    update: Option<std::sync::mpsc::Sender<UpdateCmd>>,
    /// Newest release seen, the version the balloon was shown for, and the dialog's update line.
    update_latest: Option<String>,
    update_notified: Option<String>,
    update_line: String,
}

/// (Re)register the toggle and nudge hotkeys for `settings`; returns a human-readable error list.
fn register_hotkeys(hwnd: HWND, settings: &Settings) -> Vec<String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
    let mut errors = Vec::new();
    // SAFETY: (un)registering hotkeys for our own window.
    unsafe {
        for id in HOTKEY_ID..=HOTKEY_NUDGE_BASE + 3 {
            let _ = UnregisterHotKey(Some(hwnd), id);
        }
        let hk = settings.hotkey;
        if let Err(e) = RegisterHotKey(
            Some(hwnd),
            HOTKEY_ID,
            HOT_KEY_MODIFIERS(hk.modifiers),
            hk.vk,
        ) {
            errors.push(format!(
                "toggle hotkey {} unavailable: {e}",
                options::hotkey_label(&hk)
            ));
        }
        for (i, vk) in NUDGE_VKS.iter().enumerate() {
            if let Err(e) = RegisterHotKey(
                Some(hwnd),
                HOTKEY_NUDGE_BASE + i as i32,
                HOT_KEY_MODIFIERS(settings.move_hotkey_mods),
                *vk,
            ) {
                errors.push(format!("nudge hotkey (arrow {i}) unavailable: {e}"));
            }
        }
    }
    errors
}

impl App {
    fn set_visible(&mut self, visible: bool) {
        self.settings.visible = visible;
        let show = visible && self.model.wants_visible();
        self.overlay.show(show);
        self.overlay_shown = show;
        self.hide_pending = false;
        self.redraw(true);
        if let Err(e) = self.settings.save(&paths::settings_file()) {
            log::warn!("could not save settings: {e}");
        }
    }

    /// Current reservation as `(monitor name, thickness)` for placement.
    fn reserved(&self) -> Option<(String, i32)> {
        self.appbar
            .current
            .as_ref()
            .map(|(n, _, t)| (n.clone(), *t))
    }

    /// Keep the app-bar reservation in line with settings, visibility and overlay size.
    fn update_appbar(&mut self, size: (i32, i32), mons: &[overlay::placement::MonitorInfo]) {
        let edge = edge_for(self.settings.anchor);
        if !(self.settings.reserve_space && self.settings.visible) || edge.is_none() {
            self.appbar.release();
            return;
        }
        let mon = chosen_monitor(&self.settings, mons);
        let thickness = size.1 + self.settings.offset.1.max(0);
        self.appbar
            .reserve(&mon.name, mon.monitor_rect, edge.unwrap(), thickness);
    }

    /// Recompute the frame; render, place and present only when something changed.
    fn redraw(&mut self, force: bool) {
        let dark = theme::dark_mode(self.settings.theme);
        let force = force || dark != self.renderer.dark;
        self.renderer.dark = dark;
        let frame = self.model.frame_at(&self.chars, uptime_ms());
        if frame.switching != self.switch_timer_on {
            self.switch_timer_on = frame.switching;
            // SAFETY: timer on our own window.
            unsafe {
                if frame.switching {
                    SetTimer(Some(self.hwnd), TIMER_SWITCH, SWITCH_BLINK_MS, None);
                } else {
                    let _ = KillTimer(Some(self.hwnd), TIMER_SWITCH);
                }
            }
        }
        let mons = monitors();
        if mons.is_empty() {
            return;
        }
        let mon = chosen_monitor(&self.settings, &mons);
        let scale = self.settings.scale * mon.dpi as f32 / 96.0;
        if !force && self.last_frame.as_ref() == Some(&frame) && self.last_scale == scale {
            return;
        }
        match self.renderer.render(&frame, scale, self.settings.opacity) {
            Ok(bitmap) => {
                log::debug!(
                    "rendered {}x{}, scale {scale:.2}, dark {dark}",
                    bitmap.width,
                    bitmap.height
                );
                self.update_appbar((bitmap.width, bitmap.height), &mons);
                let reserved = self.reserved();
                let (x, y, _) = place_with_reserved(
                    &self.settings,
                    (bitmap.width, bitmap.height),
                    &monitors(),
                    reserved.as_ref().map(|(n, t)| (n.as_str(), *t)),
                );
                self.overlay.set_position(x, y);
                if let Err(e) = self.overlay.present(&bitmap) {
                    log::warn!("present failed: {e}");
                }
                self.last_frame = Some(frame);
                self.last_scale = scale;
            }
            Err(e) => log::warn!("render failed: {e}"),
        }
    }

    fn on_raw_input(&mut self, lparam: LPARAM) {
        let Some(key) = input::raw_input::parse(lparam) else {
            return;
        };
        let before = self.mods.current();
        let after = self.mods.apply(&key);
        if after != before {
            self.on_model_event(ModelEvent::Mods(after));
        }
        if key.down && !is_modifier_vk(key.vk) {
            self.on_model_event(ModelEvent::Key(KeyPress {
                hid: scancode_to_hid(key.scancode),
                scancode: key.scancode.code,
                extended: key.scancode.extended,
                mods_at_press: after,
                device: key.device,
            }));
        }
    }

    fn on_language_check(&mut self, resync_mods: bool) {
        if resync_mods {
            let m = self.mods.resync();
            if m != self.model.mods {
                self.on_model_event(ModelEvent::Mods(m));
            }
        }
        if let Some(hkl) = self.language.as_mut().and_then(|w| w.poll()) {
            log::info!(
                "language: {} ({hkl:#x})",
                input::language::primary_lang_tag(hkl)
            );
            self.on_model_event(ModelEvent::Language(hkl));
        }
    }

    fn on_model_event(&mut self, event: ModelEvent) {
        // Offline grace: a Bluetooth link that re-negotiates comes back within seconds; keep the
        // last state and only flag offline when the link stays down.
        match &event {
            ModelEvent::Hid(model::HidEvent::Disconnected) => {
                if self.model.connected && !self.offline_pending {
                    self.offline_pending = true;
                    // SAFETY: timer on our own window.
                    unsafe {
                        SetTimer(Some(self.hwnd), TIMER_OFFLINE, OFFLINE_GRACE_MS, None);
                    }
                    return;
                }
            }
            ModelEvent::Hid(model::HidEvent::Connected(_)) => {
                if self.offline_pending {
                    self.offline_pending = false;
                    // SAFETY: our own timer.
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), TIMER_OFFLINE);
                    }
                }
            }
            _ => {}
        }
        if let ModelEvent::Key(k) = &event {
            if self.settings.find_key && k.hid.is_some() {
                let ms = (self.settings.find_key_secs.clamp(0.5, 5.0) * 1000.0) as u32;
                // SAFETY: (re)arming a timer on our own window; restarts the countdown on every key.
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_FIND, ms, None);
                }
            }
            let t = uptime_ms() as f64;
            log::debug!(
                "key: t={t:.1}ms hid={:?} sc={:#x}{} mods={:?} from {:?}",
                k.hid,
                k.scancode,
                if k.extended { "e" } else { "" },
                k.mods_at_press,
                k.device
            );
        }
        if let ModelEvent::Mods(m) = &event {
            log::debug!("mods: {m:?}");
        }
        self.model.apply_at(event, uptime_ms());
        if let Some(bytes) = self.model.take_pending_cache_write() {
            if let Err(e) = std::fs::write(paths::keymap_cache_file(), bytes) {
                log::warn!("could not cache keymap: {e}");
            } else {
                log::info!("keymap: live ({} layers), cached", self.model.keymap.layers);
            }
        }
        if let Some(tray) = self.tray.as_mut() {
            tray.set_connected(self.model.connected);
            let tip = self.model.status_line_at(uptime_ms());
            if self.last_tooltip != tip {
                tray.set_tooltip(&tip);
                self.last_tooltip = tip;
            }
        }
        self.apply_display_mode();
        self.redraw(false);
        log::debug!(
            "model: connected={} layer={} notify={:?} source={:?}",
            self.model.connected,
            self.model.layer.active,
            self.model.layer_notify,
            self.model.keymap_source
        );
    }

    /// Apply new settings everywhere they matter; `save` persists them.
    fn apply_settings(&mut self, new: Settings, save: bool) -> Result<(), String> {
        let old = std::mem::replace(&mut self.settings, new.clone());
        let mut errors = Vec::new();
        if old.hotkey != new.hotkey || old.move_hotkey_mods != new.move_hotkey_mods {
            errors.extend(register_hotkeys(self.hwnd, &new));
        }
        if old.update_check != new.update_check {
            if let Some(tx) = &self.update {
                let _ = tx.send(UpdateCmd::AutoCheck(new.update_check));
            }
        }
        if old.autostart != new.autostart {
            match std::env::current_exe() {
                Ok(exe) => {
                    if let Err(e) = options::autostart::set(new.autostart, &exe) {
                        errors.push(format!("autostart: {e}"));
                    }
                }
                Err(e) => errors.push(format!("autostart: {e}")),
            }
        }
        self.model.apply(ModelEvent::Settings(new.clone()));
        let show = new.visible && self.model.wants_visible();
        self.overlay.show(show);
        self.overlay_shown = show;
        self.hide_pending = false;
        self.redraw(true);
        if save {
            if let Err(e) = self.settings.save(&paths::settings_file()) {
                errors.push(format!("save: {e}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }

    /// "Only while held": show at once when wanted, hide one second after nothing is held.
    fn apply_display_mode(&mut self) {
        if !self.settings.visible {
            return;
        }
        let wanted = self.model.wants_visible();
        // SAFETY: timers on our own window.
        unsafe {
            if wanted {
                let _ = KillTimer(Some(self.hwnd), TIMER_HIDE);
                if !self.overlay_shown {
                    self.overlay.show(true);
                    self.overlay_shown = true;
                }
            } else if self.overlay_shown && !self.hide_pending {
                SetTimer(Some(self.hwnd), TIMER_HIDE, HIDE_DELAY_MS, None);
                self.hide_pending = true;
            }
        }
    }

    /// Blink tick while the keyboard swaps masters; also keeps the tray tooltip current.
    fn on_switch_tick(&mut self) {
        self.renderer.switch_phase = !self.renderer.switch_phase;
        self.redraw(true);
        if let Some(tray) = self.tray.as_mut() {
            let tip = self.model.status_line_at(uptime_ms());
            if self.last_tooltip != tip {
                tray.set_tooltip(&tip);
                self.last_tooltip = tip;
            }
        }
    }

    fn on_offline_timer(&mut self) {
        // SAFETY: our own timer.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_OFFLINE);
        }
        if self.offline_pending {
            self.offline_pending = false;
            self.on_model_event(ModelEvent::Hid(model::HidEvent::Disconnected));
        }
    }

    fn on_hide_timer(&mut self) {
        // SAFETY: our own timer.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_HIDE);
        }
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
        if let Some(dlg) = &self.options {
            dlg.set_offset(self.settings.offset);
        }
    }

    /// The firmware bundled next to the exe, if complete.
    fn bundle(&self) -> Option<firmware::Bundle> {
        let exe = std::env::current_exe().ok()?;
        firmware::bundle_at(exe.parent()?)
    }

    /// What the Options dialog's Firmware group shows.
    fn firmware_view(&self) -> FirmwareView {
        use firmware::{HalfStatus, half_status, halves_differ};
        let bundle = self.bundle();
        let [left, right] = &self.model.firmware;
        let word = |reported: &Option<String>| match &bundle {
            None => reported.clone().unwrap_or_else(|| "unknown".to_string()),
            Some(b) => match half_status(reported.as_deref(), &b.build) {
                HalfStatus::UpToDate => {
                    format!("{} (up to date)", reported.as_deref().unwrap_or(""))
                }
                HalfStatus::UpdateAvailable => {
                    format!("{} (update available)", reported.as_deref().unwrap_or(""))
                }
                HalfStatus::Unknown => "unknown — flash both halves".to_string(),
            },
        };
        let mut note = String::new();
        if let Some(b) = &bundle {
            let lagging = [left, right]
                .iter()
                .any(|r| half_status(r.as_deref(), &b.build) != HalfStatus::UpToDate);
            if lagging {
                let first = firmware::first_to_flash(left.as_deref(), right.as_deref(), &b.build);
                note.push_str(&format!(
                    "Flash the {} half first, then the other one.",
                    if first == vial::protocol::Side::Left {
                        "left"
                    } else {
                        "right"
                    }
                ));
            }
        }
        if halves_differ(left.as_deref(), right.as_deref()) {
            note.push_str(" The halves run different builds.");
        }
        if !self.model.connected {
            note.push_str(" Keyboard not connected.");
        }
        FirmwareView {
            left: format!("Left half: {}", word(left)),
            right: format!("Right half: {}", word(right)),
            bundled: match &bundle {
                Some(b) => format!("Bundled: {}", b.build),
                None => "Bundled: no firmware files next to the exe".to_string(),
            },
            note: note.trim().to_string(),
            can_flash: bundle.is_some() && self.flash.is_none(),
            update_line: self.update_line.clone(),
        }
    }

    /// Something from the update worker: remember, tell the dialog, balloon once per version, or
    /// hand over to the apply script and leave.
    fn on_update_event(&mut self, event: UpdateEvent) {
        let line = match &event {
            UpdateEvent::Available(v) => format!("Helper {} — {v} available", version::VERSION),
            UpdateEvent::UpToDate => format!("Helper {} — up to date", version::VERSION),
            UpdateEvent::CheckFailed => format!(
                "Helper {} — update check failed (offline?)",
                version::VERSION
            ),
            UpdateEvent::Downloading(v) => format!("Downloading {v}…"),
            UpdateEvent::Verified { .. } => {
                "Verified; restarting into the new version…".to_string()
            }
            UpdateEvent::Failed(e) => format!("Update failed: {e}"),
        };
        log::info!("update: {line}");
        self.update_line = line.clone();
        if let Some(dlg) = &self.options {
            dlg.set_update_line(&line);
        }
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
                self.update_latest = Some(v);
            }
            UpdateEvent::UpToDate => self.update_latest = None,
            UpdateEvent::Verified {
                payload,
                helper,
                channel,
            } => self.exit_for_update(payload, helper, channel),
            _ => {}
        }
    }

    /// Start the detached apply script, then quit: the script waits for this process, installs,
    /// and relaunches the same exe path.
    fn exit_for_update(
        &mut self,
        payload: std::path::PathBuf,
        helper: std::path::PathBuf,
        channel: update::Channel,
    ) {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let started = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-File",
            ])
            .arg(&helper)
            .args([
                "-ProcId",
                &std::process::id().to_string(),
                "-Channel",
                channel.script_name(),
                "-Payload",
            ])
            .arg(&payload)
            .arg("-Exe")
            .arg(&exe)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        match started {
            Ok(_) => {
                log::info!("update: apply script started; exiting");
                // SAFETY: ends the message loop; App's destructors release the app bar and the tray.
                unsafe { PostQuitMessage(0) };
            }
            Err(e) => {
                self.update_line = format!("Update failed: cannot start the apply script: {e}");
                if let Some(dlg) = &self.options {
                    dlg.set_update_line(&self.update_line);
                }
            }
        }
    }

    /// Start flashing `side` from the bundle; the dialog shows the steps as they come.
    fn start_flash(&mut self, side: vial::protocol::Side) -> Result<(), String> {
        if self.flash.is_some() {
            return Err("a flash is already running".into());
        }
        let bundle = self.bundle().ok_or("no bundled firmware next to the exe")?;
        let cancel = firmware_thread::spawn(self.hwnd, side, bundle);
        self.flash = Some((side, cancel));
        if let Some(dlg) = &self.options {
            dlg.set_firmware_status(&firmware::FlashStep::WaitingForDrive.text(side), true);
        }
        log::info!("flash: started for the {side:?} half");
        Ok(())
    }

    fn cancel_flash(&mut self) {
        if let Some((_, cancel)) = &self.flash {
            let _ = cancel.send(());
        }
    }

    /// A step from the flash thread: show it; a final step frees the buttons again.
    fn on_flash_step(&mut self, step: firmware::FlashStep) {
        let Some(side) = self.flash.as_ref().map(|(side, _)| *side) else {
            return;
        };
        let done = step.is_final();
        log::info!("flash: {:?} half: {:?}", side, step);
        if let Some(dlg) = &self.options {
            dlg.set_firmware_status(&step.text(side), !done);
        }
        if done {
            self.flash = None;
        }
    }

    fn status_text(&self) -> String {
        format!(
            "Helper: {}\nKeyboard: {}\nFirmware: {}\nLayer data: {}\nKeymap: {:?}\n{}",
            version::VERSION,
            match (self.model.connected, self.model.transport) {
                (true, Some(t)) => format!("connected ({})", t.label()),
                (true, None) => "connected".to_string(),
                _ => "not found".to_string(),
            },
            self.model.firmware_line(),
            match self.model.layer_notify {
                Some(true) => "live",
                Some(false) => "not supported by firmware",
                None => "waiting",
            },
            self.model.keymap_source,
            self.model.last_error.clone().unwrap_or_default()
        )
    }

    fn handle(&mut self, cmd: TrayCmd) {
        match cmd {
            TrayCmd::Toggle => {
                let v = !self.settings.visible;
                self.set_visible(v);
            }
            TrayCmd::Options => {} // opened by `main`'s handler outside the borrow (modal-safe)
            TrayCmd::Exit => {
                if let Some(hid) = self.hid.take() {
                    let _ = hid.send(hid_thread::HidCmd::Quit);
                }
                self.tray = None; // remove the icon before the loop ends
                // SAFETY: ends the message loop.
                unsafe { PostQuitMessage(0) };
            }
        }
    }
}

/// Push the ZMK tap-hold rules into the keyboard's global behaviour settings (persisted in flash):
/// permissive hold on, unilateral tap on, 280 ms hold timeout, 150 ms prior-idle (flow tap).
fn write_taphold_settings(t: &mut vial::transport::HidTransport) -> Result<(), String> {
    use vial::protocol::*;
    use vial::transport::VialTransport;
    let settings = [
        (SETTING_PERMISSIVE_HOLD, 1u16, "permissive hold"),
        (SETTING_UNILATERAL_TAP, 1, "unilateral tap"),
        (SETTING_MORSE_TIMEOUT, 280, "hold timeout ms"),
        (
            SETTING_PRIOR_IDLE_TIME,
            keymap::zmk_layout::ZMK_PRIOR_IDLE_MS,
            "prior idle ms",
        ),
    ];
    for (key, value, name) in settings {
        let req = req_set_behavior_setting(key, value);
        t.write(&req)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            match t.read(50)? {
                Some(pkt) if pkt[0] == 0xE9 => continue,
                // The firmware echoes the request for the tap-hold keys and answers 0 for device settings.
                Some(pkt) if pkt[0] == 0 || pkt[..4] == req[..4] => break,
                Some(pkt) => return Err(format!("{name}: keyboard replied {:02x?}", &pkt[..4])),
                None if std::time::Instant::now() > deadline => {
                    return Err(format!("{name}: no reply"));
                }
                None => continue,
            }
        }
        let reply = vial::reader::request_vial_setting(t, key)?;
        let now = if value <= 1 {
            reply[1] as u16
        } else {
            u16::from_le_bytes([reply[1], reply[2]])
        };
        println!("{name}: wrote {value}, keyboard now reports {now}");
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    Ok(())
}

/// Developer commands that run without the tray: `--dump-keymap <file>`, `--probe-layers [secs]`.
fn run_cli(args: &[String]) -> Option<Result<(), String>> {
    use vial::protocol::{parse_layer_state, req_layer_state, req_layer_subscribe};
    use vial::reader::{read_keymap, request};
    use vial::transport::{HidTransport, VialTransport};

    let open = || -> Result<HidTransport, String> {
        // Two readers on one Vial channel steal each other's replies (the helper polls battery).
        if !claim_single_instance() {
            return Err("op36-helper is running: exit it from the tray first, the CLI needs the keyboard to itself".into());
        }
        let api = hidapi::HidApi::new().map_err(|e| e.to_string())?;
        HidTransport::open(&api)
            .ok_or_else(|| "OP36 Vial interface not found (keyboard on USB?)".to_string())
    };
    match args.first().map(String::as_str) {
        Some("--dump-keymap") => Some((|| {
            let path = args.get(1).ok_or("usage: --dump-keymap <file>")?;
            let mut t = open()?;
            let km = read_keymap(&mut t, 6, 6)?;
            std::fs::write(path, km.to_bytes()).map_err(|e| e.to_string())?;
            for layer in 0..km.layers {
                let non_empty = keymap::OP36_KEYS
                    .iter()
                    .any(|k| km.action(layer, k) != keymap::KeyAction::No);
                if !non_empty {
                    continue;
                }
                println!("layer {layer} ({})", km.layer_name(layer));
                for k in keymap::OP36_KEYS.iter() {
                    println!(
                        "  {:>2} (r{} c{}): {:?}",
                        k.index,
                        k.row,
                        k.col,
                        km.action(layer, k)
                    );
                }
            }
            println!(
                "wrote {} ({} layers) to {path}",
                km.actions.len(),
                km.layers
            );
            Ok(())
        })()),
        Some("--probe-layers") => Some((|| {
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
            let mut t = open()?;
            let reply = request(&mut t, &req_layer_subscribe(60))?;
            match parse_layer_state(&reply) {
                Some(s) => println!("reply: {s:?}"),
                None => println!(
                    "layer notify: not supported by this firmware (echo {:02x?})",
                    &reply[..4]
                ),
            }
            let _ = request(&mut t, &req_layer_state())?;
            let start = std::time::Instant::now();
            let mut last_renew = start;
            while start.elapsed().as_secs() < secs {
                if last_renew.elapsed().as_secs() >= 20 {
                    t.write(&req_layer_subscribe(60))?;
                    last_renew = std::time::Instant::now();
                }
                if let Some(pkt) = t.read(200)? {
                    if let Some(s) = parse_layer_state(&pkt) {
                        println!("notify: {s:?}");
                    }
                }
            }
            Ok(())
        })()),
        Some("--write-taphold") => Some((|| {
            let mut t = open()?;
            write_taphold_settings(&mut t)
        })()),
        Some("--write-keymap-zmk") => Some((|| {
            // Write Boris's ZMK layout (layers 0–3) and its two bracket combos into the keyboard.
            use keymap::zmk_layout::{zmk_combos, zmk_layout};
            use vial::protocol::{action_to_via_keycode, req_combo_set, req_native_action_set};
            let mut t = open()?;
            let mut written = 0;
            for (layer, index, action) in zmk_layout() {
                let key = &keymap::OP36_KEYS[index as usize];
                let req = req_native_action_set(layer, key.row, key.col, &action)
                    .ok_or_else(|| format!("action does not fit a packet: {action:?}"))?;
                let reply = request(&mut t, &req)?;
                if reply[4] != 0 {
                    return Err(format!(
                        "keyboard rejected layer {layer} key {index} ({action:?}): status {}",
                        reply[4]
                    ));
                }
                written += 1;
                std::thread::sleep(std::time::Duration::from_millis(25)); // let the flash queue drain
            }
            for (i, combo) in zmk_combos().iter().enumerate() {
                let base = |idx: u8| {
                    let key = &keymap::OP36_KEYS[idx as usize];
                    let action = zmk_layout()
                        .into_iter()
                        .find(|(l, k, _)| *l == 0 && *k == idx)
                        .map(|e| e.2)
                        .unwrap_or(keymap::KeyAction::No);
                    let _ = key;
                    action_to_via_keycode(&action).unwrap_or(0)
                };
                let keys: Vec<u16> = combo.keys.iter().map(|k| base(*k)).collect();
                let out =
                    action_to_via_keycode(&combo.output).ok_or("combo output not expressible")?;
                t.write(&req_combo_set(i as u8, &keys, out))?;
                // Vial dynamic-entry replies start with the return code (0 = ok), not the command byte.
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
                loop {
                    match t.read(50)? {
                        Some(pkt) if pkt[0] == 0 => break,
                        Some(pkt) if pkt[0] == 0xE9 => continue,
                        Some(pkt) => return Err(format!("combo {i} rejected: {:02x?}", &pkt[..4])),
                        None if std::time::Instant::now() > deadline => {
                            return Err(format!("combo {i}: no reply"));
                        }
                        None => continue,
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            write_taphold_settings(&mut t)?;
            println!(
                "wrote {written} keys and {} combos; verifying…",
                zmk_combos().len()
            );
            let km = read_keymap(&mut t, 6, 6)?;
            let mut mismatches = 0;
            for (layer, index, action) in zmk_layout() {
                let got = km.action(layer, &keymap::OP36_KEYS[index as usize]);
                if got != action {
                    mismatches += 1;
                    println!(
                        "  layer {layer} key {index}: wanted {action:?}, keyboard has {got:?}"
                    );
                }
            }
            println!("verification: {} mismatches", mismatches);
            if mismatches == 0 {
                Ok(())
            } else {
                Err("keymap differs after write".into())
            }
        })()),
        Some("--version") => {
            println!("{}", version::version_line());
            Some(Ok(()))
        }
        Some("--snapshot") => Some((|| {
            // --snapshot <file.png> [EN|RU] [layer] [scale] [find-hid-hex]: render the keymap offline.
            let path = args
                .get(1)
                .ok_or("usage: --snapshot <file.png> [EN|RU] [layer] [scale]")?;
            let hkl = match args.get(2).map(String::as_str) {
                Some("RU") => legend::chars::RU,
                _ => legend::chars::EN,
            };
            let layer: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            let scale: f32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let mut model = Model::new(
                Settings::default(),
                std::fs::read(paths::keymap_cache_file()).ok(),
            );
            model.apply(ModelEvent::Language(hkl));
            model.apply(ModelEvent::Hid(model::HidEvent::Connected(
                vial::transport::Transport::Usb,
            )));
            model.apply(ModelEvent::Hid(model::HidEvent::Layer(model::LayerState {
                active: layer,
                mask: if layer == 0 { 0 } else { 1 << layer },
                ..Default::default()
            })));
            if let Some(hid) = args
                .get(5)
                .and_then(|h| u8::from_str_radix(h.trim_start_matches("0x"), 16).ok())
            {
                model.apply(ModelEvent::Key(model::KeyPress {
                    hid: Some(hid),
                    scancode: 0,
                    extended: false,
                    mods_at_press: Default::default(),
                    device: model::DeviceKind::Other,
                }));
            }
            // Optional tokens anywhere after the file: charging=left|right|both, master=right, switching.
            let token = |name: &str| {
                args.iter()
                    .find_map(|a| a.strip_prefix(name).map(str::to_string))
            };
            let charging = token("charging=").unwrap_or_default();
            model.apply(ModelEvent::Hid(model::HidEvent::Battery(
                vial::protocol::BatteryHalves {
                    left: Some(85),
                    right: Some(15),
                    left_charging: matches!(charging.as_str(), "left" | "both"),
                    right_charging: matches!(charging.as_str(), "right" | "both"),
                },
            )));
            if args.iter().any(|a| a == "half-offline") {
                model.apply(ModelEvent::Hid(model::HidEvent::Layer(model::LayerState {
                    active: layer,
                    mask: if layer == 0 { 0 } else { 1 << layer },
                    peer_linked: false,
                    peer_link_known: true,
                    ..Default::default()
                })));
            }
            if token("master=").as_deref() == Some("right") {
                model.apply(ModelEvent::Hid(model::HidEvent::Layer(model::LayerState {
                    active: layer,
                    mask: if layer == 0 { 0 } else { 1 << layer },
                    central_is_right: true,
                    peer_linked: !args.iter().any(|a| a == "half-offline"),
                    peer_link_known: true,
                    ..Default::default()
                })));
            }
            if args.iter().any(|a| a == "switching") {
                model.apply(ModelEvent::Hid(model::HidEvent::Switching(
                    vial::protocol::Side::Right,
                )));
                model.apply(ModelEvent::Hid(model::HidEvent::Disconnected));
            }
            let frame = model.frame(&WinCharSource::new());
            let mut renderer = Renderer::new().map_err(|e| e.to_string())?;
            renderer.switch_phase = true;
            let bmp = renderer
                .render(&frame, scale, 1.0)
                .map_err(|e| e.to_string())?;
            // Un-premultiply onto an opaque dark background so the PNG is viewable anywhere.
            let mut rgba = Vec::with_capacity(bmp.premultiplied.len() * 4);
            for px in &bmp.premultiplied {
                let (a, r, g, b) = (
                    (px >> 24) & 0xFF,
                    (px >> 16) & 0xFF,
                    (px >> 8) & 0xFF,
                    px & 0xFF,
                );
                let bg = |c: u32| (c + 0x3A * (255 - a) / 255).min(255) as u8;
                rgba.extend_from_slice(&[bg(r), bg(g), bg(b), 255]);
            }
            let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
            let mut enc = png::Encoder::new(
                std::io::BufWriter::new(file),
                bmp.width as u32,
                bmp.height as u32,
            );
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(|e| e.to_string())?;
            w.write_image_data(&rgba).map_err(|e| e.to_string())?;
            println!(
                "wrote {path} ({}x{}), status: {}",
                bmp.width, bmp.height, frame.status
            );
            Ok(())
        })()),
        _ => None,
    }
}

/// Open the Options dialog (or raise the existing one). Never holds the app borrow while the
/// dialog's own window procedure may run.
fn open_options(app: &Rc<RefCell<App>>) {
    let (settings, status, firmware) = match app.try_borrow() {
        Ok(a) => {
            if let Some(dlg) = &a.options {
                // SAFETY: raising our own window.
                unsafe {
                    let _ =
                        windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(dlg.hwnd());
                }
                return;
            }
            (a.settings.clone(), a.status_text(), a.firmware_view())
        }
        Err(_) => return,
    };
    let app_for_dialog = Rc::clone(app);
    let dialog = OptionsDialog::open(
        settings,
        monitors(),
        status,
        firmware,
        Box::new(move |action| {
            let mut a = app_for_dialog
                .try_borrow_mut()
                .map_err(|_| "application busy".to_string())?;
            match action {
                DialogAction::Settings { mut settings, save } => {
                    // The dialog owns only its controls: the hotkey may have toggled the overlay
                    // meanwhile, and a live preview must not touch the Startup shortcut.
                    settings.visible = a.settings.visible;
                    if !save {
                        settings.autostart = a.settings.autostart;
                    }
                    a.apply_settings(settings, save)
                }
                DialogAction::ReloadKeymap => {
                    if let Some(hid) = &a.hid {
                        let _ = hid.send(hid_thread::HidCmd::ReloadKeymap);
                    }
                    Ok(())
                }
                DialogAction::FlashHalf(side) => a.start_flash(side),
                DialogAction::UpdateNow => {
                    if let Some(tx) = &a.update {
                        let _ = tx.send(UpdateCmd::UpdateNow);
                    }
                    Ok(())
                }
                DialogAction::CancelFlash => {
                    a.cancel_flash();
                    Ok(())
                }
                DialogAction::Closed => {
                    a.options = None;
                    Ok(())
                }
            }
        }),
    );
    if let (Some(dialog), Ok(mut a)) = (dialog, app.try_borrow_mut()) {
        a.options = Some(dialog);
        // A fresh look at the latest release while the dialog is open, unless automatic checks are off.
        if let Some(tx) = &a.update {
            let _ = tx.send(UpdateCmd::OptionsOpened);
        }
    }
}

fn main() {
    // SAFETY: COM for the Startup-folder shortcut; apartment threaded on the UI thread.
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        // Release builds are GUI-subsystem processes without a console; borrow the parent shell's so
        // CLI output is visible (fails harmlessly in the console debug build).
        // SAFETY: plain Win32 call.
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
    if let Some(result) = run_cli(&args) {
        if let Err(e) = result {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    init_logging();
    if !claim_single_instance() {
        log::info!("another instance is running; asking it to toggle");
        toggle_running_instance();
        return;
    }
    let mut settings = Settings::load(&paths::settings_file());
    // The installer's "Start with Windows" task writes the same shortcut Options manages.
    settings.autostart = options::autostart::is_enabled();
    log::info!("op36-helper starting; settings: {settings:?}");

    let overlay = OverlayWindow::create().expect("overlay window");
    let renderer = Renderer::new().expect("Direct2D renderer");

    let cached = std::fs::read(paths::keymap_cache_file()).ok();
    let model = Model::new(settings.clone(), cached);
    log::info!("keymap source at start: {:?}", model.keymap_source);
    let app = Rc::new(RefCell::new(App {
        hwnd: HWND::default(),
        settings,
        model,
        overlay,
        tray: None,
        hid: None,
        mods: ModTracker::default(),
        language: None,
        renderer,
        chars: WinCharSource::new(),
        last_frame: None,
        last_scale: 0.0,
        options: None,
        appbar: AppBar::new(HWND::default()), // re-pointed at the handler window below
        last_tooltip: String::new(),
        overlay_shown: false,
        hide_pending: false,
        offline_pending: false,
        switch_timer_on: false,
        flash: None,
        update: None,
        update_latest: None,
        update_notified: None,
        update_line: format!("Helper {}", version::VERSION),
    }));

    // Explorer broadcasts this after a restart; the tray icon has to be added again.
    // SAFETY: plain registration of a well-known message name.
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let app_for_handler = Rc::clone(&app);
    let msg_window = MsgWindow::create(Box::new(
        move |hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM| {
            match msgwindow::system_request(msg, wparam.0) {
                Some(SystemRequest::AgreeToEnd) => return Some(LRESULT(1)),
                Some(SystemRequest::Exit) => {
                    log::info!("exit requested by the system (installer or session end)");
                    match app_for_handler.try_borrow_mut() {
                        Ok(mut app) => app.handle(TrayCmd::Exit),
                        // SAFETY: ends the message loop; the destructors clean up.
                        Err(_) => unsafe { PostQuitMessage(0) },
                    }
                    return Some(LRESULT(0));
                }
                None => {}
            }
            if msg == taskbar_created && msg != 0 {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    let tip = app.last_tooltip.clone();
                    if let Some(tray) = app.tray.as_mut() {
                        tray.readd(&tip);
                    }
                }
                return Some(LRESULT(0));
            }
            // The tray menu runs a modal loop that re-enters this handler, so never hold a borrow
            // across `tray::show_menu`, and never panic on a re-entrant borrow.
            if msg == WM_INPUT {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.on_raw_input(lparam);
                }
                return None; // Raw Input still wants DefWindowProc to run cleanup
            }
            if msg == WM_TIMER && wparam.0 == TIMER_FIND {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    // SAFETY: our own timer.
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_FIND);
                    }
                    app.on_model_event(ModelEvent::FindExpired);
                }
                return Some(LRESULT(0));
            }
            if msg == WM_APP_UPDATE {
                let event = update_thread::take_event(lparam);
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.on_update_event(*event);
                }
                return Some(LRESULT(0));
            }
            if msg == WM_APP_FLASH {
                let step = firmware_thread::take_step(lparam);
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.on_flash_step(*step);
                }
                return Some(LRESULT(0));
            }
            if msg == WM_TIMER && wparam.0 == TIMER_SWITCH {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.on_switch_tick();
                }
                return Some(LRESULT(0));
            }
            if msg == WM_TIMER && wparam.0 == TIMER_OFFLINE {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.on_offline_timer();
                }
                return Some(LRESULT(0));
            }
            if msg == WM_TIMER && wparam.0 == TIMER_HIDE {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.on_hide_timer();
                }
                return Some(LRESULT(0));
            }
            if msg == WM_TIMER && wparam.0 == TIMER_TOPMOST {
                if let Ok(app) = app_for_handler.try_borrow() {
                    if app.overlay_shown {
                        app.overlay.reassert_topmost();
                    }
                }
                return Some(LRESULT(0));
            }
            if msg == WM_SETTINGCHANGE {
                // Theme or work-area change: redraw and re-place.
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.redraw(true);
                }
                return None;
            }
            if msg == WM_APP_APPBAR {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    let mons = monitors();
                    let rect = Some(chosen_monitor(&app.settings, &mons).monitor_rect);
                    app.appbar.on_notification(wparam.0, lparam, rect);
                }
                return Some(LRESULT(0));
            }
            if msg == WM_DISPLAYCHANGE || msg == WM_DPICHANGED {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.redraw(true);
                }
                return Some(LRESULT(0));
            }
            if msg == WM_APP_LANG || (msg == WM_TIMER && wparam.0 == TIMER_LANG) {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    app.on_language_check(msg == WM_APP_LANG);
                }
                return Some(LRESULT(0));
            }
            if msg == WM_APP_HID {
                let event = hid_thread::take_event(lparam);
                match app_for_handler.try_borrow_mut() {
                    Ok(mut app) => app.on_model_event(ModelEvent::Hid(*event)),
                    Err(_) => log::warn!("dropped HID event while busy: {event:?}"),
                }
                return Some(LRESULT(0));
            }
            if msg == WM_HOTKEY
                && (HOTKEY_NUDGE_BASE..HOTKEY_NUDGE_BASE + 4).contains(&(wparam.0 as i32))
            {
                if let Ok(mut app) = app_for_handler.try_borrow_mut() {
                    let (dx, dy) = match wparam.0 as i32 - HOTKEY_NUDGE_BASE {
                        0 => (-NUDGE_PX, 0),
                        1 => (0, -NUDGE_PX),
                        2 => (NUDGE_PX, 0),
                        _ => (0, NUDGE_PX),
                    };
                    app.nudge(dx, dy);
                }
                return Some(LRESULT(0));
            }
            let cmd = match msg {
                WM_APP_TOGGLE => Some(TrayCmd::Toggle),
                WM_HOTKEY if wparam.0 as i32 == HOTKEY_ID => Some(TrayCmd::Toggle),
                WM_APP_TRAY => match tray::classify_click(lparam.0 as u32 & 0xFFFF) {
                    Some(tray::TrayClick::Command(cmd)) => Some(cmd),
                    Some(tray::TrayClick::Menu) => {
                        let visible = app_for_handler
                            .try_borrow()
                            .map(|a| a.settings.visible)
                            .unwrap_or(true);
                        tray::show_menu(hwnd, visible)
                    }
                    None => None,
                },
                _ => return None,
            };
            if cmd == Some(TrayCmd::Options) {
                open_options(&app_for_handler);
                return Some(LRESULT(0));
            }
            if let Some(cmd) = cmd {
                match app_for_handler.try_borrow_mut() {
                    Ok(mut app) => app.handle(cmd),
                    Err(_) => log::warn!("ignored {cmd:?}: application state busy"),
                }
            }
            Some(LRESULT(0))
        },
    ))
    .expect("message window");

    let hwnd = msg_window.hwnd();
    {
        let mut a = app.borrow_mut();
        a.hwnd = hwnd;
        a.appbar = AppBar::new(hwnd);
        a.tray = Some(Tray::new(hwnd, false));
        a.hid = Some(hid_thread::spawn(hwnd));
        if let Ok(exe) = std::env::current_exe() {
            update::cleanup_leftovers(&exe);
        }
        a.update = Some(update_thread::spawn(hwnd, a.settings.update_check));
        if let Err(e) = input::raw_input::register(hwnd) {
            log::warn!("raw input registration failed: {e}");
        }
        a.language = Some(LanguageWatcher::install(hwnd));
        a.on_language_check(true);
        a.redraw(true);
        let show = a.settings.visible && a.model.wants_visible();
        a.overlay.show(show);
        a.overlay_shown = show;
        // SAFETY: timer on our own window.
        unsafe {
            SetTimer(Some(hwnd), TIMER_TOPMOST, 2000, None);
        }
        for e in register_hotkeys(hwnd, &a.settings) {
            log::warn!("{e}");
        }
    }

    let mut msg = MSG::default();
    // SAFETY: standard message loop.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Tab / Shift+Tab / Esc / Enter in the Options window.
            let dialog = app
                .try_borrow()
                .ok()
                .and_then(|a| a.options.as_ref().map(|d| d.hwnd()));
            if let Some(dlg) = dialog {
                if IsDialogMessageW(dlg, &mut msg).as_bool() {
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    log::info!("op36-helper exiting");
}
