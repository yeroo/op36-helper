//! "Find a key": where on the OP36 does the key you just pressed live (spec §6.6).

use crate::keymap::{Action, HidKeyCode, KeyAction, KeyCode, Keymap, OP36_KEYS};
use crate::legend::chars::CharSource;
use crate::legend::hid_table::hid_to_scancode;
use crate::legend::labels::label_for_hid;
use crate::model::Hkl;
pub use crate::overlay::frame::{Chip, Recipe};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindQuery {
    pub hid: u8,
    /// Shift was held when the key was pressed.
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub layer: u8,
    pub key_index: u8,
    /// The user must hold Shift in addition to reaching the layer.
    pub needs_shift: bool,
    /// The keymap action already carries Shift (e.g. `SHIFTED(Kc3)`).
    pub implicit_shift: bool,
}

/// `(hid usage, carries shift)` produced by an action's tap part, if it is a plain key.
fn produced_key(action: &KeyAction) -> Option<(u8, bool)> {
    let a = match action {
        KeyAction::Single(a) | KeyAction::Tap(a) | KeyAction::TapHold(a, _, _) => a,
        _ => return None,
    };
    match a {
        Action::Key(KeyCode::Hid(h)) => Some((*h as u16 as u8, false)),
        Action::KeyWithModifier(h, m) => {
            let shift = m.left_shift() || m.right_shift();
            let others = m.left_ctrl() || m.right_ctrl() || m.left_alt() || m.right_alt() || m.left_gui() || m.right_gui();
            (!others).then_some((*h as u16 as u8, shift))
        }
        _ => None,
    }
}

/// Every way to type `q`, best first: exact on the active layer, exact elsewhere, then with an extra
/// Shift. At most three.
pub fn search(keymap: &Keymap, q: FindQuery, active_layer: u8) -> Vec<Candidate> {
    let mut out = Vec::new();
    for layer in 0..keymap.layers {
        for key in OP36_KEYS.iter() {
            let Some((hid, implicit_shift)) = produced_key(&keymap.action(layer, key)) else { continue };
            if hid != q.hid {
                continue;
            }
            let needs_shift = q.shift && !implicit_shift;
            if implicit_shift && !q.shift {
                continue; // SHIFTED(x) never produces plain x
            }
            out.push(Candidate {
                layer,
                key_index: key.index,
                needs_shift,
                implicit_shift,
            });
        }
    }
    out.sort_by_key(|c| (c.needs_shift, c.layer != active_layer, c.layer, c.key_index));
    out.truncate(3);
    out
}

/// The layer keys to hold to reach `target` from the base layer: `(key index, layer it switches to)`,
/// in order. Follows `LayerOn` keys as they resolve on each layer along the way (shortest path).
pub fn layer_path(keymap: &Keymap, target: u8) -> Vec<(u8, u8)> {
    use std::collections::VecDeque;
    if target == 0 {
        return Vec::new();
    }
    let layers = keymap.layers.min(16);
    let mut prev: Vec<Option<(u8, u8, u8)>> = vec![None; layers as usize]; // (from layer, key, to layer)
    let mut seen = vec![false; layers as usize];
    let mut queue = VecDeque::from([0u8]);
    seen[0] = true;
    while let Some(layer) = queue.pop_front() {
        if layer == target {
            break;
        }
        let mask = (1u16 << layer) | 1;
        for key in OP36_KEYS.iter() {
            let (_, action) = keymap.resolve(mask, 0, key);
            let next = match action {
                KeyAction::Single(Action::LayerOn(n)) | KeyAction::Single(Action::LayerOnWithModifier(n, _)) => n,
                KeyAction::TapHold(_, Action::LayerOn(n), _) => n,
                _ => continue,
            };
            if (next as usize) < layers as usize && !seen[next as usize] {
                seen[next as usize] = true;
                prev[next as usize] = Some((layer, key.index, next));
                queue.push_back(next);
            }
        }
    }
    let mut path = Vec::new();
    let mut cur = target;
    while let Some((from, key, to)) = prev[cur as usize] {
        path.push((key, to));
        cur = from;
        if cur == 0 {
            break;
        }
    }
    path.reverse();
    path
}

/// Label of `hid` in the current language (uppercase letters), else its short name.
fn key_label(hid: u8, shift: bool, hkl: Hkl, chars: &dyn CharSource) -> String {
    let from_chars = hid_to_scancode(hid)
        .and_then(|sc| chars.char_for(hkl, sc, shift))
        .map(|s| {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) if c.is_alphabetic() => c.to_uppercase().collect(),
                _ => s,
            }
        });
    from_chars
        .or_else(|| HidKeyCode::from_repr(hid).and_then(label_for_hid).map(str::to_string))
        .unwrap_or_else(|| format!("{hid:#04x}"))
}

/// The chip sequence for one candidate: layer keys, an optional Shift, then the key itself.
pub fn recipe(keymap: &Keymap, c: &Candidate, hid: u8, hkl: Hkl, chars: &dyn CharSource) -> Recipe {
    let mut chips: Vec<Chip> = layer_path(keymap, c.layer).into_iter().map(|(_, to)| Chip::Layer(to)).collect();
    if c.needs_shift {
        chips.push(Chip::Mod("⇧"));
    }
    chips.push(Chip::Key(key_label(hid, c.needs_shift || c.implicit_shift, hkl, chars), c.layer));
    Recipe { chips }
}

/// Recipe shown when nothing produces the key.
pub fn not_found_recipe(hid: u8, shift: bool, hkl: Hkl, chars: &dyn CharSource) -> Recipe {
    Recipe {
        chips: vec![Chip::Key(key_label(hid, shift, hkl, chars), 0), Chip::Text("not on OP36".into())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Keymap;
    use crate::legend::chars::{EN, FakeCharSource};
    use crate::model::{HidEvent, Model, ModelEvent};
    use crate::settings::Settings;

    fn factory() -> Keymap {
        Keymap::from_bytes(include_bytes!("../assets/factory_keymap.bin")).unwrap()
    }

    #[test]
    fn find_equals_points_to_symbols_layer() {
        // The RMK factory keymap has no '[' at all (the ZMK-era pictures did); '=' lives on Symbols.
        let c = search(&factory(), FindQuery { hid: 0x2E, shift: false }, 0);
        assert_eq!((c[0].layer, c[0].key_index, c[0].needs_shift), (2, 2, false));
        assert!(search(&factory(), FindQuery { hid: 0x2F, shift: false }, 0).is_empty());
    }

    #[test]
    fn find_ranks_exact_before_shifted() {
        // ':' = Semicolon + Shift. Symbols layer has SHIFTED(Semicolon) at position 22; the base
        // layer ';' (19) and the Symbols-layer ';' (17) need an extra Shift.
        let c = search(&factory(), FindQuery { hid: 0x33, shift: true }, 0);
        assert_eq!((c[0].layer, c[0].key_index, c[0].implicit_shift, c[0].needs_shift), (2, 22, true, false));
        assert_eq!((c[1].layer, c[1].key_index, c[1].needs_shift), (0, 19, true));
        assert!(c.len() <= 3);
    }

    #[test]
    fn find_on_active_layer_first() {
        let c = search(&factory(), FindQuery { hid: 0x1E, shift: false }, 1); // '1'
        assert_eq!((c[0].layer, c[0].key_index), (1, 0));
    }

    #[test]
    fn find_a_on_base_layer_is_the_home_row_mod_tap() {
        let c = search(&factory(), FindQuery { hid: 0x04, shift: false }, 0);
        assert_eq!((c[0].layer, c[0].key_index), (0, 10));
    }

    #[test]
    fn layer_path_follows_the_keymap_not_a_tri_layer_assumption() {
        let km = factory();
        assert_eq!(layer_path(&km, 0), vec![]);
        assert_eq!(layer_path(&km, 1), vec![(31, 1)]);
        assert_eq!(layer_path(&km, 2), vec![(34, 2)]);
        // Layer 3 is MO(3) on the right thumb while layer 1 is held.
        assert_eq!(layer_path(&km, 3), vec![(31, 1), (34, 3)]);
    }

    #[test]
    fn recipe_for_layer3_lists_both_thumbs() {
        let km = factory();
        let c = Candidate { layer: 3, key_index: 1, needs_shift: false, implicit_shift: false };
        let r = recipe(&km, &c, 0x3A, EN, &FakeCharSource::default()); // F1
        assert_eq!(&r.chips[..2], &[Chip::Layer(1), Chip::Layer(3)]);
        assert_eq!(r.chips[2], Chip::Key("F1".into(), 3));
    }

    #[test]
    fn recipe_adds_shift_chip_and_language_label() {
        let km = factory();
        let c = Candidate { layer: 0, key_index: 19, needs_shift: true, implicit_shift: false };
        let r = recipe(&km, &c, 0x33, crate::legend::chars::RU, &FakeCharSource::default());
        assert_eq!(r.chips, vec![Chip::Mod("⇧"), Chip::Key("Ж".into(), 0)]);
    }

    #[test]
    fn find_result_expires() {
        let mut m = Model::new(Settings::default(), None);
        m.apply(ModelEvent::Language(EN));
        m.apply(ModelEvent::Key(crate::model::KeyPress {
            hid: Some(0x2E),
            scancode: 0x0D,
            extended: false,
            mods_at_press: Default::default(),
            device: crate::model::DeviceKind::Other,
        }));
        let find = m.find.clone().expect("find result");
        assert_eq!(find.candidates[0].key_index, 2);
        let f = m.frame(&FakeCharSource::default());
        assert!(f.keys[2].halo && f.keys[34].halo, "key and its layer thumb get the halo");
        assert_eq!(f.keys[2].color_layer, 2);
        assert_eq!(f.recipes.len(), 1);
        m.apply(ModelEvent::FindExpired);
        assert!(m.find.is_none());
        assert!(m.frame(&FakeCharSource::default()).recipes.is_empty());
    }

    #[test]
    fn modifier_presses_do_not_trigger_find() {
        let mut m = Model::new(Settings::default(), None);
        m.apply(ModelEvent::Key(crate::model::KeyPress {
            hid: Some(0xE1),
            scancode: 0x2A,
            extended: false,
            mods_at_press: Default::default(),
            device: crate::model::DeviceKind::Other,
        }));
        assert!(m.find.is_none());
    }

    #[test]
    fn unknown_key_reports_not_on_op36() {
        let mut m = Model::new(Settings::default(), None);
        m.apply(ModelEvent::Language(EN));
        m.apply(ModelEvent::Key(crate::model::KeyPress {
            hid: Some(0x70), // F21: not in the factory keymap
            scancode: 0x6C,
            extended: false,
            mods_at_press: Default::default(),
            device: crate::model::DeviceKind::Other,
        }));
        let f = m.frame(&FakeCharSource::default());
        assert_eq!(f.recipes.len(), 1);
        assert!(matches!(f.recipes[0].chips.last(), Some(Chip::Text(t)) if t == "not on OP36"));
        let _ = m.apply(ModelEvent::Hid(HidEvent::Connected(crate::vial::transport::Transport::Usb)));
    }
}
