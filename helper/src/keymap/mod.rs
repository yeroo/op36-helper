//! The keymap as read from the keyboard: actions per (layer, row, col), plus the physical layout.

pub mod layout;
pub mod zmk_layout;

#[allow(unused_imports)]
pub use layout::{Hand, OP36_KEYS, PhysicalKey};
pub use rmk_types::action::{Action, KeyAction};
pub use rmk_types::keycode::{HidKeyCode, KeyCode};
pub use rmk_types::modifier::ModifierCombination;
use serde::{Deserialize, Serialize};

/// Full keymap: `actions[layer * rows * cols + row * cols + col]`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Keymap {
    pub rows: u8,
    pub cols: u8,
    pub layers: u8,
    pub actions: Vec<KeyAction>,
    pub layer_names: Vec<String>,
}

impl Keymap {
    pub fn empty(rows: u8, cols: u8, layers: u8) -> Keymap {
        Keymap {
            rows,
            cols,
            layers,
            actions: vec![KeyAction::No; rows as usize * cols as usize * layers as usize],
            layer_names: Vec::new(),
        }
    }

    pub fn flat(&self, layer: u8, row: u8, col: u8) -> usize {
        (layer as usize * self.rows as usize + row as usize) * self.cols as usize + col as usize
    }

    #[cfg(test)] // only tests use this
    pub fn set(&mut self, layer: u8, row: u8, col: u8, action: KeyAction) {
        let i = self.flat(layer, row, col);
        self.actions[i] = action;
    }

    pub fn action(&self, layer: u8, key: &PhysicalKey) -> KeyAction {
        self.actions
            .get(self.flat(layer, key.row, key.col))
            .copied()
            .unwrap_or(KeyAction::No)
    }

    /// Resolve what `key` does given the active-layer bitmask and the default layer:
    /// the highest active layer whose action is not `Transparent`, falling back to the default
    /// layer; a `Transparent` default resolves to `No`. Returns `(layer it came from, action)`.
    pub fn resolve(
        &self,
        active_mask: u16,
        default_layer: u8,
        key: &PhysicalKey,
    ) -> (u8, KeyAction) {
        for layer in (0..self.layers.min(16)).rev() {
            let active = active_mask & (1 << layer) != 0 || layer == default_layer;
            if !active {
                continue;
            }
            let action = self.action(layer, key);
            if action != KeyAction::Transparent {
                return (layer, action);
            }
            if layer == default_layer {
                break;
            }
        }
        (default_layer, KeyAction::No)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(self).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Keymap> {
        postcard::from_bytes(bytes).ok()
    }

    /// `Layer n` when the keyboard did not provide a name.
    pub fn layer_name(&self, layer: u8) -> String {
        self.layer_names
            .get(layer as usize)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Layer {layer}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout::key_at;

    fn q() -> KeyAction {
        KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Q)))
    }

    #[test]
    fn flat_index_is_layer_major() {
        let km = Keymap::empty(6, 6, 16);
        assert_eq!(km.flat(0, 0, 0), 0);
        assert_eq!(km.flat(0, 1, 2), 8);
        assert_eq!(km.flat(2, 0, 4), 2 * 36 + 4);
    }

    #[test]
    fn transparent_falls_through_to_lower_active_layer() {
        let mut km = Keymap::empty(6, 6, 4);
        km.set(0, 0, 4, q());
        km.set(1, 0, 4, KeyAction::Transparent);
        let (from, a) = km.resolve(0b0010, 0, key_at(0, 4).unwrap());
        assert_eq!(from, 0);
        assert_eq!(a, q());
    }

    #[test]
    fn highest_active_layer_wins() {
        let mut km = Keymap::empty(6, 6, 4);
        km.set(0, 0, 4, q());
        km.set(
            1,
            0,
            4,
            KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Kc1))),
        );
        km.set(
            2,
            0,
            4,
            KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::LeftBracket))),
        );
        let (from, a) = km.resolve(0b0110, 0, key_at(0, 4).unwrap());
        assert_eq!(from, 2);
        assert_eq!(
            a,
            KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::LeftBracket)))
        );
    }

    #[test]
    fn no_stays_no_and_transparent_on_default_is_no() {
        let mut km = Keymap::empty(6, 6, 4);
        km.set(1, 0, 4, KeyAction::No);
        assert_eq!(
            km.resolve(0b0010, 0, key_at(0, 4).unwrap()),
            (1, KeyAction::No)
        );
        km.set(0, 0, 4, KeyAction::Transparent);
        km.set(1, 0, 4, KeyAction::Transparent);
        assert_eq!(
            km.resolve(0b0010, 0, key_at(0, 4).unwrap()),
            (0, KeyAction::No)
        );
    }

    #[test]
    fn postcard_round_trip() {
        let mut km = Keymap::empty(6, 6, 2);
        km.set(0, 0, 4, q());
        km.layer_names = vec!["Base".into(), "Nav".into()];
        let bytes = km.to_bytes();
        assert_eq!(Keymap::from_bytes(&bytes), Some(km));
        assert_eq!(Keymap::from_bytes(b"garbage"), None);
    }
}
