//! Characters a key produces under a Windows keyboard layout.

use super::hid_table::ScanCode;
use crate::model::Hkl;

/// Keyboard layout handles used in tests and as the fallback Latin layout id.
pub const EN: Hkl = 0x0409_0409;
pub const RU: Hkl = 0x0419_0419;

pub trait CharSource {
    /// The printable text `scancode` produces under `hkl`, with or without Shift; `None` for keys that
    /// print nothing (Enter, arrows, dead keys, …).
    fn char_for(&self, hkl: Hkl, scancode: ScanCode, shift: bool) -> Option<String>;
    /// The installed Latin (English) layout used for hints and shortcuts.
    fn en_hkl(&self) -> Hkl;
}

/// `ToUnicodeEx` flag bit 2: do not change the keyboard state (keeps the user's pending dead key).
pub fn to_unicode_flags() -> u32 {
    1 << 2
}

/// Real implementation on top of `MapVirtualKeyExW` + `ToUnicodeExW`.
pub struct WinCharSource {
    en: Hkl,
}

impl WinCharSource {
    pub fn new() -> WinCharSource {
        WinCharSource { en: find_en_layout() }
    }
}

impl Default for WinCharSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CharSource for WinCharSource {
    fn char_for(&self, hkl: Hkl, scancode: ScanCode, shift: bool) -> Option<String> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{HKL, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW, ToUnicodeEx};
        let layout = HKL(hkl as *mut _);
        let vsc = if scancode.extended {
            0xE000 | scancode.code as u32
        } else {
            scancode.code as u32
        };
        // SAFETY: plain keyboard-layout queries with stack buffers of the documented sizes.
        unsafe {
            let vk = MapVirtualKeyExW(vsc, MAPVK_VSC_TO_VK_EX, Some(layout));
            if vk == 0 {
                return None;
            }
            let mut state = [0u8; 256];
            if shift {
                state[0x10] = 0x80; // VK_SHIFT
            }
            let mut buf = [0u16; 8];
            // Plain scan code here: bit 15 of this argument means "key up", so the E0 form must not
            // be passed (MapVirtualKeyExW above does want it).
            let n = ToUnicodeEx(vk, scancode.code as u32, &state, &mut buf, to_unicode_flags(), Some(layout));
            if n <= 0 {
                return None; // 0 = no translation, <0 = dead key
            }
            let text = String::from_utf16_lossy(&buf[..n as usize]);
            if text.chars().all(|c| c.is_control() || c.is_whitespace()) {
                return None;
            }
            Some(text)
        }
    }

    fn en_hkl(&self) -> Hkl {
        self.en
    }
}

/// First installed layout whose primary language is English, else load the US layout.
fn find_en_layout() -> Hkl {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayoutList, HKL, KLF_NOTELLSHELL, LoadKeyboardLayoutW};
    use windows::core::w;
    let mut list = [HKL::default(); 16];
    // SAFETY: documented call with a sized buffer.
    let n = unsafe { GetKeyboardLayoutList(Some(&mut list)) } as usize;
    for h in &list[..n.min(16)] {
        if (h.0 as usize & 0x3FF) == 0x09 {
            return h.0 as Hkl;
        }
    }
    // SAFETY: loading the standard US layout by its KLID string.
    unsafe {
        LoadKeyboardLayoutW(w!("00000409"), KLF_NOTELLSHELL)
            .map(|h| h.0 as Hkl)
            .unwrap_or(EN)
    }
}

/// Test double pre-filled with EN-US and RU for the keys the tests use.
#[cfg(test)]
pub struct FakeCharSource {
    pub map: std::collections::HashMap<(Hkl, u8, bool), &'static str>,
}

#[cfg(test)]
impl Default for FakeCharSource {
    fn default() -> Self {
        let mut map = std::collections::HashMap::new();
        // (scan code, EN plain, EN shift, RU plain, RU shift)
        let rows: &[(u8, &str, &str, &str, &str)] = &[
            (0x10, "q", "Q", "й", "Й"),
            (0x1E, "a", "A", "ф", "Ф"),
            (0x15, "y", "Y", "н", "Н"),
            (0x27, ";", ":", "ж", "Ж"),
            (0x04, "3", "#", "3", "№"),
            (0x1A, "[", "{", "х", "Х"),
            (0x33, ",", "<", "б", "Б"),
        ];
        for (sc, en, ens, ru, rus) in rows {
            map.insert((EN, *sc, false), *en);
            map.insert((EN, *sc, true), *ens);
            map.insert((RU, *sc, false), *ru);
            map.insert((RU, *sc, true), *rus);
        }
        FakeCharSource { map }
    }
}

#[cfg(test)]
impl CharSource for FakeCharSource {
    fn char_for(&self, hkl: Hkl, scancode: ScanCode, shift: bool) -> Option<String> {
        if scancode.extended {
            return None;
        }
        self.map.get(&(hkl, scancode.code, shift)).map(|s| s.to_string())
    }

    fn en_hkl(&self) -> Hkl {
        EN
    }
}
