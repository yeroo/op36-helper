//! Options dialog: pure helpers here, Win32 dialog in `dialog.rs`, Startup shortcut in `autostart.rs`.

pub mod autostart;
pub mod dialog;

use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};

use crate::overlay::placement::MonitorInfo;
use crate::settings::Hotkey;

pub const SCALE_MIN_PCT: i32 = 30;
pub const SCALE_MAX_PCT: i32 = 120;

pub fn slider_to_scale(pct: i32) -> f32 {
    pct.clamp(SCALE_MIN_PCT, SCALE_MAX_PCT) as f32 / 100.0
}

pub fn scale_to_slider(scale: f32) -> i32 {
    ((scale * 100.0).round() as i32).clamp(SCALE_MIN_PCT, SCALE_MAX_PCT)
}

// msctls_hotkey32 modifier flags (differ from RegisterHotKey's MOD_* bits).
const HOTKEYF_SHIFT: u32 = 0x01;
const HOTKEYF_CONTROL: u32 = 0x02;
const HOTKEYF_ALT: u32 = 0x04;
/// No Win-key flag exists in the control; we park it in a spare bit of the high byte.
const HOTKEYF_WIN_PRIVATE: u32 = 0x80;

/// `HKM_GETHOTKEY` word: low byte = virtual key, high byte = HOTKEYF flags.
pub fn hotkey_to_control_word(hk: &Hotkey) -> u32 {
    let mut flags = 0;
    if hk.modifiers & MOD_SHIFT.0 != 0 {
        flags |= HOTKEYF_SHIFT;
    }
    if hk.modifiers & MOD_CONTROL.0 != 0 {
        flags |= HOTKEYF_CONTROL;
    }
    if hk.modifiers & MOD_ALT.0 != 0 {
        flags |= HOTKEYF_ALT;
    }
    if hk.modifiers & MOD_WIN.0 != 0 {
        flags |= HOTKEYF_WIN_PRIVATE;
    }
    (flags << 8) | (hk.vk & 0xFF)
}

pub fn control_word_to_hotkey(word: u32) -> Hotkey {
    let flags = (word >> 8) & 0xFF;
    let mut modifiers = 0;
    if flags & HOTKEYF_SHIFT != 0 {
        modifiers |= MOD_SHIFT.0;
    }
    if flags & HOTKEYF_CONTROL != 0 {
        modifiers |= MOD_CONTROL.0;
    }
    if flags & HOTKEYF_ALT != 0 {
        modifiers |= MOD_ALT.0;
    }
    if flags & HOTKEYF_WIN_PRIVATE != 0 {
        modifiers |= MOD_WIN.0;
    }
    Hotkey {
        modifiers,
        vk: word & 0xFF,
    }
}

/// The saved monitor when it is not connected right now: it gets an extra combo row so a round
/// trip through the dialog does not silently forget it.
pub fn absent_monitor<'a>(
    selected: &'a Option<String>,
    monitors: &[MonitorInfo],
) -> Option<&'a str> {
    selected
        .as_deref()
        .filter(|name| !monitors.iter().any(|m| m.name == *name))
}

/// Combo row 0 is "Primary"; rows 1..=n are the monitors in order; row n+1 (when present) is the
/// saved-but-absent monitor.
pub fn combo_to_monitor(
    index: usize,
    monitors: &[MonitorInfo],
    selected: &Option<String>,
) -> Option<String> {
    if index == 0 {
        return None;
    }
    match monitors.get(index - 1) {
        Some(m) => Some(m.name.clone()),
        None => absent_monitor(selected, monitors).map(str::to_string),
    }
}

pub fn monitor_to_combo(selected: &Option<String>, monitors: &[MonitorInfo]) -> usize {
    match selected.as_deref() {
        None => 0,
        Some(name) => monitors
            .iter()
            .position(|m| m.name == name)
            .map(|i| i + 1)
            .unwrap_or(monitors.len() + 1),
    }
}

/// Human-readable hotkey, e.g. `Ctrl+Alt+K`.
pub fn hotkey_label(hk: &Hotkey) -> String {
    let mut parts = Vec::new();
    if hk.modifiers & MOD_CONTROL.0 != 0 {
        parts.push("Ctrl".to_string());
    }
    if hk.modifiers & MOD_ALT.0 != 0 {
        parts.push("Alt".to_string());
    }
    if hk.modifiers & MOD_SHIFT.0 != 0 {
        parts.push("Shift".to_string());
    }
    if hk.modifiers & MOD_WIN.0 != 0 {
        parts.push("Win".to_string());
    }
    parts.push(match hk.vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(hk.vk as u8).to_string(),
        0x70..=0x87 => format!("F{}", hk.vk - 0x6F),
        other => format!("VK{other:#04x}"),
    });
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::placement::MonitorInfo;
    use crate::settings::Hotkey;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};

    #[test]
    fn scale_slider_maps_percent_both_ways() {
        assert_eq!(slider_to_scale(60), 0.6);
        assert_eq!(scale_to_slider(1.2), 120);
        assert_eq!(scale_to_slider(slider_to_scale(45)), 45);
    }

    #[test]
    fn hotkey_control_word_round_trips() {
        let hk = Hotkey {
            modifiers: MOD_CONTROL.0 | MOD_ALT.0,
            vk: 0x4B,
        };
        let word = hotkey_to_control_word(&hk);
        assert_eq!(word & 0xFF, 0x4B);
        assert_eq!(control_word_to_hotkey(word), hk);
        let all = Hotkey {
            modifiers: MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0 | MOD_WIN.0,
            vk: 0x4F,
        };
        assert_eq!(control_word_to_hotkey(hotkey_to_control_word(&all)), all);
    }

    #[test]
    fn monitor_combo_index_zero_is_primary() {
        let mons = vec![
            MonitorInfo {
                name: "A".into(),
                work_area: (0, 0, 10, 10),
                monitor_rect: (0, 0, 10, 10),
                is_primary: true,
                dpi: 96,
            },
            MonitorInfo {
                name: "B".into(),
                work_area: (0, 0, 10, 10),
                monitor_rect: (0, 0, 10, 10),
                is_primary: false,
                dpi: 96,
            },
        ];
        assert_eq!(combo_to_monitor(0, &mons, &None), None);
        assert_eq!(combo_to_monitor(2, &mons, &None), Some("B".to_string()));
        assert_eq!(monitor_to_combo(&None, &mons), 0);
        assert_eq!(monitor_to_combo(&Some("B".into()), &mons), 2);
    }

    #[test]
    fn unplugged_saved_monitor_survives_a_dialog_round_trip() {
        let mons = vec![MonitorInfo {
            name: "A".into(),
            work_area: (0, 0, 10, 10),
            monitor_rect: (0, 0, 10, 10),
            is_primary: true,
            dpi: 96,
        }];
        let saved = Some("OFFICE".to_string());
        assert_eq!(absent_monitor(&saved, &mons), Some("OFFICE"));
        assert_eq!(absent_monitor(&Some("A".into()), &mons), None);
        let row = monitor_to_combo(&saved, &mons);
        assert_eq!(row, 2, "extra row after the connected monitors");
        assert_eq!(combo_to_monitor(row, &mons, &saved), saved);
        assert_eq!(
            combo_to_monitor(1, &mons, &saved),
            Some("A".to_string()),
            "user can still pick a live one"
        );
    }

    #[test]
    fn startup_shortcut_path_is_in_user_startup_folder() {
        assert!(crate::paths::startup_shortcut().ends_with(r"Programs\Startup\OP36 Helper.lnk"));
    }
}
