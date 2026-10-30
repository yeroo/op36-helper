//! The keyboard switched off: show it as off, hide the overlay after a minute, bring it back the
//! moment the keyboard is on again. Pure; the app owns the timer and the window.

/// How long the overlay keeps showing the switched-off keyboard before it hides itself.
pub const OFF_HIDE_DELAY_MS: u32 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Start the one-minute countdown.
    ArmTimer,
    /// The keyboard came back before the countdown ended.
    CancelTimer,
    /// Hide the overlay (the user's "visible" setting is left alone).
    Hide,
    /// The keyboard is back: show the overlay again.
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Idle,
    /// Keyboard off, overlay still showing it, countdown running.
    Counting,
    /// Overlay hidden by us; a reconnect brings it back.
    Hidden,
}

#[derive(Debug, Default)]
pub struct AutoHide {
    state: State,
}

impl AutoHide {
    pub fn on_disconnected(&mut self) -> Option<Action> {
        match self.state {
            State::Idle => {
                self.state = State::Counting;
                Some(Action::ArmTimer)
            }
            State::Counting | State::Hidden => None,
        }
    }

    pub fn on_timer(&mut self) -> Option<Action> {
        match self.state {
            State::Counting => {
                self.state = State::Hidden;
                Some(Action::Hide)
            }
            State::Idle | State::Hidden => None,
        }
    }

    pub fn on_connected(&mut self) -> Option<Action> {
        let action = match self.state {
            State::Counting => Some(Action::CancelTimer),
            State::Hidden => Some(Action::Restore),
            State::Idle => None,
        };
        self.state = State::Idle;
        action
    }

    /// The user showed or hid the overlay by hand: their choice stands, nothing automatic is
    /// pending any more.
    pub fn user_toggled(&mut self) {
        self.state = State::Idle;
    }

    pub fn is_hidden(&self) -> bool {
        self.state == State::Hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_off_hides_after_the_delay_and_comes_back_on_reconnect() {
        let mut h = AutoHide::default();
        assert_eq!(h.on_disconnected(), Some(Action::ArmTimer));
        assert_eq!(h.on_timer(), Some(Action::Hide));
        assert!(h.is_hidden());
        assert_eq!(h.on_connected(), Some(Action::Restore));
        assert!(!h.is_hidden());
    }

    #[test]
    fn a_reconnect_before_the_delay_cancels_the_hide() {
        let mut h = AutoHide::default();
        assert_eq!(h.on_disconnected(), Some(Action::ArmTimer));
        assert_eq!(h.on_connected(), Some(Action::CancelTimer));
        assert_eq!(h.on_timer(), None, "a late timer tick does nothing");
        assert_eq!(
            h.on_connected(),
            None,
            "connected while connected: nothing to restore"
        );
    }

    #[test]
    fn the_user_showing_or_hiding_the_overlay_ends_the_automatic_hide() {
        let mut h = AutoHide::default();
        h.on_disconnected();
        h.on_timer();
        h.user_toggled();
        assert!(!h.is_hidden());
        assert_eq!(
            h.on_connected(),
            None,
            "the user's choice stands; nothing to restore"
        );
    }

    #[test]
    fn the_delay_is_one_minute() {
        assert_eq!(OFF_HIDE_DELAY_MS, 60_000);
    }
}
