//! Track Shift / Ctrl / Alt / Win from raw key events (left and right folded together).

use crate::input::raw_input::RawKey;
use crate::model::Modifiers;

pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LMENU: u16 = 0xA4;
pub const VK_RMENU: u16 = 0xA5;

pub fn is_modifier_vk(vk: u16) -> bool {
    matches!(
        vk,
        VK_SHIFT
            | VK_CONTROL
            | VK_MENU
            | VK_LWIN
            | VK_RWIN
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_LMENU
            | VK_RMENU
    )
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ModTracker {
    left: Modifiers,
    right: Modifiers,
}

impl ModTracker {
    /// Update from one key event and return the folded state.
    pub fn apply(&mut self, k: &RawKey) -> Modifiers {
        let d = k.down;
        match k.vk {
            VK_LSHIFT | VK_SHIFT => self.left.shift = d,
            VK_RSHIFT => self.right.shift = d,
            VK_LCONTROL | VK_CONTROL => self.left.ctrl = d,
            VK_RCONTROL => self.right.ctrl = d,
            VK_LMENU | VK_MENU => self.left.alt = d,
            VK_RMENU => self.right.alt = d,
            VK_LWIN => self.left.win = d,
            VK_RWIN => self.right.win = d,
            _ => {}
        }
        self.current()
    }

    pub fn current(&self) -> Modifiers {
        Modifiers {
            shift: self.left.shift || self.right.shift,
            ctrl: self.left.ctrl || self.right.ctrl,
            alt: self.left.alt || self.right.alt,
            win: self.left.win || self.right.win,
        }
    }

    /// Re-read the real key state (after focus changes, so a modifier released while another window
    /// had focus never sticks).
    pub fn resync(&mut self) -> Modifiers {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        // SAFETY: GetAsyncKeyState has no preconditions.
        let down = |vk: u16| unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 };
        self.left = Modifiers {
            shift: down(VK_LSHIFT),
            ctrl: down(VK_LCONTROL),
            alt: down(VK_LMENU),
            win: down(VK_LWIN),
        };
        self.right = Modifiers {
            shift: down(VK_RSHIFT),
            ctrl: down(VK_RCONTROL),
            alt: down(VK_RMENU),
            win: down(VK_RWIN),
        };
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legend::hid_table::ScanCode;
    use crate::model::DeviceKind;

    fn key(vk: u16, down: bool) -> RawKey {
        RawKey {
            scancode: ScanCode { code: 0, extended: false },
            vk,
            down,
            device: DeviceKind::Other,
        }
    }

    #[test]
    fn tracker_folds_left_and_right_shift() {
        let mut t = ModTracker::default();
        assert!(t.apply(&key(VK_LSHIFT, true)).shift);
        t.apply(&key(VK_RSHIFT, true));
        assert!(t.apply(&key(VK_LSHIFT, false)).shift, "right shift still held");
        assert_eq!(t.apply(&key(VK_RSHIFT, false)), Modifiers::default());
    }

    #[test]
    fn tracker_handles_all_four_modifiers() {
        let mut t = ModTracker::default();
        t.apply(&key(VK_LCONTROL, true));
        t.apply(&key(VK_RMENU, true));
        let m = t.apply(&key(VK_LWIN, true));
        assert_eq!(
            m,
            Modifiers {
                shift: false,
                ctrl: true,
                alt: true,
                win: true
            }
        );
    }

    #[test]
    fn tracker_ignores_non_modifiers() {
        let mut t = ModTracker::default();
        assert_eq!(t.apply(&key(0x41, true)), Modifiers::default()); // 'A'
    }

    #[test]
    fn generic_shift_vk_counts_as_left() {
        let mut t = ModTracker::default();
        assert!(t.apply(&key(VK_SHIFT, true)).shift);
        assert!(!t.apply(&key(VK_SHIFT, false)).shift);
    }
}
