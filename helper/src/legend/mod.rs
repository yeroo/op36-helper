//! What to print on each key: spec §6.3 legend rules.

pub mod chars;
pub mod hid_table;
pub mod labels;

use chars::CharSource;
use hid_table::{ScanCode, hid_to_scancode, is_modifier_usage};
use labels::{label_for_action, label_for_hid, mod_icon};

use crate::keymap::{Action, HidKeyCode, KeyAction, KeyCode, ModifierCombination, PhysicalKey};
use crate::model::{Hkl, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Normal,
    /// Home-row resting position (drawn light grey on the base layer).
    Home,
    /// Nothing on this layer (pale tint).
    Empty,
    /// The layer key currently being held (white with the layer icon).
    HeldLayer,
    /// A key that switches to another layer; the payload is the target layer.
    LayerKey(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Legend {
    pub primary: String,
    pub secondary: Option<String>,
    /// `true` when `secondary` is the Latin hint on a non-Latin layout (drawn small, red, in the
    /// corner); `false` when it is the Shift variant (drawn stacked above the primary).
    pub secondary_is_hint: bool,
    /// Icon for the hold modifier of a mod-tap key.
    pub mod_icon: Option<String>,
    /// Target layer of a layer or layer-tap key.
    pub layer_icon: Option<u8>,
    pub kind: KeyKind,
}

impl Legend {
    fn empty() -> Legend {
        Legend {
            primary: String::new(),
            secondary: None,
            secondary_is_hint: false,
            mod_icon: None,
            layer_icon: None,
            kind: KeyKind::Empty,
        }
    }

    fn text(primary: impl Into<String>, secondary: Option<String>) -> Legend {
        Legend {
            primary: primary.into(),
            secondary,
            secondary_is_hint: false,
            mod_icon: None,
            layer_icon: None,
            kind: KeyKind::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegendCtx {
    /// Keyboard layout of the foreground window.
    pub hkl: Hkl,
    /// Modifiers currently held (physical or home-row).
    pub mods: Modifiers,
    /// Show the Latin letter as a small hint when a non-Latin layout is active.
    pub show_en_hint: bool,
    pub active_layer: u8,
}

/// Letters are shown uppercase in both languages, like on real keycaps.
fn display(text: &str) -> String {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_alphabetic() => {
            // Keycap style: one letter, upper case. Letters whose upper case is longer (ß → SS) stay.
            let up: String = c.to_uppercase().collect();
            if up.chars().count() == 1 { up } else { text.to_string() }
        }
        _ => text.to_string(),
    }
}

fn differs_beyond_case(a: &str, b: &str) -> bool {
    a.to_lowercase() != b.to_lowercase()
}

/// Rules 3–6 for a plain HID key producing characters; `implicit_shift` = the keymap already adds Shift.
fn char_legend(
    sc: ScanCode,
    implicit_shift: bool,
    ctx: &LegendCtx,
    chars: &dyn CharSource,
) -> Option<Legend> {
    let en = chars.en_hkl();
    let shift = implicit_shift || ctx.mods.shift;
    if ctx.mods.ctrl || ctx.mods.alt || ctx.mods.win {
        // Rule 6: shortcuts work on the Latin key whatever the language.
        return chars.char_for(en, sc, implicit_shift).map(|c| Legend::text(display(&c), None));
    }
    let primary = chars.char_for(ctx.hkl, sc, shift)?;
    if ctx.mods.shift {
        // Rule 5: Shift held swaps the primary and hides the secondary.
        return Some(Legend::text(display(&primary), None));
    }
    let mut legend = if ctx.hkl != en && ctx.show_en_hint {
        // Rule 4 (non-Latin layout, hint on): EN form as a hint when it differs.
        let hint = chars
            .char_for(en, sc, shift)
            .filter(|e| differs_beyond_case(e, &primary))
            .map(|e| display(&e));
        let mut l = Legend::text(display(&primary), hint);
        l.secondary_is_hint = l.secondary.is_some();
        l
    } else if !implicit_shift {
        // Rule 4: the Shift variant when it differs by more than case, in whatever layout is active
        // (`:` over `;`, `#` over `3`, `№` over `3` in Russian).
        let shifted = chars
            .char_for(ctx.hkl, sc, true)
            .filter(|s| differs_beyond_case(s, &primary));
        Legend::text(display(&primary), shifted)
    } else {
        Legend::text(display(&primary), None)
    };
    legend.secondary_is_hint &= legend.secondary.is_some();
    Some(legend)
}

fn hid_legend(h: HidKeyCode, implicit: ModifierCombination, ctx: &LegendCtx, chars: &dyn CharSource) -> Legend {
    let usage = h as u16 as u8;
    if is_modifier_usage(usage) {
        return Legend::text(label_for_hid(h).unwrap_or("Mod"), None);
    }
    let implicit_shift = implicit.left_shift() || implicit.right_shift();
    let other_mods = {
        let mut m = implicit;
        m.set_left_shift(false);
        m.set_right_shift(false);
        mod_icon(m)
    };
    let text = hid_to_scancode(usage).and_then(|sc| char_legend(sc, implicit_shift, ctx, chars));
    let mut legend = text.unwrap_or_else(|| Legend::text(label_for_hid(h).unwrap_or(""), None));
    if !other_mods.is_empty() {
        legend.primary = format!("{other_mods}{}", legend.primary);
    }
    legend
}

fn action_legend(a: &Action, ctx: &LegendCtx, chars: &dyn CharSource) -> Legend {
    match a {
        Action::No => Legend::empty(),
        Action::Key(KeyCode::Hid(h)) => hid_legend(*h, ModifierCombination::default(), ctx, chars),
        Action::KeyWithModifier(h, m) => hid_legend(*h, *m, ctx, chars),
        Action::LayerOn(n) | Action::LayerOnWithModifier(n, _) => {
            let mut l = Legend::text("", None);
            l.layer_icon = Some(*n);
            l.kind = if ctx.active_layer == *n {
                KeyKind::HeldLayer
            } else {
                KeyKind::LayerKey(*n)
            };
            l
        }
        other => Legend::text(label_for_action(other).unwrap_or_default(), None),
    }
}

/// The legend for `action` (already resolved from layer `from_layer`) on `key`.
pub fn legend_for(
    action: &KeyAction,
    _from_layer: u8,
    key: &PhysicalKey,
    ctx: &LegendCtx,
    chars: &dyn CharSource,
) -> Legend {
    let mut legend = match action {
        KeyAction::No | KeyAction::Transparent => Legend::empty(),
        KeyAction::Single(a) | KeyAction::Tap(a) => action_legend(a, ctx, chars),
        KeyAction::TapHold(tap, hold, _) => {
            let mut l = action_legend(tap, ctx, chars);
            match hold {
                Action::Modifier(m) => l.mod_icon = Some(mod_icon(*m)),
                Action::LayerOn(n) => {
                    l.layer_icon = Some(*n);
                    if l.kind == KeyKind::Normal {
                        l.kind = KeyKind::LayerKey(*n);
                    }
                }
                other => {
                    if let Some(label) = label_for_action(other) {
                        l.mod_icon = Some(label);
                    }
                }
            }
            l
        }
        KeyAction::Morse(n) => Legend::text(format!("TD {n}"), None),
        other => Legend::text(format!("{other:?}"), None),
    };
    if legend.kind == KeyKind::Normal && key.home_row && ctx.active_layer == 0 {
        legend.kind = KeyKind::Home;
    }
    legend
}

#[cfg(test)]
mod tests {
    use super::chars::{FakeCharSource, EN, RU};
    use super::hid_table::{ScanCode, hid_to_scancode, scancode_to_hid};

    #[test]
    fn display_uppercases_single_letters_but_not_sharp_s() {
        use super::display;
        assert_eq!(display("q"), "Q");
        assert_eq!(display("й"), "Й");
        assert_eq!(display("ß"), "ß");
        assert_eq!(display("Esc"), "Esc");
    }
    use super::*;
    use crate::keymap::{Action, HidKeyCode, KeyAction, KeyCode, ModifierCombination, OP36_KEYS};
    use crate::model::Modifiers;

    fn ctx(hkl: isize, mods: Modifiers, active_layer: u8) -> LegendCtx {
        LegendCtx {
            hkl,
            mods,
            show_en_hint: true,
            active_layer,
        }
    }

    fn key(h: HidKeyCode) -> KeyAction {
        KeyAction::Single(Action::Key(KeyCode::Hid(h)))
    }

    fn shifted(h: HidKeyCode) -> KeyAction {
        KeyAction::Single(Action::KeyWithModifier(h, ModifierCombination::from_packed_bits(0b00010)))
    }

    fn legend(action: KeyAction, c: LegendCtx) -> Legend {
        legend_for(&action, 0, &OP36_KEYS[0], &c, &FakeCharSource::default())
    }

    #[test]
    fn base_letter_in_ru_shows_ru_primary_and_en_hint() {
        let l = legend(key(HidKeyCode::Q), ctx(RU, Modifiers::default(), 0));
        assert_eq!(l.primary, "Й");
        assert_eq!(l.secondary.as_deref(), Some("Q"));
    }

    #[test]
    fn base_letter_in_en_has_no_secondary() {
        let l = legend(key(HidKeyCode::Q), ctx(EN, Modifiers::default(), 0));
        assert_eq!((l.primary.as_str(), l.secondary), ("Q", None));
    }

    #[test]
    fn semicolon_in_en_shows_shift_variant() {
        let l = legend(key(HidKeyCode::Semicolon), ctx(EN, Modifiers::default(), 0));
        assert_eq!(l.primary, ";");
        assert_eq!(l.secondary.as_deref(), Some(":"));
        assert!(!l.secondary_is_hint, "a Shift variant is stacked, not a corner hint");
    }

    #[test]
    fn ru_without_hint_still_shows_shift_variants() {
        let mut c = ctx(RU, Modifiers::default(), 1);
        c.show_en_hint = false;
        let l = legend(key(HidKeyCode::Kc3), c); // Nav layer digit: № over 3 in Russian
        assert_eq!((l.primary.as_str(), l.secondary.as_deref()), ("3", Some("№")));
        assert!(!l.secondary_is_hint);
        let l = legend(key(HidKeyCode::Q), c);
        assert_eq!((l.primary.as_str(), l.secondary), ("Й", None));
    }

    #[test]
    fn ru_hint_is_marked_as_hint() {
        let l = legend(key(HidKeyCode::Q), ctx(RU, Modifiers::default(), 0));
        assert_eq!(l.secondary.as_deref(), Some("Q"));
        assert!(l.secondary_is_hint);
    }

    #[test]
    fn shifted_symbol_layer_key_in_ru() {
        let l = legend(shifted(HidKeyCode::Kc3), ctx(RU, Modifiers::default(), 2));
        assert_eq!(l.primary, "№");
        assert_eq!(l.secondary.as_deref(), Some("#"));
    }

    #[test]
    fn shift_held_swaps_primary() {
        let l = legend(
            key(HidKeyCode::Semicolon),
            ctx(EN, Modifiers { shift: true, ..Default::default() }, 0),
        );
        assert_eq!((l.primary.as_str(), l.secondary), (":", None));
    }

    #[test]
    fn ctrl_held_shows_en_letters() {
        let l = legend(key(HidKeyCode::Q), ctx(RU, Modifiers { ctrl: true, ..Default::default() }, 0));
        assert_eq!((l.primary.as_str(), l.secondary), ("Q", None));
    }

    #[test]
    fn mod_tap_shows_hold_icon() {
        let a = KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::A)),
            Action::Modifier(ModifierCombination::from_packed_bits(0b01000)),
            Default::default(),
        );
        let l = legend(a, ctx(EN, Modifiers::default(), 0));
        assert_eq!(l.primary, "A");
        assert_eq!(l.mod_icon.as_deref(), Some("⊞"));
    }

    #[test]
    fn layer_key_marks_kind() {
        let a = KeyAction::Single(Action::LayerOn(1));
        assert_eq!(legend(a.clone(), ctx(EN, Modifiers::default(), 0)).kind, KeyKind::LayerKey(1));
        assert_eq!(legend(a, ctx(EN, Modifiers::default(), 1)).kind, KeyKind::HeldLayer);
    }

    #[test]
    fn transparent_and_no_render_empty() {
        let l = legend(KeyAction::No, ctx(EN, Modifiers::default(), 0));
        assert_eq!((l.kind, l.primary.as_str()), (KeyKind::Empty, ""));
        assert_eq!(legend(KeyAction::Transparent, ctx(EN, Modifiers::default(), 0)).kind, KeyKind::Empty);
    }

    #[test]
    fn home_row_key_kind_is_home() {
        let c = ctx(EN, Modifiers::default(), 0);
        let l = legend_for(&key(HidKeyCode::A), 0, &OP36_KEYS[10], &c, &FakeCharSource::default());
        assert_eq!(l.kind, KeyKind::Home);
    }

    #[test]
    fn non_printing_keys_use_labels() {
        let l = legend(key(HidKeyCode::Backspace), ctx(RU, Modifiers::default(), 0));
        assert_eq!((l.primary.as_str(), l.secondary), ("Bksp", None));
        let l = legend(KeyAction::Single(Action::User(2)), ctx(EN, Modifiers::default(), 3));
        assert_eq!(l.primary, "BT 2");
        let l = legend(key(HidKeyCode::LGui), ctx(EN, Modifiers::default(), 3));
        assert_eq!(l.primary, "Win");
    }

    #[test]
    fn legend_uses_foreground_layout_for_any_hkl() {
        let de = 0x0407_0407isize;
        let mut chars = FakeCharSource::default();
        chars.map.insert((de, 0x15, false), "z");
        chars.map.insert((de, 0x15, true), "Z");
        let l = legend_for(&key(HidKeyCode::Y), 0, &OP36_KEYS[5], &ctx(de, Modifiers::default(), 0), &chars);
        assert_eq!(l.primary, "Z");
        assert_eq!(l.secondary.as_deref(), Some("Y"));
    }

    #[test]
    fn char_source_passes_no_change_flag() {
        assert_eq!(chars::to_unicode_flags(), 1 << 2);
    }

    #[test]
    fn hid_table_round_trips_letters_and_arrows() {
        assert_eq!(hid_to_scancode(0x04), Some(ScanCode { code: 0x1E, extended: false }));
        assert_eq!(scancode_to_hid(ScanCode { code: 0x1E, extended: false }), Some(0x04));
        assert_eq!(hid_to_scancode(0x50), Some(ScanCode { code: 0x4B, extended: true }));
        assert_eq!(scancode_to_hid(ScanCode { code: 0x4B, extended: true }), Some(0x50));
        assert_eq!(scancode_to_hid(ScanCode { code: 0x4B, extended: false }), Some(0x5C)); // keypad 4
        assert_eq!(hid_to_scancode(0xE3), Some(ScanCode { code: 0x5B, extended: true })); // left win
        assert_eq!(hid_to_scancode(0x02), None);
    }
}
