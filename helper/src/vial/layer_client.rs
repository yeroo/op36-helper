//! Subscription state machine for the firmware's layer notifications (pure, time injected).

use super::protocol::{
    LayerState, Packet, Side, parse_firmware_build, parse_handoff, parse_layer_state, req_firmware_build, req_layer_state,
    req_layer_subscribe,
};

/// A layer-notify packet from the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerPacket {
    State(LayerState),
    /// The halves are swapping roles; the given side becomes the master.
    Handoff(Side),
    /// One half's firmware build id (empty when the keyboard does not know it yet).
    Firmware(Side, String),
}

pub const TTL_SECS: u8 = 60;
pub const RENEW_EVERY_MS: u64 = 20_000;
/// Silence after `on_connect` that means the firmware does not implement layer notify.
pub const SUPPORT_TIMEOUT_MS: u64 = 1_000;

#[derive(Debug, Default)]
pub struct LayerClient {
    connected_at_ms: Option<u64>,
    last_renew_ms: u64,
    /// `None` until the first layer packet (supported) or a second of silence (unsupported).
    pub supported: Option<bool>,
}

impl LayerClient {
    pub fn new() -> LayerClient {
        LayerClient::default()
    }

    /// Packets to send right after the Vial interface opens: subscribe, then ask for the state.
    pub fn on_connect(&mut self, now_ms: u64) -> Vec<Packet> {
        self.connected_at_ms = Some(now_ms);
        self.last_renew_ms = now_ms;
        self.supported = None;
        vec![req_layer_subscribe(TTL_SECS), req_layer_state(), req_firmware_build(Side::Left), req_firmware_build(Side::Right)]
    }

    /// A renewal packet when one is due.
    pub fn tick(&mut self, now_ms: u64) -> Option<Packet> {
        if now_ms.saturating_sub(self.last_renew_ms) >= RENEW_EVERY_MS {
            self.last_renew_ms = now_ms;
            Some(req_layer_subscribe(TTL_SECS))
        } else {
            None
        }
    }

    /// Feed every packet read from the device; layer-state packets come back parsed.
    pub fn on_packet(&mut self, pkt: &Packet) -> Option<LayerPacket> {
        let packet = match (parse_layer_state(pkt), parse_handoff(pkt), parse_firmware_build(pkt)) {
            (Some(state), _, _) => LayerPacket::State(state),
            (_, Some(side), _) => LayerPacket::Handoff(side),
            (_, _, Some((side, build))) => LayerPacket::Firmware(side, build),
            _ => return None,
        };
        self.supported = Some(true);
        Some(packet)
    }

    /// Call on read timeouts; decides "unsupported" after a second without any layer packet.
    pub fn on_timeout(&mut self, now_ms: u64) {
        if self.supported.is_none()
            && self
                .connected_at_ms
                .is_some_and(|t| now_ms.saturating_sub(t) >= SUPPORT_TIMEOUT_MS)
        {
            self.supported = Some(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_packet_distinguishes_state_and_handoff() {
        let mut c = LayerClient::new();
        let mut state = [0u8; 32];
        state[..7].copy_from_slice(&[0xE9, 0x01, 2, 0, 0x04, 0x00, 1]);
        assert!(matches!(c.on_packet(&state), Some(LayerPacket::State(s)) if s.active == 2));
        let mut handoff = [0u8; 32];
        handoff[..3].copy_from_slice(&[0xE9, 0x02, 1]);
        assert_eq!(c.on_packet(&handoff), Some(LayerPacket::Handoff(Side::Right)));
        assert_eq!(c.supported, Some(true));
        let other = [0x08u8; 32];
        assert_eq!(c.on_packet(&other), None);
        let mut fw = [0u8; 32];
        fw[..4].copy_from_slice(&[0xE9, 0x03, 0, 2]);
        fw[4..6].copy_from_slice(b"v1");
        assert_eq!(c.on_packet(&fw), Some(LayerPacket::Firmware(Side::Left, "v1".into())));
    }

    #[test]
    fn connect_sends_subscribe_then_get_state() {
        let mut c = LayerClient::new();
        let v = c.on_connect(0);
        assert_eq!(v.len(), 4, "subscribe, get state, both firmware builds");
        assert_eq!(&v[0][..4], &[0x07, 0xE9, 0x01, 60]);
        assert_eq!(&v[1][..3], &[0x08, 0xE9, 0x01]);
        assert_eq!(&v[2][..4], &[0x08, 0xE9, 0x03, 0]);
        assert_eq!(&v[3][..4], &[0x08, 0xE9, 0x03, 1]);
    }

    #[test]
    fn renews_every_20_seconds() {
        let mut c = LayerClient::new();
        c.on_connect(0);
        assert!(c.tick(19_999).is_none());
        let renew = c.tick(20_000).expect("renew due");
        assert_eq!(&renew[..4], &[0x07, 0xE9, 0x01, 60]);
        assert!(c.tick(20_001).is_none());
        assert!(c.tick(40_000).is_some());
    }

    #[test]
    fn first_layer_packet_marks_supported() {
        let mut c = LayerClient::new();
        c.on_connect(0);
        assert_eq!(c.supported, None);
        let mut p = [0u8; 32];
        p[..3].copy_from_slice(&[0xE9, 0x01, 2]);
        let Some(LayerPacket::State(state)) = c.on_packet(&p) else { panic!("layer state") };
        assert_eq!(state.active, 2);
        assert_eq!(c.supported, Some(true));
        assert!(c.on_packet(&[0x11; 32]).is_none());
    }

    #[test]
    fn unsupported_after_one_second_silence() {
        let mut c = LayerClient::new();
        c.on_connect(0);
        c.on_timeout(999);
        assert_eq!(c.supported, None);
        c.on_timeout(1_000);
        assert_eq!(c.supported, Some(false));
    }

    #[test]
    fn silence_after_support_confirmed_does_not_revoke_it() {
        let mut c = LayerClient::new();
        c.on_connect(0);
        let mut p = [0u8; 32];
        p[..2].copy_from_slice(&[0xE9, 0x01]);
        c.on_packet(&p);
        c.on_timeout(5_000);
        assert_eq!(c.supported, Some(true));
    }
}
