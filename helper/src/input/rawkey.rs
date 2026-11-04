//! One key event from any keyboard, in the Windows vocabulary (set-1 scan code and virtual key) the
//! modifier tracker and the find-key search share. Linux evdev events are translated into it.

use crate::input::modifiers::{
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT,
    VK_SHIFT,
};
use crate::legend::hid_table::ScanCode;
use crate::model::DeviceKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKey {
    pub scancode: ScanCode,
    /// Virtual key, with generic Shift/Ctrl/Alt resolved to the left/right variant.
    pub vk: u16,
    pub down: bool,
    pub device: DeviceKind,
}

impl RawKey {
    #[cfg_attr(not(windows), allow(dead_code))]
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

