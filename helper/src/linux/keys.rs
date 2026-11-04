//! Global key input on Linux from evdev (`/dev/input/event*`): every keyboard, the OP36 told apart
//! by its USB ids. Wayland hands key events only to the focused client, so this is the only way
//! to see Shift / Ctrl / Alt / Super and find-a-key presses system-wide. Reading the devices needs
//! the `input` group (or an ACL); without it the overlay still runs, minus those features.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::AppEvent;
use super::xkb::evdev_to_scancode;
use crate::input::modifiers::{
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
};
use crate::input::rawkey::RawKey;
use crate::legend::hid_table::{ScanCode, hid_to_scancode};
use crate::model::{DeviceKind, Modifiers};
use crate::sink::Sink;
use crate::vial::transport::{PID, VID};

const RESCAN_EVERY: Duration = Duration::from_secs(3);

/// The Windows virtual key the shared modifier tracker expects for an evdev modifier code.
pub fn modifier_vk(code: u16) -> u16 {
    match code {
        42 => VK_LSHIFT,
        54 => VK_RSHIFT,
        29 => VK_LCONTROL,
        97 => VK_RCONTROL,
        56 => VK_LMENU,
        100 => VK_RMENU,
        125 => VK_LWIN,
        126 => VK_RWIN,
        _ => 0,
    }
}

/// An evdev key event as the shared [`RawKey`] (repeats count as presses, like Raw Input's).
pub fn raw_key(code: u16, value: i32, device: DeviceKind) -> Option<RawKey> {
    let sc = evdev_to_scancode(code)?;
    Some(RawKey {
        scancode: sc,
        vk: modifier_vk(code),
        down: value != 0,
        device,
    })
}

/// Whether input devices could be opened; shown in Options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Ok,
    /// Keyboards exist but none could be opened (not in the `input` group).
    Denied,
    NoKeyboards,
}

impl Access {
    pub fn describe(self) -> &'static str {
        match self {
            Access::Ok => "live",
            Access::Denied => {
                "no access to /dev/input (modifiers, find-a-key and hotkeys are off; see README: input group)"
            }
            Access::NoKeyboards => "no keyboards found",
        }
    }
}

fn is_keyboard(dev: &evdev::Device) -> bool {
    dev.supported_keys()
        .is_some_and(|k| k.contains(evdev::KeyCode::KEY_A) && k.contains(evdev::KeyCode::KEY_SPACE))
}

fn kind_of(dev: &evdev::Device) -> DeviceKind {
    let id = dev.input_id();
    if id.vendor() == VID && id.product() == PID {
        DeviceKind::Op36
    } else {
        DeviceKind::Other
    }
}

/// Pressed modifiers across all open keyboards, read from the kernel (resync after the session
/// was locked or a key was released while we were not reading).
#[derive(Default)]
struct HeldState {
    left: Modifiers,
    right: Modifiers,
}

/// Start the device scanner: one reader thread per keyboard, rescanned every few seconds for
/// hot-plugged ones. Returns the initial access state.
pub fn spawn(events: Sink<AppEvent>) -> Access {
    let open: Arc<Mutex<HashSet<PathBuf>>> = Default::default();
    let first = scan(&events, &open);
    let ev = events.clone();
    std::thread::Builder::new()
        .name("evdev-scan".into())
        .spawn(move || {
            let mut last = first;
            loop {
                std::thread::sleep(RESCAN_EVERY);
                let now = scan(&ev, &open);
                if now != last {
                    ev(AppEvent::KeyAccess(now));
                    last = now;
                }
            }
        })
        .expect("spawn evdev scanner");
    first
}

fn scan(events: &Sink<AppEvent>, open: &Arc<Mutex<HashSet<PathBuf>>>) -> Access {
    let Ok(dir) = std::fs::read_dir("/dev/input") else {
        return Access::NoKeyboards;
    };
    let mut any_keyboard = false;
    let mut denied = false;
    let mut paths: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
        })
        .collect();
    paths.sort();
    for path in paths {
        if open.lock().map(|o| o.contains(&path)).unwrap_or(false) {
            any_keyboard = true;
            continue;
        }
        match evdev::Device::open(&path) {
            Ok(dev) => {
                if !is_keyboard(&dev) {
                    continue;
                }
                any_keyboard = true;
                if let Ok(mut o) = open.lock() {
                    o.insert(path.clone());
                }
                log::info!(
                    "keys: reading {} ({})",
                    dev.name().unwrap_or("?"),
                    path.display()
                );
                let ev = events.clone();
                let open = Arc::clone(open);
                let _ = std::thread::Builder::new()
                    .name("evdev".into())
                    .spawn(move || read_device(dev, path, ev, open));
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => denied = true,
            Err(_) => {}
        }
    }
    if any_keyboard {
        Access::Ok
    } else if denied {
        Access::Denied
    } else {
        Access::NoKeyboards
    }
}

fn read_device(
    mut dev: evdev::Device,
    path: PathBuf,
    events: Sink<AppEvent>,
    open: Arc<Mutex<HashSet<PathBuf>>>,
) {
    let kind = kind_of(&dev);
    loop {
        match dev.fetch_events() {
            Ok(batch) => {
                for e in batch {
                    if e.event_type() != evdev::EventType::KEY {
                        continue;
                    }
                    if let Some(k) = raw_key(e.code(), e.value(), kind) {
                        events(AppEvent::Key(k));
                    }
                }
            }
            Err(e) => {
                log::info!("keys: {} closed ({e})", path.display());
                break;
            }
        }
    }
    if let Ok(mut o) = open.lock() {
        o.remove(&path);
    }
}

/// Modifiers held right now across every readable keyboard (a fresh open per call; used only
/// on rare resyncs).
pub fn held_modifiers() -> Option<(Modifiers, Modifiers)> {
    let mut st = HeldState::default();
    let mut any = false;
    for entry in std::fs::read_dir("/dev/input").ok()?.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("event"))
        {
            continue;
        }
        let Ok(dev) = evdev::Device::open(&path) else {
            continue;
        };
        if !is_keyboard(&dev) {
            continue;
        }
        let Ok(keys) = dev.get_key_state() else {
            continue;
        };
        any = true;
        use evdev::KeyCode as K;
        st.left.shift |= keys.contains(K::KEY_LEFTSHIFT);
        st.right.shift |= keys.contains(K::KEY_RIGHTSHIFT);
        st.left.ctrl |= keys.contains(K::KEY_LEFTCTRL);
        st.right.ctrl |= keys.contains(K::KEY_RIGHTCTRL);
        st.left.alt |= keys.contains(K::KEY_LEFTALT);
        st.right.alt |= keys.contains(K::KEY_RIGHTALT);
        st.left.win |= keys.contains(K::KEY_LEFTMETA);
        st.right.win |= keys.contains(K::KEY_RIGHTMETA);
    }
    any.then_some((st.left, st.right))
}

/// The scan code of a Windows virtual key as stored in a [`crate::settings::Hotkey`].
pub fn vk_scancode(vk: u32) -> Option<ScanCode> {
    let hid = match vk {
        0x41..=0x5A => 0x04 + (vk - 0x41) as u8,       // A–Z
        0x31..=0x39 => 0x1E + (vk - 0x31) as u8,       // 1–9
        0x30 => 0x27,                                   // 0
        0x70..=0x7B => 0x3A + (vk - 0x70) as u8,       // F1–F12
        0x7C..=0x87 => 0x68 + (vk - 0x7C) as u8,       // F13–F24
        0x25 => 0x50,                                   // Left
        0x26 => 0x52,                                   // Up
        0x27 => 0x4F,                                   // Right
        0x28 => 0x51,                                   // Down
        0x20 => 0x2C,                                   // Space
        _ => return None,
    };
    hid_to_scancode(hid)
}

/// `MOD_*` bits of the held modifiers.
pub fn mod_bits(m: Modifiers) -> u32 {
    use crate::settings::modbits::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
    let mut bits = 0;
    if m.alt {
        bits |= MOD_ALT;
    }
    if m.ctrl {
        bits |= MOD_CONTROL;
    }
    if m.shift {
        bits |= MOD_SHIFT;
    }
    if m.win {
        bits |= MOD_WIN;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evdev_events_become_raw_keys() {
        let k = raw_key(42, 1, DeviceKind::Other).unwrap();
        assert_eq!(k.vk, VK_LSHIFT);
        assert!(k.down);
        let a = raw_key(30, 2, DeviceKind::Op36).unwrap(); // KEY_A repeat
        assert_eq!(a.scancode.code, 0x1E);
        assert!(a.down, "repeat counts as a press");
        assert!(!raw_key(30, 0, DeviceKind::Other).unwrap().down);
    }

    #[test]
    fn hotkey_vks_map_to_scancodes() {
        assert_eq!(vk_scancode(0x4B).map(|s| s.code), Some(0x25)); // K
        assert_eq!(
            vk_scancode(0x25),
            Some(ScanCode {
                code: 0x4B,
                extended: true
            })
        ); // Left arrow
        assert_eq!(vk_scancode(0x01), None);
    }
}
