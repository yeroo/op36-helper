//! Persistent user settings (`%APPDATA%\op36-helper\settings.json`).

use std::path::Path;

use serde::{Deserialize, Serialize};
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL};

/// Where the overlay sits inside the chosen monitor's work area.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum DisplayMode {
    /// Visible whenever the overlay is enabled.
    Always,
    /// Visible only while a layer key or Shift/Ctrl/Alt/Win is held (plus a short fade-out).
    WhileHeld,
}

/// Colour theme: follow Windows' app theme, or force one.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Auto,
    Light,
    Dark,
}

/// A global hotkey as `RegisterHotKey` wants it: `MOD_*` bits and a virtual-key code.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub visible: bool,
    /// Monitor device name (`\.\DISPLAY2`); `None` = primary.
    pub monitor: Option<String>,
    pub anchor: Anchor,
    /// Pixel offset from the anchor edge(s): x to the right, y upward from a bottom anchor.
    pub offset: (i32, i32),
    pub scale: f32,
    pub opacity: f32,
    pub hotkey: Hotkey,
    pub autostart: bool,
    pub display_mode: DisplayMode,
    pub find_key: bool,
    pub find_key_secs: f32,
    pub show_en_hint: bool,
    /// Modifier bits (`MOD_*`) combined with the arrow keys to nudge the overlay by 10 px.
    pub move_hotkey_mods: u32,
    pub theme: Theme,
    /// Register as an application bar so maximized windows stop at the overlay's edge.
    pub reserve_space: bool,
    /// Look for a newer release on GitHub at start and every 12 h (the only automatic network use).
    pub update_check: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visible: true,
            monitor: None,
            anchor: Anchor::Bottom,
            offset: (0, 8),
            scale: 0.6,
            opacity: 0.65,
            hotkey: Hotkey {
                modifiers: MOD_CONTROL.0 | MOD_ALT.0,
                vk: 0x4B, // 'K'
            },
            autostart: false,
            display_mode: DisplayMode::Always,
            find_key: true,
            find_key_secs: 2.0,
            show_en_hint: false,
            move_hotkey_mods: MOD_CONTROL.0 | MOD_ALT.0,
            theme: Theme::Auto,
            reserve_space: false,
            update_check: true,
        }
    }
}

/// Pixels the arrow hotkeys move the overlay per press.
pub const NUDGE_PX: i32 = 10;

impl Settings {
    /// Move the overlay by `(dx, dy)` screen pixels (right/down positive) by adjusting the
    /// anchor-relative offset: x counts away from the anchored horizontal edge, y away from the
    /// anchored vertical edge (bottom anchors count upward).
    pub fn nudge(&mut self, dx: i32, dy: i32) {
        let x_sign = match self.anchor {
            Anchor::TopRight | Anchor::Right | Anchor::BottomRight => -1,
            _ => 1,
        };
        let y_sign = match self.anchor {
            Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => -1,
            _ => 1,
        };
        self.offset.0 += dx * x_sign;
        self.offset.1 += dy * y_sign;
    }

    /// Defaults when the file is missing or unreadable; a warning is logged for a corrupt file.
    pub fn load(path: &Path) -> Settings {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!(
                    "settings file {} is invalid ({e}); using defaults",
                    path.display()
                );
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    /// Writes to `<path>.tmp` then renames, so a crash mid-write never leaves a corrupt file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_string_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL};

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert_eq!((s.scale, s.opacity, s.find_key_secs), (0.6, 0.65, 2.0));
        assert_eq!(s.anchor, Anchor::Bottom);
        assert_eq!(s.offset, (0, 8));
        assert_eq!(
            s.hotkey,
            Hotkey {
                modifiers: MOD_CONTROL.0 | MOD_ALT.0,
                vk: 0x4B
            }
        );
        assert_eq!(s.display_mode, DisplayMode::Always);
        assert!(s.visible && s.find_key && !s.show_en_hint && !s.autostart);
        assert_eq!(s.monitor, None);
    }

    #[test]
    fn nudge_moves_in_screen_direction_for_every_anchor() {
        // Bottom anchor: offset.1 counts upward from the bottom edge, offset.0 rightward from centre.
        let mut s = Settings::default();
        s.nudge(10, 0);
        assert_eq!(s.offset, (10, 8));
        s.nudge(0, -10); // up on screen
        assert_eq!(s.offset, (10, 18));
        // Top-right anchor: x counts leftward from the right edge, y downward from the top.
        s.anchor = Anchor::TopRight;
        s.offset = (0, 0);
        s.nudge(10, 10);
        assert_eq!(s.offset, (-10, 10));
        // Left anchor: x rightward; y (centre) downward.
        s.anchor = Anchor::Left;
        s.offset = (0, 0);
        s.nudge(-10, 10);
        assert_eq!(s.offset, (-10, 10));
    }

    #[test]
    fn theme_and_reserve_defaults() {
        let s = Settings::default();
        assert_eq!(s.theme, Theme::Auto);
        assert!(!s.reserve_space);
    }

    #[test]
    fn move_hotkey_default_is_ctrl_alt() {
        assert_eq!(
            Settings::default().move_hotkey_mods,
            MOD_CONTROL.0 | MOD_ALT.0
        );
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        let mut s = Settings::default();
        s.scale = 0.9;
        s.monitor = Some(r"\.\DISPLAY2".to_string());
        s.anchor = Anchor::BottomRight;
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
    }

    #[test]
    fn corrupt_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Settings::load(&dir.path().join("nope.json")),
            Settings::default()
        );
    }
}
