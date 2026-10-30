//! Pure encode/decode of the 32-byte Vial packets the helper uses.

use crate::keymap::{Action, HidKeyCode, KeyAction, KeyCode, ModifierCombination};

pub type Packet = [u8; 32];

// Standard VIA command ids.
pub const CMD_CUSTOM_SET_VALUE: u8 = 0x07;
pub const CMD_CUSTOM_GET_VALUE: u8 = 0x08;
pub const CMD_KEYMAP_GET_LAYER_COUNT: u8 = 0x11;
pub const CMD_KEYMAP_GET_BUFFER: u8 = 0x12;
/// Vendor namespace and the native-key-action sub-ids (Entropy's lossless key encoding).
pub const ERGOHAVEN_NAMESPACE: u8 = 0xE8;
pub const ERGOHAVEN_NATIVE_KEY_ACTION: u8 = 0x03;
pub const ERGOHAVEN_NEXT_NATIVE_KEY_ACTION: u8 = 0x04;
pub const NATIVE_KEY_ACTION_VERSION: u8 = 0x01;
/// Our namespace and sub-id (firmware feature `layer_notify`).
pub const LAYER_NOTIFY_NAMESPACE: u8 = 0xE9;
pub const LAYER_NOTIFY_SUB_STATE: u8 = 0x01;

fn packet(prefix: &[u8]) -> Packet {
    let mut p = [0u8; 32];
    p[..prefix.len()].copy_from_slice(prefix);
    p
}

pub fn req_layer_count() -> Packet {
    packet(&[CMD_KEYMAP_GET_LAYER_COUNT])
}

/// `size` bytes (≤ 28) of VIA keycodes starting at byte `offset` of the flat keymap.
pub fn req_keymap_buffer(offset: u16, size: u8) -> Packet {
    let [hi, lo] = offset.to_be_bytes();
    packet(&[CMD_KEYMAP_GET_BUFFER, hi, lo, size])
}

pub fn parse_keymap_buffer(reply: &Packet, size: u8) -> Vec<u16> {
    reply[4..4 + (size as usize).min(28)]
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

pub fn req_native_action(layer: u8, row: u8, col: u8) -> Packet {
    packet(&[
        CMD_CUSTOM_GET_VALUE,
        ERGOHAVEN_NAMESPACE,
        ERGOHAVEN_NATIVE_KEY_ACTION,
        NATIVE_KEY_ACTION_VERSION,
        layer,
        row,
        col,
    ])
}

/// Ask for the next position at or after flat index `start` whose action VIA cannot express.
pub fn req_next_native_action(start: u16) -> Packet {
    let [lo, hi] = start.to_le_bytes();
    packet(&[
        CMD_CUSTOM_GET_VALUE,
        ERGOHAVEN_NAMESPACE,
        ERGOHAVEN_NEXT_NATIVE_KEY_ACTION,
        NATIVE_KEY_ACTION_VERSION,
        lo,
        hi,
    ])
}

pub fn req_layer_subscribe(ttl_secs: u8) -> Packet {
    packet(&[
        CMD_CUSTOM_SET_VALUE,
        LAYER_NOTIFY_NAMESPACE,
        LAYER_NOTIFY_SUB_STATE,
        ttl_secs,
    ])
}

pub fn req_layer_state() -> Packet {
    packet(&[
        CMD_CUSTOM_GET_VALUE,
        LAYER_NOTIFY_NAMESPACE,
        LAYER_NOTIFY_SUB_STATE,
    ])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerState {
    pub active: u8,
    pub default: u8,
    pub mask: u16,
    /// Firmware layout id, bit 7 stripped.
    pub fw_layout: u8,
    /// Bit 7 of the layout byte: the right half is the central (dynamic master). `false` on
    /// firmware that predates the feature, i.e. the left half.
    pub central_is_right: bool,
    /// Bit 6: the other half is linked to the master. Firmware that predates the feature leaves
    /// it clear, so `unknown` tells the two apart and keeps both halves drawn normally.
    pub peer_linked: bool,
    /// The firmware reports the link state (bit 6 or bit 7 set). `false` = older firmware or no
    /// state received yet, and the helper then draws both halves normally.
    pub peer_link_known: bool,
}

/// Which half (dynamic master).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Sub-id of the handoff notification `E9 02 side` (0 = left becomes master, 1 = right).
pub const LAYER_NOTIFY_SUB_HANDOFF: u8 = 0x02;
/// Sub-id of the firmware-build query: request `08 E9 03 side`, reply `E9 03 side len text…`.
pub const LAYER_NOTIFY_SUB_FIRMWARE: u8 = 0x03;
const FW_LAYOUT_CENTRAL_RIGHT: u8 = 0x80;
const FW_LAYOUT_PEER_LINKED: u8 = 0x40;
/// Firmware that reports the link state always sets one of the two status bits for the master
/// half itself, so "both clear" means the feature is missing.
const FW_LAYOUT_STATUS_BITS: u8 = FW_LAYOUT_CENTRAL_RIGHT | FW_LAYOUT_PEER_LINKED;

/// `E9 01 active default mask_lo mask_hi fw_layout` — notification or reply.
pub fn parse_layer_state(pkt: &Packet) -> Option<LayerState> {
    (pkt[0] == LAYER_NOTIFY_NAMESPACE && pkt[1] == LAYER_NOTIFY_SUB_STATE).then(|| LayerState {
        active: pkt[2],
        default: pkt[3],
        mask: u16::from_le_bytes([pkt[4], pkt[5]]),
        fw_layout: pkt[6] & !FW_LAYOUT_STATUS_BITS,
        central_is_right: pkt[6] & FW_LAYOUT_CENTRAL_RIGHT != 0,
        peer_linked: pkt[6] & FW_LAYOUT_PEER_LINKED != 0,
        peer_link_known: pkt[6] & FW_LAYOUT_STATUS_BITS != 0,
    })
}

/// Ask the keyboard for one half's firmware build id (`git describe` at build time).
pub fn req_firmware_build(side: Side) -> Packet {
    packet(&[
        CMD_CUSTOM_GET_VALUE,
        LAYER_NOTIFY_NAMESPACE,
        LAYER_NOTIFY_SUB_FIRMWARE,
        (side == Side::Right) as u8,
    ])
}

/// The firmware carries at most this many bytes of `git describe` (`SplitMessage::FirmwareBuild`).
pub const FIRMWARE_BUILD_MAX: usize = 24;

/// `E9 03 side len text…`; an empty text means the keyboard does not know that half's build yet.
pub fn parse_firmware_build(pkt: &Packet) -> Option<(Side, String)> {
    (pkt[0] == LAYER_NOTIFY_NAMESPACE && pkt[1] == LAYER_NOTIFY_SUB_FIRMWARE).then(|| {
        let side = if pkt[2] == 1 { Side::Right } else { Side::Left };
        let n = (pkt[3] as usize).min(FIRMWARE_BUILD_MAX);
        (side, String::from_utf8_lossy(&pkt[4..4 + n]).into_owned())
    })
}

/// `E9 02 side`: the halves are swapping roles; `side` becomes the master.
pub fn parse_handoff(pkt: &Packet) -> Option<Side> {
    (pkt[0] == LAYER_NOTIFY_NAMESPACE && pkt[1] == LAYER_NOTIFY_SUB_HANDOFF)
        .then(|| if pkt[2] == 1 { Side::Right } else { Side::Left })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStatus {
    UnsupportedVersion,
    InvalidPosition,
    InvalidPayload,
    Unknown(u8),
}

fn native_status(code: u8) -> NativeStatus {
    match code {
        2 => NativeStatus::UnsupportedVersion,
        3 => NativeStatus::InvalidPosition,
        4 => NativeStatus::InvalidPayload,
        other => NativeStatus::Unknown(other),
    }
}

fn postcard_payload(reply: &Packet, len_at: usize) -> Result<KeyAction, NativeStatus> {
    let len = reply[len_at] as usize;
    let start = len_at + 1;
    if len == 0 || start + len > reply.len() {
        return Err(NativeStatus::InvalidPayload);
    }
    postcard::from_bytes(&reply[start..start + len]).map_err(|_| NativeStatus::InvalidPayload)
}

/// Reply to [`req_native_action`]: status at byte 4, payload length at byte 5, postcard from byte 6.
pub fn parse_native_action(reply: &Packet) -> Result<KeyAction, NativeStatus> {
    match reply[4] {
        0 => postcard_payload(reply, 5),
        code => Err(native_status(code)),
    }
}

/// Reply to [`req_next_native_action`]: `Ok(None)` at END, else `(flat index, action)`.
pub fn parse_next_native_action(reply: &Packet) -> Result<Option<(u16, KeyAction)>, NativeStatus> {
    match reply[4] {
        0 => {
            let index = u16::from_le_bytes([reply[5], reply[6]]);
            postcard_payload(reply, 7).map(|a| Some((index, a)))
        }
        1 => Ok(None),
        code => Err(native_status(code)),
    }
}

pub const CMD_VIAL: u8 = 0xFE;
pub const VIAL_DYNAMIC_ENTRY_OP: u8 = 0x0D;
pub const VIAL_DYNAMIC_COMBO_SET: u8 = 0x04;
pub const COMBO_MAX_LENGTH: usize = 4;

/// Write one key action natively: `07 E8 03 01 layer row col len <postcard>`. `None` if it does not fit.
pub fn req_native_action_set(layer: u8, row: u8, col: u8, action: &KeyAction) -> Option<Packet> {
    let mut p = packet(&[
        CMD_CUSTOM_SET_VALUE,
        ERGOHAVEN_NAMESPACE,
        ERGOHAVEN_NATIVE_KEY_ACTION,
        NATIVE_KEY_ACTION_VERSION,
        layer,
        row,
        col,
    ]);
    let len = postcard::to_slice(action, &mut p[8..]).ok()?.len();
    p[7] = len as u8;
    Some(p)
}

/// Vial combo write: `FE 0D 04 idx k0 k1 k2 k3 out` with little-endian VIA keycodes (unused keys 0).
pub fn req_combo_set(index: u8, keys: &[u16], output: u16) -> Packet {
    let mut p = packet(&[
        CMD_VIAL,
        VIAL_DYNAMIC_ENTRY_OP,
        VIAL_DYNAMIC_COMBO_SET,
        index,
    ]);
    for (i, kc) in keys.iter().take(COMBO_MAX_LENGTH).enumerate() {
        p[4 + i * 2..6 + i * 2].copy_from_slice(&kc.to_le_bytes());
    }
    p[4 + COMBO_MAX_LENGTH * 2..6 + COMBO_MAX_LENGTH * 2].copy_from_slice(&output.to_le_bytes());
    p
}

/// The VIA keycode for the simple actions the combo protocol can carry.
pub fn action_to_via_keycode(action: &KeyAction) -> Option<u16> {
    match action {
        KeyAction::No => Some(0),
        KeyAction::Transparent => Some(1),
        KeyAction::Single(Action::Key(KeyCode::Hid(h))) => Some(*h as u16),
        KeyAction::Single(Action::KeyWithModifier(h, m)) => {
            Some(((m.into_packed_bits() as u16) << 8) | *h as u16)
        }
        KeyAction::Single(Action::LayerOn(l)) => Some(0x5220 | *l as u16),
        _ => None,
    }
}

pub const VIAL_GET_BEHAVIOR_SETTING: u8 = 0x0A;
pub const VIAL_SET_BEHAVIOR_SETTING: u8 = 0x0B;
/// Vial behaviour-setting ids used for the tap-hold rules.
pub const SETTING_MORSE_TIMEOUT: u16 = 0x07;
pub const SETTING_PERMISSIVE_HOLD: u16 = 0x16;
pub const SETTING_UNILATERAL_TAP: u16 = 0x1A;
pub const SETTING_PRIOR_IDLE_TIME: u16 = 0x1B;

/// `FE 0B key_lo key_hi value_lo value_hi`; the reply starts with the return code (0 = ok).
pub fn req_set_behavior_setting(key: u16, value: u16) -> Packet {
    let [kl, kh] = key.to_le_bytes();
    let [vl, vh] = value.to_le_bytes();
    packet(&[CMD_VIAL, VIAL_SET_BEHAVIOR_SETTING, kl, kh, vl, vh])
}

pub fn req_get_behavior_setting(key: u16) -> Packet {
    let [kl, kh] = key.to_le_bytes();
    packet(&[CMD_VIAL, VIAL_GET_BEHAVIOR_SETTING, kl, kh])
}

pub const ERGOHAVEN_BATTERY_HALVES: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatteryHalves {
    pub left: Option<u8>,
    pub right: Option<u8>,
    /// The half is on USB power (flags bits 2/3; absent in firmware before 2026-10-30).
    pub left_charging: bool,
    pub right_charging: bool,
}

/// Vendor query for both halves' battery levels.
pub fn req_battery_halves() -> Packet {
    packet(&[
        CMD_CUSTOM_GET_VALUE,
        ERGOHAVEN_NAMESPACE,
        ERGOHAVEN_BATTERY_HALVES,
    ])
}

/// Reply: byte 3 version, byte 4 flags (bit 0 left known, bit 1 right known), bytes 5/6 percent.
pub fn parse_battery_halves(reply: &Packet) -> Option<BatteryHalves> {
    if reply[..3]
        != [
            CMD_CUSTOM_GET_VALUE,
            ERGOHAVEN_NAMESPACE,
            ERGOHAVEN_BATTERY_HALVES,
        ]
    {
        return None;
    }
    let flags = reply[4];
    Some(BatteryHalves {
        left: (flags & 1 != 0).then_some(reply[5].min(100)),
        right: (flags & 2 != 0).then_some(reply[6].min(100)),
        left_charging: flags & 4 != 0,
        right_charging: flags & 8 != 0,
    })
}

/// A reply echoes the command byte; custom get/set replies also echo namespace and sub-id.
pub fn is_reply_for(request: &Packet, reply: &Packet) -> bool {
    if reply[0] != request[0] {
        return false;
    }
    !matches!(request[0], CMD_CUSTOM_GET_VALUE | CMD_CUSTOM_SET_VALUE)
        || reply[1..3] == request[1..3]
}

fn hid(code: u16) -> HidKeyCode {
    let code = code as u8;
    HidKeyCode::from_repr(code).unwrap_or(HidKeyCode::No)
}

/// Decode the VIA keycode ranges the firmware emits for simple actions (mirrors the firmware's
/// `from_via_keycode`). `None` means "VIA cannot express this; read the native action instead".
pub fn via_keycode_to_action(kc: u16) -> Option<KeyAction> {
    let mods = |bits: u16| ModifierCombination::from_packed_bits(bits as u8);
    let layer = |bits: u16| bits as u8;
    Some(match kc {
        0x0000 => KeyAction::No,
        0x0001 => KeyAction::Transparent,
        0x0002..=0x00FF => KeyAction::Single(Action::Key(KeyCode::Hid(hid(kc)))),
        0x0100..=0x1FFF => {
            KeyAction::Single(Action::KeyWithModifier(hid(kc & 0xFF), mods(kc >> 8)))
        }
        0x2000..=0x3FFF => KeyAction::TapHold(
            Action::Key(KeyCode::Hid(hid(kc & 0xFF))),
            Action::Modifier(mods((kc >> 8) & 0b11111)),
            Default::default(),
        ),
        0x4000..=0x4FFF => KeyAction::TapHold(
            Action::Key(KeyCode::Hid(hid(kc & 0xFF))),
            Action::LayerOn(layer((kc >> 8) & 0xF)),
            Default::default(),
        ),
        0x5000..=0x51FF => KeyAction::Single(Action::LayerOnWithModifier(
            layer((kc >> 5) & 0xF),
            mods(kc & 0b11111),
        )),
        0x5200..=0x521F => KeyAction::Single(Action::LayerToggleOnly(layer(kc & 0xF))),
        0x5220..=0x523F => KeyAction::Single(Action::LayerOn(layer(kc & 0xF))),
        0x5240..=0x525F => KeyAction::Single(Action::DefaultLayer(layer(kc & 0xF))),
        0x5260..=0x527F => KeyAction::Single(Action::LayerToggle(layer(kc & 0xF))),
        0x5280..=0x529F => KeyAction::Single(Action::OneShotLayer(layer(kc & 0xF))),
        0x52A0..=0x52BF => KeyAction::Single(Action::OneShotModifier(mods(kc & 0x1F))),
        0x52E0..=0x52FF => KeyAction::Single(Action::PersistentDefaultLayer(layer(kc & 0xF))),
        0x7700..=0x771F => KeyAction::Single(Action::TriggerMacro(layer(kc & 0x1F))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Action, HidKeyCode, KeyAction, KeyCode, ModifierCombination};

    fn reply(prefix: &[u8]) -> Packet {
        let mut p = [0u8; 32];
        p[..prefix.len()].copy_from_slice(prefix);
        p
    }

    #[test]
    fn keymap_buffer_request_is_big_endian() {
        assert_eq!(&req_keymap_buffer(0x0102, 28)[..4], &[0x12, 0x01, 0x02, 28]);
    }

    #[test]
    fn parse_keymap_buffer_reads_big_endian_u16s() {
        let mut r = [0u8; 32];
        r[4..8].copy_from_slice(&[0x00, 0x04, 0x52, 0x21]);
        assert_eq!(parse_keymap_buffer(&r, 4), vec![0x0004, 0x5221]);
    }

    #[test]
    fn parse_layer_state_rejects_other_packets() {
        assert!(parse_layer_state(&[0x08; 32]).is_none());
    }

    #[test]
    fn parse_layer_state_reads_fields() {
        let p = reply(&[0xE9, 1, 3, 0, 0x0E, 0, 1]);
        assert_eq!(
            parse_layer_state(&p),
            Some(LayerState {
                active: 3,
                default: 0,
                mask: 0x0E,
                fw_layout: 1,
                central_is_right: false,
                peer_linked: false,
                peer_link_known: false,
            })
        );
    }

    #[test]
    fn layer_requests_match_firmware_protocol() {
        assert_eq!(&req_layer_subscribe(60)[..4], &[0x07, 0xE9, 0x01, 60]);
        assert_eq!(&req_layer_state()[..3], &[0x08, 0xE9, 0x01]);
        assert_eq!(
            &req_native_action(2, 1, 4)[..7],
            &[0x08, 0xE8, 0x03, 0x01, 2, 1, 4]
        );
        assert_eq!(
            &req_next_native_action(0x0105)[..6],
            &[0x08, 0xE8, 0x04, 0x01, 0x05, 0x01]
        );
    }

    #[test]
    fn via_basic_keycodes_decode() {
        assert_eq!(via_keycode_to_action(0x0000), Some(KeyAction::No));
        assert_eq!(via_keycode_to_action(0x0001), Some(KeyAction::Transparent));
        assert_eq!(
            via_keycode_to_action(0x0014),
            Some(KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Q))))
        );
    }

    #[test]
    fn via_mod_tap_decodes_to_tap_hold() {
        // MT(LGui, A): packed modifier bit 3 = gui → 0x2000 | 0x0800 | 0x04
        match via_keycode_to_action(0x2804).unwrap() {
            KeyAction::TapHold(
                Action::Key(KeyCode::Hid(HidKeyCode::A)),
                Action::Modifier(m),
                _,
            ) => {
                assert!(m.left_gui() && !m.left_shift())
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn via_shifted_key_decodes_to_key_with_modifier() {
        assert_eq!(
            via_keycode_to_action(0x0220),
            Some(KeyAction::Single(Action::KeyWithModifier(
                HidKeyCode::Kc3,
                ModifierCombination::from_packed_bits(0b00010)
            )))
        );
    }

    #[test]
    fn via_layer_keys_decode() {
        assert_eq!(
            via_keycode_to_action(0x5221),
            Some(KeyAction::Single(Action::LayerOn(1)))
        );
        assert_eq!(
            via_keycode_to_action(0x5262),
            Some(KeyAction::Single(Action::LayerToggle(2)))
        );
        assert_eq!(
            via_keycode_to_action(0x4104),
            Some(KeyAction::TapHold(
                Action::Key(KeyCode::Hid(HidKeyCode::A)),
                Action::LayerOn(1),
                Default::default()
            ))
        );
    }

    #[test]
    fn via_unknown_ranges_are_none() {
        assert_eq!(via_keycode_to_action(0x7C77), None); // tri-layer: the native read decides
        assert_eq!(via_keycode_to_action(0x5700), None); // morse
    }

    #[test]
    fn parse_native_action_decodes_postcard_payload() {
        let action = KeyAction::Single(Action::KeyWithModifier(
            HidKeyCode::Kc9,
            ModifierCombination::from_packed_bits(0b00010),
        ));
        let mut r = reply(&[0x08, 0xE8, 0x03, 0x01, 0]);
        let n = postcard::to_slice(&action, &mut r[6..]).unwrap().len();
        r[5] = n as u8;
        assert_eq!(parse_native_action(&r), Ok(action));
        r[4] = 3;
        assert_eq!(parse_native_action(&r), Err(NativeStatus::InvalidPosition));
    }

    #[test]
    fn parse_next_native_action_decodes_index_and_end() {
        let action = KeyAction::Morse(2);
        let mut r = reply(&[0x08, 0xE8, 0x04, 0x01, 0, 0x23, 0x01]);
        let n = postcard::to_slice(&action, &mut r[8..]).unwrap().len();
        r[7] = n as u8;
        assert_eq!(parse_next_native_action(&r), Ok(Some((0x0123, action))));
        r[4] = 1; // END
        assert_eq!(parse_next_native_action(&r), Ok(None));
    }

    #[test]
    fn native_action_set_request_carries_postcard_payload() {
        let action = KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::LeftBracket)));
        let p = req_native_action_set(2, 3, 1, &action).unwrap();
        assert_eq!(&p[..7], &[0x08 - 1, 0xE8, 0x03, 0x01, 2, 3, 1]);
        let len = p[7] as usize;
        assert_eq!(
            postcard::from_bytes::<KeyAction>(&p[8..8 + len]).unwrap(),
            action
        );
    }

    #[test]
    fn combo_set_request_layout() {
        let p = req_combo_set(1, &[0x0018, 0x000C], 0x002F);
        assert_eq!(&p[..4], &[0xFE, 0x0D, 0x04, 1]);
        assert_eq!(&p[4..12], &[0x18, 0, 0x0C, 0, 0, 0, 0, 0]);
        assert_eq!(&p[12..14], &[0x2F, 0]);
        assert_eq!(
            action_to_via_keycode(&KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::U)))),
            Some(0x18)
        );
    }

    #[test]
    fn behavior_setting_requests() {
        assert_eq!(
            &req_set_behavior_setting(SETTING_PRIOR_IDLE_TIME, 150)[..6],
            &[0xFE, 0x0B, 0x1B, 0x00, 150, 0]
        );
        assert_eq!(
            &req_get_behavior_setting(SETTING_PERMISSIVE_HOLD)[..4],
            &[0xFE, 0x0A, 0x16, 0x00]
        );
    }

    #[test]
    fn battery_halves_request_and_parse() {
        assert_eq!(&req_battery_halves()[..3], &[0x08, 0xE8, 0x01]);
        let mut r = reply(&[0x08, 0xE8, 0x01, 0x01, 0b11, 85, 72]);
        assert_eq!(
            parse_battery_halves(&r),
            Some(BatteryHalves {
                left: Some(85),
                right: Some(72),
                left_charging: false,
                right_charging: false
            })
        );
        r[4] = 0b01;
        assert_eq!(
            parse_battery_halves(&r),
            Some(BatteryHalves {
                left: Some(85),
                right: None,
                left_charging: false,
                right_charging: false
            })
        );
        assert_eq!(parse_battery_halves(&reply(&[0x08, 0xE8, 0x02])), None);
    }

    #[test]
    fn battery_reply_carries_charging_bits() {
        let r = reply(&[0x08, 0xE8, 0x01, 1, 0b0000_1111, 85, 40]);
        let b = parse_battery_halves(&r).unwrap();
        assert_eq!(
            (b.left, b.right, b.left_charging, b.right_charging),
            (Some(85), Some(40), true, true)
        );
        let r = reply(&[0x08, 0xE8, 0x01, 1, 0b0000_1010, 85, 40]);
        let b = parse_battery_halves(&r).unwrap();
        assert_eq!(
            (b.left, b.right, b.left_charging, b.right_charging),
            (None, Some(40), false, true)
        );
    }

    #[test]
    fn old_firmware_reply_has_no_charging_or_right_central() {
        let b = parse_battery_halves(&reply(&[0x08, 0xE8, 0x01, 1, 0b11, 85, 40])).unwrap();
        assert!(!b.left_charging && !b.right_charging);
        let st = parse_layer_state(&reply(&[0xE9, 0x01, 2, 0, 0x04, 0x00, 1])).unwrap();
        assert!(!st.central_is_right);
        assert!(
            !st.peer_link_known,
            "old firmware says nothing about the other half"
        );
        assert_eq!(st.fw_layout, 1);
    }

    #[test]
    fn layer_state_bit7_means_right_half_is_central() {
        let st = parse_layer_state(&reply(&[0xE9, 0x01, 0, 0, 0x01, 0x00, 0x81])).unwrap();
        assert!(st.central_is_right);
        assert_eq!(
            st.fw_layout, 1,
            "the status bits are stripped from the layout byte"
        );
    }

    #[test]
    fn layer_state_bit6_reports_the_other_half() {
        let linked = parse_layer_state(&reply(&[0xE9, 0x01, 0, 0, 0x01, 0x00, 0x41])).unwrap();
        assert!(linked.peer_linked && linked.peer_link_known);
        assert_eq!(linked.fw_layout, 1);
        let offline = parse_layer_state(&reply(&[0xE9, 0x01, 0, 0, 0x01, 0x00, 0x81])).unwrap();
        assert!(
            !offline.peer_linked && offline.peer_link_known,
            "right is central, left offline"
        );
        let old = parse_layer_state(&reply(&[0xE9, 0x01, 0, 0, 0x01, 0x00, 0x01])).unwrap();
        assert!(!old.peer_link_known, "firmware without the feature");
        assert!(!old.peer_linked);
    }

    #[test]
    fn firmware_build_request_and_reply() {
        assert_eq!(
            &req_firmware_build(Side::Right)[..4],
            &[0x08, 0xE9, 0x03, 1]
        );
        assert_eq!(&req_firmware_build(Side::Left)[..4], &[0x08, 0xE9, 0x03, 0]);
        let mut r = reply(&[0xE9, 0x03, 1, 5]);
        r[4..9].copy_from_slice(b"v0.2.");
        assert_eq!(
            parse_firmware_build(&r),
            Some((Side::Right, "v0.2.".to_string()))
        );
        assert_eq!(
            parse_firmware_build(&reply(&[0xE9, 0x03, 0, 0])),
            Some((Side::Left, String::new()))
        );
        assert_eq!(parse_firmware_build(&reply(&[0xE9, 0x01, 0, 0])), None);
    }

    #[test]
    fn handoff_packet_names_the_new_master() {
        let mut p = reply(&[0xE9, 0x02, 1]);
        assert_eq!(parse_handoff(&p), Some(Side::Right));
        p[2] = 0;
        assert_eq!(parse_handoff(&p), Some(Side::Left));
        p[1] = 0x01;
        assert_eq!(parse_handoff(&p), None);
    }

    #[test]
    fn reply_matching_rules() {
        assert!(is_reply_for(&req_layer_count(), &reply(&[0x11])));
        assert!(!is_reply_for(&req_layer_count(), &reply(&[0xE9])));
        let req = req_native_action(0, 0, 0);
        assert!(is_reply_for(&req, &reply(&[0x08, 0xE8, 0x03])));
        assert!(!is_reply_for(&req, &reply(&[0x08, 0xE8, 0x01])));
    }
}
