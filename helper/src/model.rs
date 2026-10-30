//! Pure application state: what the overlay should show, derived from keymap, layer, modifiers,
//! language and find-key state. No Win32 here.

use crate::keymap::Keymap;
use crate::settings::Settings;
pub use crate::vial::protocol::LayerState;

const FACTORY_KEYMAP: &[u8] = include_bytes!("../assets/factory_keymap.bin");

/// Where the current keymap came from, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapSource {
    Live,
    Cached,
    Bundled,
}

/// Held modifiers as seen by Windows (left and right folded together).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub win: bool,
}

/// Windows keyboard layout handle (`HKL` as an integer).
pub type Hkl = isize;

/// Which keyboard a key press came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Op36,
    Other,
}

/// A non-modifier key going down anywhere on the system (filled by the raw-input task).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    /// HID usage of the key, when the scan code maps to one.
    pub hid: Option<u8>,
    pub scancode: u8,
    pub extended: bool,
    pub mods_at_press: Modifiers,
    pub device: DeviceKind,
}

/// Result of a "find a key" search.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FindResult {
    pub query_hid: u8,
    pub shift: bool,
    pub candidates: Vec<crate::find_key::Candidate>,
}

/// Events from the HID thread.
#[derive(Debug, Clone, PartialEq)]
pub enum HidEvent {
    Connected(crate::vial::transport::Transport),
    Disconnected,
    Keymap(Keymap),
    Layer(LayerState),
    LayerNotifySupported(bool),
    Battery(crate::vial::protocol::BatteryHalves),
    /// The halves are swapping roles; the given side becomes the master. The current master
    /// disappears from the host right after this, and the new one appears a few seconds later.
    Switching(crate::vial::protocol::Side),
    /// One half's firmware build id as the keyboard reports it (empty = not known yet).
    Firmware(crate::vial::protocol::Side, String),
    Error(String),
}

/// How long a swap may take before the overlay falls back to the plain offline state.
pub const SWITCHING_TIMEOUT_MS: u64 = 15_000;

#[derive(Debug, Clone, PartialEq)]
pub enum ModelEvent {
    Hid(HidEvent),
    Mods(Modifiers),
    Language(Hkl),
    Key(KeyPress),
    FindExpired,
    Settings(Settings),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub keymap: Keymap,
    pub keymap_source: KeymapSource,
    pub connected: bool,
    pub layer_notify: Option<bool>,
    pub layer: LayerState,
    pub mods: Modifiers,
    pub hkl: Hkl,
    pub find: Option<FindResult>,
    pub settings: Settings,
    pub last_error: Option<String>,
    pub battery: Option<crate::vial::protocol::BatteryHalves>,
    pub transport: Option<crate::vial::transport::Transport>,
    /// The right half is the central (from the layer state's bit 7, or the announced new master).
    pub central_is_right: bool,
    /// A handoff was announced at this time (ms) with this new master; cleared on connect.
    switching: Option<(u64, crate::vial::protocol::Side)>,
    /// Firmware build ids `[left, right]` as reported after the last connect.
    pub firmware: [Option<String>; 2],
    pending_cache_write: Option<Vec<u8>>,
}

impl Model {
    /// `cached` is the content of `keymap.bin` from a previous live read, if any.
    pub fn new(settings: Settings, cached: Option<Vec<u8>>) -> Model {
        let (keymap, keymap_source) = match cached.as_deref().and_then(Keymap::from_bytes) {
            Some(km) => (km, KeymapSource::Cached),
            None => (
                Keymap::from_bytes(FACTORY_KEYMAP).expect("bundled factory keymap decodes"),
                KeymapSource::Bundled,
            ),
        };
        Model {
            keymap,
            keymap_source,
            connected: false,
            layer_notify: None,
            layer: LayerState::default(),
            mods: Modifiers::default(),
            hkl: 0,
            find: None,
            settings,
            last_error: None,
            battery: None,
            transport: None,
            central_is_right: false,
            switching: None,
            firmware: [None, None],
            pending_cache_write: None,
        }
    }

    /// `apply_at` with the clock at zero (tests and the offline snapshot).
    pub fn apply(&mut self, event: ModelEvent) {
        self.apply_at(event, 0);
    }

    /// A swap announced less than `SWITCHING_TIMEOUT_MS` ago and not yet followed by a connect.
    pub fn is_switching(&self, now_ms: u64) -> bool {
        self.switching
            .is_some_and(|(started, _)| now_ms.saturating_sub(started) < SWITCHING_TIMEOUT_MS)
    }

    pub fn apply_at(&mut self, event: ModelEvent, now_ms: u64) {
        match event {
            ModelEvent::Hid(HidEvent::Connected(t)) => {
                self.connected = true;
                self.transport = Some(t);
                self.last_error = None;
                let announced = self.is_switching(now_ms);
                if let Some((_, side)) = self.switching.take()
                    && announced
                {
                    self.central_is_right = side == crate::vial::protocol::Side::Right;
                }
            }
            ModelEvent::Hid(HidEvent::Switching(side)) => self.switching = Some((now_ms, side)),
            ModelEvent::Hid(HidEvent::Firmware(side, build)) => {
                self.firmware[(side == crate::vial::protocol::Side::Right) as usize] =
                    (!build.is_empty()).then_some(build);
            }
            ModelEvent::Hid(HidEvent::Disconnected) => {
                self.connected = false;
                self.layer = LayerState::default();
                self.layer_notify = None;
                self.battery = None;
                self.transport = None;
                self.firmware = [None, None];
            }
            ModelEvent::Hid(HidEvent::Battery(b)) => self.battery = Some(b),
            ModelEvent::Hid(HidEvent::Keymap(km)) => {
                self.pending_cache_write = Some(km.to_bytes());
                self.keymap = km;
                self.keymap_source = KeymapSource::Live;
            }
            ModelEvent::Hid(HidEvent::Layer(state)) => {
                self.layer = state;
                self.central_is_right = state.central_is_right;
            }
            ModelEvent::Hid(HidEvent::LayerNotifySupported(s)) => self.layer_notify = Some(s),
            ModelEvent::Hid(HidEvent::Error(e)) => self.last_error = Some(e),
            ModelEvent::Mods(m) => self.mods = m,
            ModelEvent::Language(hkl) => self.hkl = hkl,
            ModelEvent::Key(press) => {
                if let Some(hid) = press
                    .hid
                    .filter(|h| !crate::legend::hid_table::is_modifier_usage(*h))
                {
                    if self.settings.find_key {
                        let q = crate::find_key::FindQuery {
                            hid,
                            shift: press.mods_at_press.shift,
                        };
                        self.find = Some(FindResult {
                            query_hid: hid,
                            shift: q.shift,
                            candidates: crate::find_key::search(&self.keymap, q, self.layer.active),
                        });
                    }
                }
            }
            ModelEvent::FindExpired => self.find = None,
            ModelEvent::Settings(s) => self.settings = s,
        }
    }

    /// In "only while held" mode the overlay shows while a non-default layer is active, a modifier
    /// is held, or a find-key result is on screen; in "always" mode it is simply enabled.
    pub fn wants_visible(&self) -> bool {
        match self.settings.display_mode {
            crate::settings::DisplayMode::Always => true,
            crate::settings::DisplayMode::WhileHeld => {
                self.layer.active != self.layer.default
                    || self.mods != Modifiers::default()
                    || self.find.is_some()
            }
        }
    }

    /// `left: v…, right: v…` for the Options dialog; `unknown` for a half that has not reported.
    pub fn firmware_line(&self) -> String {
        let show = |b: &Option<String>| b.clone().unwrap_or_else(|| "unknown".to_string());
        format!(
            "left {}, right {}",
            show(&self.firmware[0]),
            show(&self.firmware[1])
        )
    }

    /// `status_line_at` with the clock at zero.
    #[cfg(test)]
    pub fn status_line(&self) -> String {
        self.status_line_at(0)
    }

    /// The status line: `OP36 · RU · Nav`, with offline / switching / no-layer-data markers.
    pub fn status_line_at(&self, now_ms: u64) -> String {
        let mut s = format!(
            "OP36 · {} · {}",
            crate::input::language::primary_lang_tag(self.hkl),
            self.keymap.layer_name(self.layer.active)
        );
        if self.is_switching(now_ms) {
            s.push_str(" · switching master…");
        } else if !self.connected {
            s.push_str(" · offline");
        } else {
            if let Some(t) = self.transport {
                s.push_str(" · ");
                s.push_str(t.label());
            }
            if self.layer_notify == Some(false) {
                s.push_str(" · no layer data");
            }
        }
        s
    }

    /// `frame_at` with the clock at zero.
    pub fn frame(
        &self,
        chars: &dyn crate::legend::chars::CharSource,
    ) -> crate::overlay::frame::Frame {
        self.frame_at(chars, 0)
    }

    /// Resolve every key through the keymap and the legend rules into one drawable snapshot.
    pub fn frame_at(
        &self,
        chars: &dyn crate::legend::chars::CharSource,
        now_ms: u64,
    ) -> crate::overlay::frame::Frame {
        use crate::legend::{LegendCtx, legend_for};
        use crate::overlay::frame::{Frame, KeyDraw};
        let active = self.layer.active;
        let mask = self.layer.mask | (1u16 << active.min(15));
        let ctx = LegendCtx {
            hkl: self.hkl,
            mods: self.mods,
            show_en_hint: self.settings.show_en_hint,
            active_layer: active,
        };
        let mut keys: Vec<KeyDraw> = crate::keymap::OP36_KEYS
            .iter()
            .map(|key| {
                let (from, action) = self.keymap.resolve(mask, self.layer.default, key);
                KeyDraw {
                    index: key.index,
                    legend: legend_for(&action, from, key, &ctx, chars),
                    color_layer: active,
                    halo: false,
                }
            })
            .collect();
        let mut recipes = Vec::new();
        if let Some(find) = &self.find {
            if find.candidates.is_empty() {
                recipes.push(crate::find_key::not_found_recipe(
                    find.query_hid,
                    find.shift,
                    self.hkl,
                    chars,
                ));
            }
            for c in &find.candidates {
                recipes.push(crate::find_key::recipe(
                    &self.keymap,
                    c,
                    find.query_hid,
                    self.hkl,
                    chars,
                ));
                let key = &crate::keymap::OP36_KEYS[c.key_index as usize];
                let action = self.keymap.action(c.layer, key);
                let draw = &mut keys[c.key_index as usize];
                draw.halo = true;
                draw.color_layer = c.layer;
                if c.layer != active {
                    let find_ctx = LegendCtx {
                        active_layer: c.layer,
                        ..ctx
                    };
                    draw.legend = legend_for(&action, c.layer, key, &find_ctx, chars);
                }
                for (thumb, _) in crate::find_key::layer_path(&self.keymap, c.layer) {
                    keys[thumb as usize].halo = true;
                }
            }
        }
        let battery = self.battery.filter(|_| self.connected);
        Frame {
            status: self.status_line_at(now_ms),
            link: self
                .transport
                .filter(|_| self.connected && !self.is_switching(now_ms)),
            link_side: if self.central_is_right {
                crate::vial::protocol::Side::Right
            } else {
                crate::vial::protocol::Side::Left
            },
            // The app holds a disconnect back for its offline grace; the swap shows from the packet on.
            switching: self.is_switching(now_ms),
            offline_half: (self.connected && self.layer.peer_link_known && !self.layer.peer_linked)
                .then(|| {
                    if self.central_is_right {
                        crate::vial::protocol::Side::Left
                    } else {
                        crate::vial::protocol::Side::Right
                    }
                }),
            layer_color: active,
            keys,
            recipes,
            connected: self.connected,
            battery_left: battery.and_then(|b| b.left),
            battery_right: battery.and_then(|b| b.right),
            battery_left_charging: battery.is_some_and(|b| b.left_charging),
            battery_right_charging: battery.is_some_and(|b| b.right_charging),
        }
    }

    /// Bytes to persist as the keymap cache, once per live read.
    pub fn take_pending_cache_write(&mut self) -> Option<Vec<u8>> {
        self.pending_cache_write.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Keymap;

    fn connected_model() -> Model {
        let mut m = Model::new(Settings::default(), None);
        m.apply(ModelEvent::Hid(HidEvent::Connected(
            crate::vial::transport::Transport::Usb,
        )));
        m
    }

    #[test]
    fn starts_from_bundled_factory_keymap_when_no_cache() {
        let m = Model::new(Settings::default(), None);
        assert_eq!(m.keymap_source, KeymapSource::Bundled);
        assert_eq!(m.keymap.layers, 16);
        assert!(!m.connected);
    }

    #[test]
    fn cached_keymap_is_preferred_over_bundled() {
        let mut km = Keymap::empty(6, 6, 3);
        km.layer_names = vec!["Cached".into()];
        let m = Model::new(Settings::default(), Some(km.to_bytes()));
        assert_eq!(m.keymap_source, KeymapSource::Cached);
        assert_eq!(m.keymap.layers, 3);
    }

    #[test]
    fn model_marks_disconnected() {
        let mut m = connected_model();
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState {
            active: 2,
            ..Default::default()
        })));
        assert_eq!(m.layer.active, 2);
        m.apply(ModelEvent::Hid(HidEvent::Disconnected));
        assert!(!m.connected);
        assert_eq!(m.layer.active, 0);
        assert_eq!(m.keymap.layers, 16, "keymap is kept while offline");
    }

    #[test]
    fn model_prefers_live_keymap_and_caches_it() {
        let mut m = connected_model();
        let km = Keymap::empty(6, 6, 2);
        m.apply(ModelEvent::Hid(HidEvent::Keymap(km.clone())));
        assert_eq!(m.keymap_source, KeymapSource::Live);
        assert_eq!(m.keymap, km);
        assert_eq!(m.take_pending_cache_write(), Some(km.to_bytes()));
        assert_eq!(m.take_pending_cache_write(), None);
    }

    #[test]
    fn layer_notify_support_is_recorded() {
        let mut m = connected_model();
        assert_eq!(m.layer_notify, None);
        m.apply(ModelEvent::Hid(HidEvent::LayerNotifySupported(false)));
        assert_eq!(m.layer_notify, Some(false));
    }

    #[test]
    fn frame_marks_held_layer_key_and_recolors() {
        use crate::legend::KeyKind;
        use crate::legend::chars::{EN, FakeCharSource};
        let mut m = connected_model();
        m.apply(ModelEvent::Language(EN));
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState {
            active: 1,
            mask: 0b10,
            ..Default::default()
        })));
        m.keymap.layer_names = vec!["Base".into(), "Nav".into()];
        let f = m.frame(&FakeCharSource::default());
        assert_eq!(f.keys.len(), 36);
        assert!(f.keys.iter().all(|k| k.color_layer == 1));
        assert_eq!(
            f.keys[31].legend.kind,
            KeyKind::HeldLayer,
            "left thumb MO(1) is the held key"
        );
        assert_eq!(
            f.keys[34].legend.kind,
            KeyKind::LayerKey(3),
            "right thumb leads to layer 3 from Nav"
        );
        assert_eq!(f.status, "OP36 · EN · Nav · USB");
        assert_eq!(f.layer_color, 1);
        assert!(f.connected);
    }

    #[test]
    fn frame_status_marks_offline() {
        use crate::legend::chars::FakeCharSource;
        let m = Model::new(Settings::default(), None);
        let f = m.frame(&FakeCharSource::default());
        assert!(f.status.ends_with("· offline"), "{}", f.status);
        assert!(!f.connected);
    }

    #[test]
    fn frame_link_is_the_transport_while_connected() {
        use crate::legend::chars::FakeCharSource;
        let mut m = connected_model();
        assert_eq!(
            m.frame(&FakeCharSource::default()).link,
            Some(crate::vial::transport::Transport::Usb)
        );
        m.apply(ModelEvent::Hid(HidEvent::Disconnected));
        assert_eq!(m.frame(&FakeCharSource::default()).link, None);
        m.apply(ModelEvent::Hid(HidEvent::Connected(
            crate::vial::transport::Transport::Ble,
        )));
        assert_eq!(
            m.frame(&FakeCharSource::default()).link,
            Some(crate::vial::transport::Transport::Ble)
        );
    }

    #[test]
    fn switching_hides_offline_until_reconnect() {
        use crate::legend::chars::FakeCharSource;
        use crate::vial::protocol::Side;
        let mut m = connected_model();
        m.apply_at(ModelEvent::Hid(HidEvent::Switching(Side::Right)), 1_000);
        m.apply_at(ModelEvent::Hid(HidEvent::Disconnected), 1_500);
        assert!(m.is_switching(4_000));
        assert!(
            !m.status_line_at(4_000).contains("offline"),
            "{}",
            m.status_line_at(4_000)
        );
        assert!(
            m.status_line_at(4_000).contains("switching"),
            "{}",
            m.status_line_at(4_000)
        );
        assert!(m.frame_at(&FakeCharSource::default(), 4_000).switching);
        m.apply_at(
            ModelEvent::Hid(HidEvent::Connected(crate::vial::transport::Transport::Usb)),
            4_500,
        );
        assert!(!m.is_switching(4_600));
        let f = m.frame_at(&FakeCharSource::default(), 4_600);
        assert_eq!(f.link_side, Side::Right);
        assert!(!f.switching);
    }

    #[test]
    fn switching_shows_as_soon_as_the_handoff_arrives() {
        // I2: the app holds Disconnected back for the offline grace; the overlay must not wait for it.
        use crate::legend::chars::FakeCharSource;
        use crate::vial::protocol::Side;
        let mut m = connected_model();
        m.apply_at(ModelEvent::Hid(HidEvent::Switching(Side::Right)), 1_000);
        let f = m.frame_at(&FakeCharSource::default(), 1_200);
        assert!(
            f.switching,
            "still 'connected' from the app's point of view, but switching"
        );
        assert_eq!(f.link, None, "no link glyph while switching");
        assert!(
            m.status_line_at(1_200).contains("switching"),
            "{}",
            m.status_line_at(1_200)
        );
        // Reconnect after the timeout: the announced side is stale and must not be applied.
        let mut m = connected_model();
        m.apply_at(ModelEvent::Hid(HidEvent::Switching(Side::Right)), 0);
        m.apply_at(ModelEvent::Hid(HidEvent::Disconnected), 100);
        m.apply_at(
            ModelEvent::Hid(HidEvent::Connected(crate::vial::transport::Transport::Usb)),
            20_000,
        );
        assert_eq!(
            m.frame_at(&FakeCharSource::default(), 20_000).link_side,
            Side::Left
        );
    }

    #[test]
    fn firmware_builds_are_kept_per_half_until_disconnect() {
        use crate::vial::protocol::Side;
        let mut m = connected_model();
        assert_eq!(m.firmware_line(), "left unknown, right unknown");
        m.apply(ModelEvent::Hid(HidEvent::Firmware(
            Side::Left,
            "v0.2.0-22-gabc".into(),
        )));
        m.apply(ModelEvent::Hid(HidEvent::Firmware(
            Side::Right,
            String::new(),
        )));
        assert_eq!(m.firmware_line(), "left v0.2.0-22-gabc, right unknown");
        m.apply(ModelEvent::Hid(HidEvent::Firmware(
            Side::Right,
            "v0.2.0-22-gabc".into(),
        )));
        assert_eq!(
            m.firmware_line(),
            "left v0.2.0-22-gabc, right v0.2.0-22-gabc"
        );
        m.apply(ModelEvent::Hid(HidEvent::Disconnected));
        assert_eq!(m.firmware_line(), "left unknown, right unknown");
    }

    #[test]
    fn switching_times_out_to_offline() {
        use crate::vial::protocol::Side;
        let mut m = connected_model();
        m.apply_at(ModelEvent::Hid(HidEvent::Switching(Side::Right)), 0);
        m.apply_at(ModelEvent::Hid(HidEvent::Disconnected), 100);
        assert!(m.is_switching(14_999));
        assert!(!m.is_switching(15_000));
        assert!(
            m.status_line_at(15_000).ends_with("offline"),
            "{}",
            m.status_line_at(15_000)
        );
    }

    #[test]
    fn an_offline_half_is_reported_in_the_frame() {
        use crate::legend::chars::FakeCharSource;
        use crate::vial::protocol::Side;
        let mut m = connected_model();
        let linked = LayerState {
            peer_linked: true,
            peer_link_known: true,
            ..Default::default()
        };
        m.apply(ModelEvent::Hid(HidEvent::Layer(linked)));
        assert_eq!(m.frame(&FakeCharSource::default()).offline_half, None);
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState {
            peer_linked: false,
            ..linked
        })));
        assert_eq!(
            m.frame(&FakeCharSource::default()).offline_half,
            Some(Side::Right),
            "the master half is never the offline one"
        );
        // Right half as master: the left is the one missing.
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState {
            central_is_right: true,
            peer_linked: false,
            peer_link_known: true,
            ..Default::default()
        })));
        assert_eq!(
            m.frame(&FakeCharSource::default()).offline_half,
            Some(Side::Left)
        );
        // Firmware without the feature, and an offline keyboard, draw both halves normally.
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState::default())));
        assert_eq!(m.frame(&FakeCharSource::default()).offline_half, None);
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState {
            peer_linked: false,
            ..linked
        })));
        m.apply(ModelEvent::Hid(HidEvent::Disconnected));
        assert_eq!(m.frame(&FakeCharSource::default()).offline_half, None);
    }

    #[test]
    fn layer_state_bit_moves_the_link_side() {
        use crate::legend::chars::FakeCharSource;
        use crate::vial::protocol::Side;
        let mut m = connected_model();
        assert_eq!(m.frame(&FakeCharSource::default()).link_side, Side::Left);
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState {
            central_is_right: true,
            ..Default::default()
        })));
        assert_eq!(m.frame(&FakeCharSource::default()).link_side, Side::Right);
    }

    #[test]
    fn charging_flags_reach_the_frame_only_while_connected() {
        use crate::legend::chars::FakeCharSource;
        let mut m = connected_model();
        m.apply(ModelEvent::Hid(HidEvent::Battery(
            crate::vial::protocol::BatteryHalves {
                left: Some(85),
                right: Some(72),
                left_charging: true,
                right_charging: false,
            },
        )));
        let f = m.frame(&FakeCharSource::default());
        assert_eq!(
            (f.battery_left_charging, f.battery_right_charging),
            (true, false)
        );
        m.apply(ModelEvent::Hid(HidEvent::Disconnected));
        let f = m.frame(&FakeCharSource::default());
        assert_eq!(
            (f.battery_left_charging, f.battery_right_charging),
            (false, false)
        );
    }

    #[test]
    fn battery_levels_go_to_the_frame_not_the_status() {
        use crate::legend::chars::FakeCharSource;
        let mut m = connected_model();
        m.apply(ModelEvent::Language(crate::legend::chars::EN));
        m.apply(ModelEvent::Hid(HidEvent::Battery(
            crate::vial::protocol::BatteryHalves {
                left: Some(85),
                right: Some(72),
                left_charging: false,
                right_charging: false,
            },
        )));
        assert!(!m.status_line().contains('%'), "{}", m.status_line());
        let f = m.frame(&FakeCharSource::default());
        assert_eq!((f.battery_left, f.battery_right), (Some(85), Some(72)));
        m.apply(ModelEvent::Hid(HidEvent::Disconnected));
        let f = m.frame(&FakeCharSource::default());
        assert_eq!(
            (f.battery_left, f.battery_right),
            (None, None),
            "no battery while offline"
        );
    }

    #[test]
    fn status_line_offline_and_without_layer_notify() {
        let mut m = Model::new(Settings::default(), None);
        m.apply(ModelEvent::Language(crate::legend::chars::RU));
        assert_eq!(m.status_line(), "OP36 · RU · Layer 0 · offline");
        m.apply(ModelEvent::Hid(HidEvent::Connected(
            crate::vial::transport::Transport::Usb,
        )));
        m.apply(ModelEvent::Hid(HidEvent::LayerNotifySupported(false)));
        assert!(
            m.status_line().ends_with("· USB · no layer data"),
            "{}",
            m.status_line()
        );
    }

    #[test]
    fn while_held_mode_visibility() {
        let mut m = Model::new(Settings::default(), None);
        m.settings.display_mode = crate::settings::DisplayMode::WhileHeld;
        assert!(!m.wants_visible(), "nothing held: hidden");
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState {
            active: 1,
            mask: 0b10,
            ..Default::default()
        })));
        assert!(m.wants_visible(), "layer held: shown");
        m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState::default())));
        m.apply(ModelEvent::Mods(Modifiers {
            ctrl: true,
            ..Default::default()
        }));
        assert!(m.wants_visible(), "modifier held: shown");
        m.apply(ModelEvent::Mods(Modifiers::default()));
        m.find = Some(FindResult::default());
        assert!(m.wants_visible(), "find result: shown");
        m.apply(ModelEvent::FindExpired);
        assert!(!m.wants_visible());
        m.settings.display_mode = crate::settings::DisplayMode::Always;
        assert!(m.wants_visible(), "always mode ignores state");
    }

    #[test]
    fn modifiers_and_language_update() {
        let mut m = connected_model();
        m.apply(ModelEvent::Mods(Modifiers {
            shift: true,
            ..Default::default()
        }));
        assert!(m.mods.shift);
        m.apply(ModelEvent::Language(0x04190419));
        assert_eq!(m.hkl, 0x04190419);
    }
}
