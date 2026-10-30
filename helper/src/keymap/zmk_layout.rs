//! Boris's original ZMK keymap for the OP36 (vendor `ergohaven-zmk-config/config/op36.keymap`),
//! expressed as RMK key actions so it can be written back over Vial.

use rmk_types::action::KeyboardAction;

use crate::keymap::{Action, HidKeyCode, KeyAction, KeyCode, ModifierCombination};

const SHIFT: u8 = 0b00010;
const LCTRL: u8 = 0b00001;
const LALT: u8 = 0b00100;
const LGUI: u8 = 0b01000;
const RCTRL: u8 = 0b10001;
const RSHIFT: u8 = 0b10010;
const RGUI: u8 = 0b11000;

fn k(h: HidKeyCode) -> KeyAction {
    KeyAction::Single(Action::Key(KeyCode::Hid(h)))
}

/// `LS(x)` in ZMK: the key with Shift.
fn s(h: HidKeyCode) -> KeyAction {
    KeyAction::Single(Action::KeyWithModifier(h, ModifierCombination::from_packed_bits(SHIFT)))
}

/// Tap-hold rules matching the ZMK config (`balanced`, `hold-trigger-on-release`, opposite-hand
/// hold triggers, `require-prior-idle-ms = 150`, `tapping-term-ms = 280`): permissive hold,
/// same-hand presses always tap, flow tap on, 280 ms hold timeout.
pub fn zmk_morse_profile() -> rmk_types::morse::MorseProfile {
    rmk_types::morse::MorseProfile::new(Some(true), Some(rmk_types::morse::MorseMode::PermissiveHold), Some(280), None)
        .with_enable_flow_tap(Some(true))
}

/// Milliseconds since the previous key within which a home-row key resolves as a plain tap at once.
pub const ZMK_PRIOR_IDLE_MS: u16 = 150;

/// `&hml MOD KEY` / `&hmr MOD KEY`: tap the key, hold for the modifier.
fn mt(tap: KeyAction, mods: u8) -> KeyAction {
    let tap_action = match tap {
        KeyAction::Single(a) => a,
        _ => Action::No,
    };
    KeyAction::TapHold(
        tap_action,
        Action::Modifier(ModifierCombination::from_packed_bits(mods)),
        zmk_morse_profile(),
    )
}

fn mo(layer: u8) -> KeyAction {
    KeyAction::Single(Action::LayerOn(layer))
}

fn user(n: u8) -> KeyAction {
    KeyAction::Single(Action::User(n))
}

fn ctl(c: KeyboardAction) -> KeyAction {
    KeyAction::Single(Action::KeyboardControl(c))
}

/// `(layer, key index, action)` for layers 0–3, all 36 keys each, in reading order.
pub fn zmk_layout() -> Vec<(u8, u8, KeyAction)> {
    use HidKeyCode::*;
    const TR: KeyAction = KeyAction::Transparent;
    const NO: KeyAction = KeyAction::No;
    let layers: [[KeyAction; 36]; 4] = [
        // ---- Base ----
        [
            k(Q), k(W), k(E), k(R), k(T), k(Y), k(U), k(I), k(O), k(P),
            mt(k(A), LGUI), mt(k(S), LALT), mt(k(D), SHIFT), mt(k(F), LCTRL), k(G),
            k(H), mt(k(J), RCTRL), mt(k(K), RSHIFT), mt(k(L), LALT), mt(k(Semicolon), RGUI),
            k(Z), k(X), k(C), k(V), k(B), k(N), k(M), k(Comma), k(Dot), k(Quote),
            k(Backspace), mo(1), k(Space), k(Enter), mo(2), k(Tab),
        ],
        // ---- Nav ----
        [
            k(Kc1), k(Kc2), k(Kc3), k(Kc4), k(Kc5), k(Kc6), k(Kc7), k(Kc8), k(Kc9), k(Kc0),
            mt(k(Escape), LGUI), k(LAlt), k(LShift), k(LCtrl), k(Enter),
            k(PrintScreen), mt(k(Left), RCTRL), mt(k(Down), RSHIFT), mt(k(Up), LALT), mt(k(Right), RGUI),
            NO, k(Home), k(Insert), k(End), k(Delete), k(CapsLock), NO, k(PageDown), k(PageUp), NO,
            TR, TR, TR, TR, mo(3), TR,
        ],
        // ---- Symbols ----
        [
            NO, s(Comma), k(Equal), s(Dot), k(Grave), s(Kc6), k(LeftBracket), s(Minus), k(RightBracket), NO,
            mt(k(Backslash), LGUI), mt(s(Kc9), LALT), mt(k(Minus), SHIFT), mt(s(Kc0), LCTRL), s(Equal),
            s(Kc5), mt(s(LeftBracket), RCTRL), mt(k(Semicolon), RSHIFT), mt(s(RightBracket), LALT), mt(s(Kc1), RGUI),
            s(Kc3), s(Kc8), s(Semicolon), k(Slash), NO, NO, s(Backslash), s(Grave), s(Kc7), s(Kc4),
            TR, mo(3), TR, TR, TR, TR,
        ],
        // ---- Adjust ----
        [
            ctl(KeyboardAction::Bootloader), k(F1), k(F2), k(F3), k(F4), user(0), user(1), user(2), user(3), user(7),
            k(LGui), mt(k(F5), LALT), mt(k(F6), SHIFT), mt(k(F7), LCTRL), k(F8),
            ctl(KeyboardAction::OutputBluetooth), mt(k(AudioVolDown), RCTRL), mt(k(AudioMute), RSHIFT), mt(k(AudioVolUp), LALT), k(RGui),
            NO, k(F9), k(F10), k(F11), k(F12), ctl(KeyboardAction::OutputUsb), k(MediaPrevTrack), k(MediaPlayPause), k(MediaNextTrack), NO,
            TR, TR, TR, TR, TR, TR,
        ],
    ];
    layers
        .iter()
        .enumerate()
        .flat_map(|(layer, keys)| {
            keys.iter()
                .enumerate()
                .map(move |(i, a)| (layer as u8, i as u8, *a))
        })
        .collect()
}

/// A combo as the ZMK config had it: key indices (reading order) and the key it produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZmkCombo {
    pub keys: Vec<u8>,
    pub output: KeyAction,
}

/// `kha` (U + I → `[`, which is `х` in Russian) and `hrdsgn` (I + O → `]`, `ъ`), base layer only.
pub fn zmk_combos() -> Vec<ZmkCombo> {
    vec![
        ZmkCombo {
            keys: vec![6, 7],
            output: k(HidKeyCode::LeftBracket),
        },
        ZmkCombo {
            keys: vec![7, 8],
            output: k(HidKeyCode::RightBracket),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Action, HidKeyCode, KeyAction, KeyCode, Keymap};

    #[test]
    fn covers_four_layers_of_36_keys() {
        let entries = zmk_layout();
        assert_eq!(entries.len(), 4 * 36);
        for layer in 0..4u8 {
            for index in 0..36u8 {
                assert!(entries.iter().any(|(l, i, _)| *l == layer && *i == index), "missing {layer}/{index}");
            }
        }
    }

    #[test]
    fn base_layer_matches_the_rmk_factory_base() {
        let factory = Keymap::from_bytes(include_bytes!("../../assets/factory_keymap.bin")).unwrap();
        for (layer, index, action) in zmk_layout().into_iter().filter(|(l, _, _)| *l == 0) {
            let key = &crate::keymap::OP36_KEYS[index as usize];
            let strip = |a: KeyAction| match a {
                KeyAction::TapHold(t, h, _) => KeyAction::TapHold(t, h, Default::default()),
                other => other,
            };
            if index == 18 {
                // ZMK used LAlt on the right hand's L; the RMK factory uses RAlt (AltGr on some layouts).
                assert!(matches!(action, KeyAction::TapHold(Action::Key(KeyCode::Hid(HidKeyCode::L)), Action::Modifier(m), _) if m.left_alt() && !m.right_alt()));
                continue;
            }
            assert_eq!(strip(factory.action(layer, key)), strip(action), "base key {index}");
        }
    }

    #[test]
    fn symbols_layer_has_brackets_back() {
        let entries = zmk_layout();
        let at = |layer: u8, index: u8| entries.iter().find(|(l, i, _)| *l == layer && *i == index).unwrap().2;
        assert_eq!(at(2, 6), KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::LeftBracket))));
        assert_eq!(at(2, 8), KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::RightBracket))));
        assert_eq!(at(1, 0), KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Kc1))));
        assert_eq!(at(3, 0), KeyAction::Single(Action::KeyboardControl(rmk_types::action::KeyboardAction::Bootloader)));
        assert_eq!(at(3, 29), KeyAction::No, "ZMK studio unlock has no RMK equivalent");
        assert_eq!(at(1, 34), KeyAction::Single(Action::LayerOn(3)));
        assert_eq!(at(2, 31), KeyAction::Single(Action::LayerOn(3)));
    }

    #[test]
    fn home_row_keys_carry_zmk_tap_hold_rules() {
        use rmk_types::morse::MorseMode;
        let p = zmk_morse_profile();
        assert_eq!(p.mode(), Some(MorseMode::PermissiveHold));
        assert_eq!(p.unilateral_tap(), Some(true));
        assert_eq!(p.enable_flow_tap(), Some(true));
        assert_eq!(p.hold_timeout_ms(), Some(280));
        let at = |layer: u8, index: u8| zmk_layout().into_iter().find(|(l, i, _)| *l == layer && *i == index).unwrap().2;
        assert!(matches!(at(0, 10), KeyAction::TapHold(_, _, prof) if prof == p));
    }

    #[test]
    fn combos_are_u_i_and_i_o_for_brackets() {
        let c = zmk_combos();
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].keys.as_slice(), c[0].output), ([6, 7].as_slice(), KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::LeftBracket)))));
        assert_eq!((c[1].keys.as_slice(), c[1].output), ([7, 8].as_slice(), KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::RightBracket)))));
    }
}
