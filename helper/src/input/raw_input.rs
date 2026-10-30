//! Raw Input registration and parsing: every key press on the system, with its source device.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RID_INPUT, RIDEV_INPUTSINK, RIDI_DEVICENAME, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
};

use crate::input::modifiers::{
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT,
    VK_SHIFT,
};
use crate::legend::hid_table::ScanCode;
use crate::model::DeviceKind;

const RI_KEY_BREAK: u16 = 0x01;
const RI_KEY_E0: u16 = 0x02;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKey {
    pub scancode: ScanCode,
    /// Virtual key, with generic Shift/Ctrl/Alt resolved to the left/right variant.
    pub vk: u16,
    pub down: bool,
    pub device: DeviceKind,
}

impl RawKey {
    pub fn from_fields(
        make_code: u8,
        extended: bool,
        vk: u16,
        down: bool,
        device: DeviceKind,
    ) -> RawKey {
        let vk = match vk {
            VK_SHIFT if make_code == 0x36 => VK_RSHIFT,
            VK_SHIFT => VK_LSHIFT,
            VK_CONTROL if extended => VK_RCONTROL,
            VK_CONTROL => VK_LCONTROL,
            VK_MENU if extended => VK_RMENU,
            VK_MENU => VK_LMENU,
            other => other,
        };
        RawKey {
            scancode: ScanCode {
                code: make_code,
                extended,
            },
            vk,
            down,
            device,
        }
    }
}

/// `Op36` when the Raw Input device name carries the keyboard's VID/PID.
pub fn device_kind_from_name(name: &str) -> DeviceKind {
    let upper = name.to_ascii_uppercase();
    if upper.contains("VID_E126&PID_0036") {
        DeviceKind::Op36
    } else {
        DeviceKind::Other
    }
}

/// Receive keyboard input from every device, even when another window has focus.
pub fn register(hwnd: HWND) -> windows::core::Result<()> {
    let rid = RAWINPUTDEVICE {
        usUsagePage: 1,
        usUsage: 6,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };
    // SAFETY: one fully initialised descriptor.
    unsafe { RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32) }
}

thread_local! {
    static DEVICE_KINDS: RefCell<HashMap<isize, DeviceKind>> = RefCell::new(HashMap::new());
}

fn device_kind(handle: HANDLE) -> DeviceKind {
    let key = handle.0 as isize;
    DEVICE_KINDS.with(|cache| {
        *cache.borrow_mut().entry(key).or_insert_with(|| {
            let mut len: u32 = 0;
            let mut buf = [0u16; 512];
            // SAFETY: two-call pattern with a sized buffer.
            let name = unsafe {
                GetRawInputDeviceInfoW(Some(handle), RIDI_DEVICENAME, None, &mut len);
                let n = GetRawInputDeviceInfoW(
                    Some(handle),
                    RIDI_DEVICENAME,
                    Some(buf.as_mut_ptr() as *mut _),
                    &mut len,
                );
                if n == u32::MAX {
                    String::new()
                } else {
                    String::from_utf16_lossy(&buf[..(n as usize).min(512)])
                }
            };
            device_kind_from_name(&name)
        })
    })
}

/// Decode a `WM_INPUT` message; `None` for non-keyboard input.
pub fn parse(lparam: LPARAM) -> Option<RawKey> {
    let mut raw = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    // SAFETY: documented call; buffer is a full RAWINPUT, the largest fixed-size variant.
    let got = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut _),
            RID_INPUT,
            Some(&mut raw as *mut RAWINPUT as *mut _),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if got == u32::MAX || raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return None;
    }
    // SAFETY: dwType says the union holds a RAWKEYBOARD.
    let kb = unsafe { raw.data.keyboard };
    if kb.VKey == 0xFF {
        return None; // fake key (e.g. the extra E1 1D of Pause)
    }
    Some(RawKey::from_fields(
        kb.MakeCode as u8,
        kb.Flags & RI_KEY_E0 != 0,
        kb.VKey,
        kb.Flags & RI_KEY_BREAK == 0,
        device_kind(raw.header.hDevice),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_kind_from_name_detects_the_op36() {
        assert_eq!(
            device_kind_from_name(
                r"\\?\HID#VID_E126&PID_0036&MI_00#8&19a504f3&0&0000#{884b96c3-56ef-11d1-bc8c-00a0c91405dd}"
            ),
            DeviceKind::Op36
        );
        assert_eq!(
            device_kind_from_name(r"\\?\HID#VID_046D&PID_C548&MI_00#..."),
            DeviceKind::Other
        );
        assert_eq!(
            device_kind_from_name(r"\\?\hid#vid_e126&pid_0036&col01#..."),
            DeviceKind::Op36
        );
        assert_eq!(device_kind_from_name(""), DeviceKind::Other);
    }

    #[test]
    fn raw_key_from_fields_builds_scancode_with_extended_flag() {
        let k = RawKey::from_fields(0x4B, true, 0x25, false, DeviceKind::Other); // Left arrow, release
        assert_eq!(k.scancode.code, 0x4B);
        assert!(k.scancode.extended);
        assert_eq!(k.vk, 0x25);
        assert!(!k.down);
    }

    #[test]
    fn generic_modifier_vks_resolve_to_sides() {
        assert_eq!(
            RawKey::from_fields(0x36, false, VK_SHIFT, true, DeviceKind::Other).vk,
            VK_RSHIFT
        );
        assert_eq!(
            RawKey::from_fields(0x2A, false, VK_SHIFT, true, DeviceKind::Other).vk,
            VK_LSHIFT
        );
        assert_eq!(
            RawKey::from_fields(0x1D, true, VK_CONTROL, true, DeviceKind::Other).vk,
            VK_RCONTROL
        );
        assert_eq!(
            RawKey::from_fields(0x38, false, VK_MENU, true, DeviceKind::Other).vk,
            VK_LMENU
        );
    }
}
