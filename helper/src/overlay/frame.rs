//! What the renderer draws: a resolved snapshot of the overlay (pure data, `PartialEq` so the UI
//! thread can skip redraws when nothing changed).

use crate::legend::Legend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// `OP36 · RU · Nav` (plus `· offline` / `· no layer data`): tray tooltip and CLI only, the
    /// overlay itself shows no title (the shape says OP36, the tray says the language, the held thumb
    /// key says the layer).
    pub status: String,
    /// How the central half talks to the PC; drawn as a USB / Bluetooth glyph on that half. `None`
    /// while offline.
    pub link: Option<crate::vial::transport::Transport>,
    /// The half that is the central (master); the link glyph sits on it.
    pub link_side: crate::vial::protocol::Side,
    /// The halves are swapping roles: no link glyph, a pulsing arrow between the halves instead.
    pub switching: bool,
    /// A half that is not linked to the master right now: drawn dimmed, with `offline` on its case.
    pub offline_half: Option<crate::vial::protocol::Side>,
    /// Layer whose colour scheme the whole board uses.
    pub layer_color: u8,
    pub keys: Vec<KeyDraw>,
    pub recipes: Vec<Recipe>,
    pub connected: bool,
    /// Battery percentages drawn on the halves (iPhone-style pictograms), when known.
    pub battery_left: Option<u8>,
    pub battery_right: Option<u8>,
    /// The half is on USB power: a bolt in its pictogram.
    pub battery_left_charging: bool,
    pub battery_right_charging: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDraw {
    pub index: u8,
    pub legend: Legend,
    /// Layer whose colour this key uses (differs from `Frame::layer_color` for find-key hits).
    pub color_layer: u8,
    pub halo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub chips: Vec<Chip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chip {
    /// Hold the thumb key that reaches this layer (drawn as the layer-stack icon in its colour).
    Layer(u8),
    /// Hold a modifier: `⇧`, `^`, `⎇`, `⊞`.
    Mod(&'static str),
    /// The final key, labelled in the current language, in the colour of its layer.
    Key(String, u8),
    /// Plain text (`not on OP36`).
    Text(String),
}
