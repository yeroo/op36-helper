//! Short labels for keys and actions that print nothing.

use crate::keymap::{Action, HidKeyCode, KeyCode, ModifierCombination};
use rmk_types::action::KeyboardAction;

pub fn label_for_hid(usage: HidKeyCode) -> Option<&'static str> {
    use HidKeyCode::*;
    Some(match usage {
        Backspace => "Bksp",
        Enter => "Enter",
        Tab => "Tab",
        Escape => "Esc",
        Space => "Space",
        Delete => "Del",
        Insert => "Ins",
        Home => "Home",
        End => "End",
        PageUp => "Page Up",
        PageDown => "Page Down",
        PrintScreen => "Prt Sc",
        CapsLock => "Caps",
        ScrollLock => "Scroll",
        Pause => "Pause",
        Application => "Menu",
        Left => "←",
        Right => "→",
        Up => "↑",
        Down => "↓",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        F13 => "F13",
        F14 => "F14",
        F15 => "F15",
        F16 => "F16",
        F17 => "F17",
        F18 => "F18",
        F19 => "F19",
        F20 => "F20",
        F21 => "F21",
        F22 => "F22",
        F23 => "F23",
        F24 => "F24",
        AudioVolUp => "Vol+",
        AudioVolDown => "Vol−",
        AudioMute => "Mute",
        MediaPlayPause => "⏯",
        MediaNextTrack => "⏭",
        MediaPrevTrack => "⏮",
        MediaStop => "⏹",
        LCtrl | RCtrl => "Ctrl",
        LShift | RShift => "Shift",
        LAlt | RAlt => "Alt",
        LGui | RGui => "Win",
        _ => return None,
    })
}

/// Icons for held modifiers (Windows idiom): ⊞ Win, ⇧ Shift, ^ Ctrl, ⎇ Alt.
pub fn mod_icon(m: ModifierCombination) -> String {
    let mut s = String::new();
    if m.left_gui() || m.right_gui() {
        s.push('⊞');
    }
    if m.left_alt() || m.right_alt() {
        s.push('⎇');
    }
    if m.left_shift() || m.right_shift() {
        s.push('⇧');
    }
    if m.left_ctrl() || m.right_ctrl() {
        s.push('^');
    }
    s
}

/// Label for actions that are not plain keys. `None` means "this is a plain key, use the chars".
pub fn label_for_action(a: &Action) -> Option<String> {
    Some(match a {
        Action::Key(KeyCode::Hid(h)) => return label_for_hid(*h).map(str::to_string),
        Action::Key(KeyCode::Consumer(c)) => format!("{c:?}"),
        Action::Key(KeyCode::SystemControl(s)) => format!("{s:?}"),
        Action::Modifier(m) => mod_icon(*m),
        Action::User(n) => match n {
            0..=4 => format!("BT {n}"),
            5 => "BT ▶".into(),
            6 => "BT ◀".into(),
            7 => "BT CLR".into(),
            _ => format!("Sym {n}"),
        },
        Action::KeyboardControl(c) => match c {
            KeyboardAction::Bootloader => "Flash".into(),
            KeyboardAction::OutputUsb => "Out USB".into(),
            KeyboardAction::OutputBluetooth => "Out BLE".into(),
            KeyboardAction::OutputAuto => "Out Auto".into(),
            KeyboardAction::CapsWordToggle => "Caps Word".into(),
            KeyboardAction::Reboot => "Reboot".into(),
            other => format!("{other:?}"),
        },
        Action::TriggerMacro(n) => format!("Macro {n}"),
        Action::LayerToggle(n) => format!("TG {n}"),
        Action::LayerToggleOnly(n) => format!("TO {n}"),
        Action::DefaultLayer(n) | Action::PersistentDefaultLayer(n) => format!("DF {n}"),
        Action::OneShotLayer(n) => format!("OSL {n}"),
        Action::OneShotModifier(m) => format!("OS {}", mod_icon(*m)),
        Action::OneShotKey(k) => format!("OS {k:?}"),
        Action::TriLayerLower => "Lower".into(),
        Action::TriLayerUpper => "Raise".into(),
        Action::LayerOn(_) | Action::LayerOnWithModifier(..) | Action::LayerOff(_) => return None,
        Action::KeyWithModifier(..) | Action::No => return None,
        other => format!("{other:?}"),
    })
}
