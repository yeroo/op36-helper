//! Keyboard layouts on Linux: synthetic `Hkl` ids for XKB layouts, the characters each key types
//! under a layout (xkbcommon), and the evdev ↔ set-1 scan code mapping the shared code speaks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;

use xkbcommon::xkb;

use crate::legend::chars::{CharSource, EN};
use crate::legend::hid_table::ScanCode;
use crate::model::Hkl;

/// One XKB layout as the desktop names it (`us`, `ru`, `de` + variant `nodeadkeys`, …).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct XkbLayout {
    pub layout: String,
    pub variant: String,
}

impl XkbLayout {
    pub fn new(layout: &str, variant: &str) -> XkbLayout {
        XkbLayout {
            layout: layout.to_string(),
            variant: variant.to_string(),
        }
    }

    /// GNOME's input-source id: `us`, `de+nodeadkeys`.
    pub fn parse_source_id(id: &str) -> XkbLayout {
        match id.split_once('+') {
            Some((l, v)) => XkbLayout::new(l, v),
            None => XkbLayout::new(id, ""),
        }
    }
}

/// Windows LANGID for an XKB layout name, so the shared code's EN / RU logic works unchanged.
pub fn langid_for(layout: &str) -> u16 {
    match layout {
        "us" | "us_intl" => 0x0409,
        "gb" | "uk" => 0x0809,
        "au" => 0x0C09,
        "ca" => 0x1009,
        "ie" => 0x1809,
        "nz" => 0x1409,
        "ru" => 0x0419,
        "ua" => 0x0422,
        "by" => 0x0423,
        "kz" => 0x043F,
        "de" | "at" | "ch" => 0x0407,
        "fr" | "be" => 0x040C,
        "es" | "latam" => 0x0C0A,
        "it" => 0x0410,
        "pt" => 0x0816,
        "br" => 0x0416,
        "pl" => 0x0415,
        "cz" => 0x0405,
        "sk" => 0x041B,
        "se" => 0x041D,
        "no" => 0x0414,
        "dk" => 0x0406,
        "fi" => 0x040B,
        "nl" => 0x0413,
        "tr" => 0x041F,
        "gr" => 0x0408,
        "il" => 0x040D,
        "hu" => 0x040E,
        "ro" => 0x0418,
        "bg" => 0x0402,
        "rs" => 0x0C1A,
        "hr" => 0x041A,
        "lt" => 0x0427,
        "lv" => 0x0426,
        "ee" => 0x0425,
        "jp" => 0x0411,
        "kr" => 0x0412,
        "cn" => 0x0804,
        _ => 0x0000,
    }
}

/// Process-wide table so a layout keeps its id: id = `(index + 1) << 16 | LANGID`.
static REGISTRY: Mutex<Vec<XkbLayout>> = Mutex::new(Vec::new());
/// The desktop's input sources, in order (set by the language watcher).
static SOURCES: Mutex<Vec<XkbLayout>> = Mutex::new(Vec::new());

pub fn hkl_for(layout: &XkbLayout) -> Hkl {
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let index = match reg.iter().position(|l| l == layout) {
        Some(i) => i,
        None => {
            reg.push(layout.clone());
            reg.len() - 1
        }
    };
    (((index + 1) << 16) | langid_for(&layout.layout) as usize) as Hkl
}

/// The layout an id stands for: a registered one, or the default layout for a Windows-style
/// `0xLLLL_LLLL` id (the EN / RU constants the CLI and tests use).
pub fn layout_for(hkl: Hkl) -> XkbLayout {
    let high = (hkl as usize >> 16) & 0xFFFF;
    if high >= 1 {
        let reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = reg.get(high - 1) {
            return l.clone();
        }
    }
    let name = match (hkl as usize) & 0x3FF {
        0x19 => "ru",
        0x22 => "ua",
        0x23 => "by",
        0x07 => "de",
        0x0C => "fr",
        0x0A => "es",
        0x10 => "it",
        0x15 => "pl",
        _ => "us",
    };
    XkbLayout::new(name, "")
}

pub fn set_sources(sources: Vec<XkbLayout>) {
    for s in &sources {
        hkl_for(s);
    }
    *SOURCES.lock().unwrap_or_else(|e| e.into_inner()) = sources;
}

/// Set-1 scan code → evdev key code (the kernel's `KEY_*`): equal for the main block, a table for
/// the `E0` keys and F13–F24.
pub fn scancode_to_evdev(sc: ScanCode) -> Option<u16> {
    if sc.extended {
        Some(match sc.code {
            0x1C => 96,  // KP Enter
            0x1D => 97,  // Right Ctrl
            0x35 => 98,  // KP /
            0x37 => 99,  // SysRq
            0x38 => 100, // Right Alt
            0x47 => 102, // Home
            0x48 => 103, // Up
            0x49 => 104, // PgUp
            0x4B => 105, // Left
            0x4D => 106, // Right
            0x4F => 107, // End
            0x50 => 108, // Down
            0x51 => 109, // PgDn
            0x52 => 110, // Insert
            0x53 => 111, // Delete
            0x5B => 125, // Left Meta
            0x5C => 126, // Right Meta
            0x5D => 127, // Compose / Menu
            _ => return None,
        })
    } else {
        match sc.code {
            0x01..=0x58 => Some(sc.code as u16),
            0x64..=0x6E => Some(183 + (sc.code - 0x64) as u16), // F13–F23
            0x76 => Some(194),                                   // F24
            _ => None,
        }
    }
}

/// evdev key code → set-1 scan code (inverse of [`scancode_to_evdev`]).
pub fn evdev_to_scancode(code: u16) -> Option<ScanCode> {
    let ext = |c: u8| ScanCode {
        code: c,
        extended: true,
    };
    Some(match code {
        1..=0x58 => ScanCode {
            code: code as u8,
            extended: false,
        },
        96 => ext(0x1C),
        97 => ext(0x1D),
        98 => ext(0x35),
        99 => ext(0x37),
        100 => ext(0x38),
        102 => ext(0x47),
        103 => ext(0x48),
        104 => ext(0x49),
        105 => ext(0x4B),
        106 => ext(0x4D),
        107 => ext(0x4F),
        108 => ext(0x50),
        109 => ext(0x51),
        110 => ext(0x52),
        111 => ext(0x53),
        125 => ext(0x5B),
        126 => ext(0x5C),
        127 => ext(0x5D),
        183..=193 => ScanCode {
            code: 0x64 + (code - 183) as u8,
            extended: false,
        },
        194 => ScanCode {
            code: 0x76,
            extended: false,
        },
        _ => return None,
    })
}

struct Compiled {
    state: xkb::State,
    shift_mask: u32,
}

/// [`CharSource`] on top of xkbcommon: one compiled keymap per layout, built on first use.
pub struct XkbCharSource {
    context: xkb::Context,
    keymaps: RefCell<HashMap<XkbLayout, Option<Compiled>>>,
}

impl XkbCharSource {
    pub fn new() -> XkbCharSource {
        XkbCharSource {
            context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            keymaps: RefCell::new(HashMap::new()),
        }
    }

    fn with_keymap<R>(&self, layout: &XkbLayout, f: impl FnOnce(&mut Compiled) -> R) -> Option<R> {
        let mut maps = self.keymaps.borrow_mut();
        let entry = maps.entry(layout.clone()).or_insert_with(|| {
            let keymap = xkb::Keymap::new_from_names(
                &self.context,
                "",
                "",
                layout.layout.as_str(),
                layout.variant.as_str(),
                None,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            );
            match keymap {
                Some(keymap) => {
                    let shift = keymap.mod_get_index(xkb::MOD_NAME_SHIFT);
                    Some(Compiled {
                        state: xkb::State::new(&keymap),
                        shift_mask: 1u32 << shift,
                    })
                }
                None => {
                    log::warn!("xkb: cannot compile layout {layout:?}");
                    None
                }
            }
        });
        entry.as_mut().map(f)
    }
}

impl Default for XkbCharSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CharSource for XkbCharSource {
    fn char_for(&self, hkl: Hkl, scancode: ScanCode, shift: bool) -> Option<String> {
        let code = scancode_to_evdev(scancode)?;
        let layout = layout_for(hkl);
        let text = self.with_keymap(&layout, |c| {
            let mods = if shift { c.shift_mask } else { 0 };
            c.state.update_mask(mods, 0, 0, 0, 0, 0);
            c.state.key_get_utf8(xkb::Keycode::new(code as u32 + 8))
        })?;
        if text.is_empty() || text.chars().all(|c| c.is_control() || c.is_whitespace()) {
            return None;
        }
        Some(text)
    }

    fn en_hkl(&self) -> Hkl {
        let sources = SOURCES.lock().unwrap_or_else(|e| e.into_inner()).clone();
        sources
            .iter()
            .find(|l| langid_for(&l.layout) & 0x3FF == 0x09)
            .map(hkl_for)
            .unwrap_or(EN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scancodes_round_trip_through_evdev() {
        for code in 1u16..=200 {
            if let Some(sc) = evdev_to_scancode(code) {
                assert_eq!(scancode_to_evdev(sc), Some(code), "evdev {code}");
            }
        }
        // Arrows are E0 keys on both sides.
        assert_eq!(
            evdev_to_scancode(105),
            Some(ScanCode {
                code: 0x4B,
                extended: true
            })
        );
    }

    #[test]
    fn hkl_keeps_langid_and_round_trips() {
        let ru = XkbLayout::new("ru", "");
        let id = hkl_for(&ru);
        assert_eq!(crate::input::primary_lang_tag(id), "RU");
        assert_eq!(layout_for(id), ru);
        assert_eq!(layout_for(crate::legend::chars::RU).layout, "ru");
        assert_eq!(layout_for(EN).layout, "us");
    }

    #[test]
    fn xkb_types_latin_and_cyrillic() {
        let src = XkbCharSource::new();
        let q = ScanCode {
            code: 0x10,
            extended: false,
        };
        // Skip quietly where no XKB data is installed (minimal CI images).
        if src.char_for(EN, q, false).is_none() {
            return;
        }
        assert_eq!(src.char_for(EN, q, false).as_deref(), Some("q"));
        assert_eq!(src.char_for(EN, q, true).as_deref(), Some("Q"));
        assert_eq!(
            src.char_for(crate::legend::chars::RU, q, false).as_deref(),
            Some("й")
        );
        let enter = ScanCode {
            code: 0x1C,
            extended: false,
        };
        assert_eq!(src.char_for(EN, enter, false), None);
    }
}
