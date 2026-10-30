//! Byte transport to the keyboard's Vial interface (real HID, or a scripted fake for tests).

use super::protocol::Packet;

pub const VID: u16 = 0xE126;
pub const PID: u16 = 0x0036;
pub const USAGE_PAGE: u16 = 0xFF60;
pub const USAGE: u16 = 0x61;

pub trait VialTransport {
    fn write(&mut self, pkt: &Packet) -> Result<(), String>;
    /// `Ok(None)` on timeout.
    fn read(&mut self, timeout_ms: i32) -> Result<Option<Packet>, String>;
    /// How long a request may wait for its reply. Bluetooth links with power-saving connection
    /// parameters need far longer than USB.
    fn reply_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(500)
    }
}

/// How the keyboard is reached. Over BLE the firmware numbers the Vial report (id 5) because the
/// composite HID service must number every report; over USB the collection is unnumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Ble,
}

impl Transport {
    pub fn report_id(self) -> u8 {
        match self {
            Transport::Usb => 0,
            Transport::Ble => BLE_VIAL_REPORT_ID,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Transport::Usb => "USB",
            Transport::Ble => "BLE",
        }
    }
}

/// `CompositeReportType::Vial` in the firmware's BLE report map.
pub const BLE_VIAL_REPORT_ID: u8 = 5;

/// Windows exposes Bluetooth LE HID collections with the HOGP service UUID in the device path.
pub fn transport_from_path(path: &str) -> Transport {
    let p = path.to_ascii_lowercase();
    if p.contains("00001812-0000-1000-8000-00805f9b34fb")
        || p.contains("bthle")
        || p.contains("bthenum")
    {
        Transport::Ble
    } else {
        Transport::Usb
    }
}

/// Bytes to hand to hidapi for `pkt`: report id first (0 for unnumbered collections).
pub fn frame_write(report_id: u8, pkt: &Packet) -> [u8; 33] {
    let mut buf = [0u8; 33];
    buf[0] = report_id;
    buf[1..].copy_from_slice(pkt);
    buf
}

/// The 32 payload bytes of a report read from hidapi (`n` bytes long).
pub fn unframe_read(report_id: u8, buf: &[u8; 33], n: usize) -> Option<Packet> {
    if n == 0 {
        return None;
    }
    let mut pkt = [0u8; 32];
    if report_id != 0 && buf[0] == report_id && n >= 33 {
        pkt.copy_from_slice(&buf[1..33]);
    } else {
        pkt.copy_from_slice(&buf[..32]);
    }
    Some(pkt)
}

pub struct HidTransport {
    device: hidapi::HidDevice,
    pub transport: Transport,
}

/// A Bluetooth session should give way as soon as a USB collection of the keyboard exists: the
/// keyboard types over USB whenever it is enumerated, and the glyph must say so (a dual-role swap
/// brings the bonded Bluetooth link up before the USB enumeration finishes).
pub fn should_upgrade<'a>(current: Transport, paths: impl Iterator<Item = &'a str>) -> bool {
    current == Transport::Ble
        && paths
            .into_iter()
            .any(|p| transport_from_path(p) == Transport::Usb)
}

impl HidTransport {
    /// `true` when a USB collection of the keyboard is present while this session runs over Bluetooth.
    pub fn usb_available(&self, api: &hidapi::HidApi) -> bool {
        let paths = api
            .device_list()
            .filter(|d| {
                d.vendor_id() == VID
                    && d.product_id() == PID
                    && d.usage_page() == USAGE_PAGE
                    && d.usage() == USAGE
            })
            .map(|d| d.path().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        should_upgrade(self.transport, paths.iter().map(String::as_str))
    }

    /// Open the OP36's raw-HID (Vial) collection, preferring USB over Bluetooth when both are present.
    pub fn open(api: &hidapi::HidApi) -> Option<HidTransport> {
        let mut candidates: Vec<_> = api
            .device_list()
            .filter(|d| {
                d.vendor_id() == VID
                    && d.product_id() == PID
                    && d.usage_page() == USAGE_PAGE
                    && d.usage() == USAGE
            })
            .collect();
        candidates
            .sort_by_key(|d| transport_from_path(&d.path().to_string_lossy()) == Transport::Ble);
        for info in candidates {
            let transport = transport_from_path(&info.path().to_string_lossy());
            match api.open_path(info.path()) {
                Ok(device) => {
                    log::info!("Vial interface opened over {}", transport.label());
                    return Some(HidTransport { device, transport });
                }
                Err(e) => log::warn!("hid open ({}): {e}", transport.label()),
            }
        }
        None
    }
}

impl VialTransport for HidTransport {
    fn write(&mut self, pkt: &Packet) -> Result<(), String> {
        let buf = frame_write(self.transport.report_id(), pkt);
        self.device
            .write(&buf)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn read(&mut self, timeout_ms: i32) -> Result<Option<Packet>, String> {
        let mut buf = [0u8; 33];
        match self.device.read_timeout(&mut buf, timeout_ms) {
            Ok(n) => Ok(unframe_read(self.transport.report_id(), &buf, n)),
            Err(e) => Err(e.to_string()),
        }
    }

    fn reply_timeout(&self) -> std::time::Duration {
        match self.transport {
            Transport::Usb => std::time::Duration::from_millis(500),
            Transport::Ble => std::time::Duration::from_millis(3000),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ble_session_upgrades_when_a_usb_path_appears() {
        let ble = r"\?\hid#{00001812-0000-1000-8000-00805f9b34fb}_dev_vid&02e126_pid&0036#8&1&0#{4d1e55b2}";
        let usb = r"\?\hid#vid_e126&pid_0036&mi_02#7&1&0&0000#{4d1e55b2}";
        assert!(should_upgrade(Transport::Ble, [ble, usb].into_iter()));
        assert!(
            !should_upgrade(Transport::Ble, [ble].into_iter()),
            "no USB path yet"
        );
        assert!(
            !should_upgrade(Transport::Usb, [ble, usb].into_iter()),
            "already on USB"
        );
    }

    #[test]
    fn ble_paths_are_recognised() {
        assert_eq!(
            transport_from_path(
                r"\?\hid#{00001812-0000-1000-8000-00805f9b34fb}_dev_vid&02e126_pid&0036&col05#9&1#{...}"
            ),
            Transport::Ble
        );
        assert_eq!(
            transport_from_path(r"\?\hid#vid_e126&pid_0036&mi_02#7&2edf967e&0&0000#{...}"),
            Transport::Usb
        );
    }

    #[test]
    fn ble_framing_prefixes_and_strips_report_id_5() {
        let pkt = [7u8; 32];
        let w = frame_write(BLE_VIAL_REPORT_ID, &pkt);
        assert_eq!(w[0], 5);
        assert_eq!(&w[1..], &pkt);
        let mut r = [0u8; 33];
        r[0] = 5;
        r[1..].copy_from_slice(&pkt);
        assert_eq!(unframe_read(BLE_VIAL_REPORT_ID, &r, 33), Some(pkt));
    }

    #[test]
    fn usb_framing_is_unnumbered() {
        let pkt = [9u8; 32];
        assert_eq!(frame_write(0, &pkt)[0], 0);
        let mut r = [0u8; 33];
        r[..32].copy_from_slice(&pkt);
        assert_eq!(unframe_read(0, &r, 32), Some(pkt));
        assert_eq!(unframe_read(0, &r, 0), None);
    }
}

/// Scripted transport for unit tests: `script` is what the keyboard will "send", `sent` records writes.
#[cfg(test)]
#[derive(Default)]
pub struct FakeTransport {
    pub script: std::collections::VecDeque<Packet>,
    pub sent: Vec<Packet>,
}

#[cfg(test)]
impl VialTransport for FakeTransport {
    fn write(&mut self, pkt: &Packet) -> Result<(), String> {
        self.sent.push(*pkt);
        Ok(())
    }

    fn read(&mut self, _timeout_ms: i32) -> Result<Option<Packet>, String> {
        Ok(self.script.pop_front())
    }
}
