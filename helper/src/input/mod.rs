//! System-wide key input (Raw Input on Windows, evdev on Linux) and the keyboard language.

#[cfg(windows)]
pub mod language;
pub mod modifiers;
#[cfg(windows)]
pub mod raw_input;
pub mod rawkey;

use crate::model::Hkl;

/// `"EN"`, `"RU"`, or the 4-hex language id for anything else.
pub fn primary_lang_tag(hkl: Hkl) -> String {
    match (hkl as usize) & 0x3FF {
        0x09 => "EN".to_string(),
        0x19 => "RU".to_string(),
        _ => format!("{:04X}", (hkl as usize) & 0xFFFF),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_lang_tag_maps_known_ids() {
        assert_eq!(primary_lang_tag(0x04090409), "EN");
        assert_eq!(primary_lang_tag(0x04190419), "RU");
        assert_eq!(primary_lang_tag(0x04070407), "0407");
        assert_eq!(primary_lang_tag(0xF0C00409), "EN"); // layout-id form still carries the language
    }
}
