//! Read the whole keymap from the keyboard.

use std::time::Instant;

use super::protocol::*;
use super::transport::VialTransport;
use crate::keymap::{KeyAction, Keymap};


/// Send `req` and wait for its reply, discarding anything else (layer notifications, stale replies).
pub fn request(t: &mut dyn VialTransport, req: &Packet) -> Result<Packet, String> {
    request_collect(t, req, &mut Vec::new())
}

/// Like [`request`], but every other packet seen while waiting is pushed to `stray` so the caller
/// can still act on it (a layer notification must not be lost behind a battery query).
pub fn request_collect(t: &mut dyn VialTransport, req: &Packet, stray: &mut Vec<Packet>) -> Result<Packet, String> {
    t.write(req)?;
    let timeout = t.reply_timeout();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(pkt) = t.read(50)? {
            if is_reply_for(req, &pkt) {
                return Ok(pkt);
            }
            stray.push(pkt);
        }
    }
    Err(format!("no reply to command {:#04x} within {:?}", req[0], timeout))
}

/// Read one Vial behaviour setting; the reply starts with a return code instead of echoing the command.
pub fn request_vial_setting(t: &mut dyn VialTransport, key: u16) -> Result<Packet, String> {
    t.write(&req_get_behavior_setting(key))?;
    let deadline = Instant::now() + t.reply_timeout();
    while Instant::now() < deadline {
        if let Some(pkt) = t.read(50)? {
            if pkt[0] == 0 {
                return Ok(pkt);
            }
        }
    }
    Err(format!("no reply to behaviour setting {key:#04x}"))
}

/// Layer count, then the VIA keycode buffer for every layer, then the vendor's native-action scan
/// for every position VIA could not express.
pub fn read_keymap(t: &mut dyn VialTransport, rows: u8, cols: u8) -> Result<Keymap, String> {
    let layers = request(t, &req_layer_count())?[1];
    if layers == 0 {
        return Err("keyboard reports zero layers".into());
    }
    let mut keymap = Keymap::empty(rows, cols, layers);
    let total_bytes = keymap.actions.len() * 2;
    let mut keycodes = Vec::with_capacity(keymap.actions.len());
    let mut offset = 0usize;
    while offset < total_bytes {
        let size = (total_bytes - offset).min(28) as u8;
        let reply = request(t, &req_keymap_buffer(offset as u16, size))?;
        keycodes.extend(parse_keymap_buffer(&reply, size));
        offset += size as usize;
    }
    // Non-zero VIA codes the decoder does not know (e.g. 0x7C1C for a shifted mod-tap) are
    // fetched natively below; zero codes are either `No` or reported by the native scan.
    let mut unknown = Vec::new();
    for (i, (slot, kc)) in keymap.actions.iter_mut().zip(keycodes).enumerate() {
        match via_keycode_to_action(kc) {
            Some(action) => *slot = action,
            None => {
                *slot = KeyAction::No;
                unknown.push(i);
            }
        }
    }
    let mut start = 0u16;
    loop {
        let reply = request(t, &req_next_native_action(start))?;
        match parse_next_native_action(&reply) {
            Ok(Some((index, action))) => {
                if let Some(slot) = keymap.actions.get_mut(index as usize) {
                    *slot = action;
                }
                start = index.saturating_add(1);
                if start as usize >= keymap.actions.len() {
                    break;
                }
            }
            Ok(None) => break,
            Err(status) => {
                log::warn!("native key action scan stopped: {status:?}");
                break;
            }
        }
    }
    let per_layer = rows as usize * cols as usize;
    for i in unknown {
        let (layer, rem) = ((i / per_layer) as u8, i % per_layer);
        let (row, col) = ((rem / cols as usize) as u8, (rem % cols as usize) as u8);
        let reply = request(t, &req_native_action(layer, row, col))?;
        match parse_native_action(&reply) {
            Ok(action) => keymap.actions[i] = action,
            Err(status) => log::warn!("native read of layer {layer} ({row},{col}) failed: {status:?}"),
        }
    }
    Ok(keymap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Action, HidKeyCode, KeyAction, KeyCode};
    use crate::vial::protocol::Packet;
    use crate::vial::transport::FakeTransport;

    fn reply(prefix: &[u8]) -> Packet {
        let mut p = [0u8; 32];
        p[..prefix.len()].copy_from_slice(prefix);
        p
    }

    #[test]
    fn request_collect_keeps_the_layer_notification_that_arrived_first() {
        let mut t = FakeTransport::default();
        t.script.push_back(reply(&[0xE9, 0x01, 1, 0, 2, 0]));
        t.script.push_back(reply(&[0x08, 0xE8, 0x01, 3, 85, 72]));
        let mut stray = Vec::new();
        let got = request_collect(&mut t, &crate::vial::protocol::req_battery_halves(), &mut stray).unwrap();
        assert_eq!(got[1..3], [0xE8, 0x01]);
        assert_eq!(stray.len(), 1);
        assert_eq!(stray[0][0], 0xE9);
    }

    /// 3 layers × 1 row × 2 cols. flat 1 = Transparent; flat 2 = 0x7C77 (non-zero, unknown to the
    /// decoder → direct native read); flat 3 = 0x0000 but a Morse action (reported by the scan).
    fn scripted() -> FakeTransport {
        let mut t = FakeTransport::default();
        t.script.push_back(reply(&[0x11, 3])); // layer count
        let mut buf = reply(&[0x12, 0, 0, 12]); // 3*1*2 keycodes = 12 bytes, one chunk
        buf[4..16].copy_from_slice(&[0x00, 0x14, 0x00, 0x01, 0x7C, 0x77, 0x00, 0x00, 0, 0, 0, 0]);
        t.script.push_back(buf);
        let mut hit = reply(&[0x08, 0xE8, 0x04, 0x01, 0, 3, 0]); // scan hit at flat 3
        let n = postcard::to_slice(&KeyAction::Morse(2), &mut hit[8..]).unwrap().len();
        hit[7] = n as u8;
        t.script.push_back(hit);
        t.script.push_back(reply(&[0x08, 0xE8, 0x04, 0x01, 1])); // scan END
        let mut native = reply(&[0x08, 0xE8, 0x03, 0x01, 0]); // direct read of flat 2 = layer 1 (0,0)
        let n = postcard::to_slice(&KeyAction::Single(Action::TriLayerLower), &mut native[6..])
            .unwrap()
            .len();
        native[5] = n as u8;
        t.script.push_back(native);
        t
    }

    #[test]
    fn reader_assembles_layers_scan_hits_and_direct_native_reads() {
        let mut t = scripted();
        let km = read_keymap(&mut t, 1, 2).unwrap();
        assert_eq!((km.rows, km.cols, km.layers), (1, 2, 3));
        assert_eq!(km.actions[0], KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Q))));
        assert_eq!(km.actions[1], KeyAction::Transparent);
        assert_eq!(km.actions[2], KeyAction::Single(Action::TriLayerLower));
        assert_eq!(km.actions[3], KeyAction::Morse(2));
        assert_eq!(km.actions[4], KeyAction::No);
        assert_eq!(&t.sent[0][..1], &[0x11]);
        assert_eq!(&t.sent[1][..4], &[0x12, 0, 0, 12]);
        assert_eq!(&t.sent[2][..6], &[0x08, 0xE8, 0x04, 0x01, 0, 0]);
        assert_eq!(&t.sent[3][..6], &[0x08, 0xE8, 0x04, 0x01, 4, 0]); // resumes after the hit
        assert_eq!(&t.sent[4][..7], &[0x08, 0xE8, 0x03, 0x01, 1, 0, 0]); // direct read of layer 1 (0,0)
    }

    #[test]
    fn reader_reads_unknown_nonzero_keycodes_natively_when_scan_misses_them() {
        // The firmware maps some rich actions to a non-zero VIA code (e.g. 0x7C1C) that the
        // next-native scan does not report; the reader must fetch those positions directly.
        let mut t = FakeTransport::default();
        t.script.push_back(reply(&[0x11, 1]));
        let mut buf = reply(&[0x12, 0, 0, 4]);
        buf[4..8].copy_from_slice(&[0x00, 0x14, 0x7C, 0x1C]);
        t.script.push_back(buf);
        t.script.push_back(reply(&[0x08, 0xE8, 0x04, 0x01, 1])); // scan: END immediately
        let action = KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc9, crate::keymap::ModifierCombination::from_packed_bits(2)),
            Action::Modifier(crate::keymap::ModifierCombination::from_packed_bits(4)),
            Default::default(),
        );
        let mut native = reply(&[0x08, 0xE8, 0x03, 0x01, 0]);
        let n = postcard::to_slice(&action, &mut native[6..]).unwrap().len();
        native[5] = n as u8;
        t.script.push_back(native);
        let km = read_keymap(&mut t, 1, 2).unwrap();
        assert_eq!(km.actions[1], action);
        assert_eq!(&t.sent[3][..7], &[0x08, 0xE8, 0x03, 0x01, 0, 0, 1]);
    }

    #[test]
    fn reader_skips_layer_notifications_while_waiting() {
        let mut t = scripted();
        t.script.push_front(reply(&[0xE9, 0x01, 1, 0, 2, 0, 0]));
        assert!(read_keymap(&mut t, 1, 2).is_ok());
    }

    #[test]
    fn reader_times_out_without_reply() {
        let mut t = FakeTransport::default();
        assert!(read_keymap(&mut t, 1, 2).is_err());
    }
}
