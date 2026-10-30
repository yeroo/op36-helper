//! A lost link is shown as offline only after a grace period: Bluetooth re-negotiates in seconds.
//! Pure gate in front of the model; the app owns the timer.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Start the grace timer; keep showing the last state.
    Hold,
    /// Apply the disconnect to the model now.
    Apply,
    /// Nothing to do.
    Ignore,
}

#[derive(Debug, Default)]
pub struct OfflineGrace {
    pending: bool,
}

impl OfflineGrace {
    /// The link dropped. `connected` is what the model currently believes.
    pub fn on_disconnected(&mut self, connected: bool) -> Gate {
        if !connected {
            return Gate::Apply;
        }
        if self.pending {
            return Gate::Ignore;
        }
        self.pending = true;
        Gate::Hold
    }

    /// The grace timer fired: the link is still down, so the disconnect lands now.
    pub fn on_timer(&mut self) -> Gate {
        if self.pending {
            self.pending = false;
            Gate::Apply
        } else {
            Gate::Ignore
        }
    }

    /// The link is back; returns whether a grace timer was running (and must be cancelled).
    pub fn on_connected(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disconnect_is_held_then_applied_when_the_grace_runs_out() {
        let mut g = OfflineGrace::default();
        assert_eq!(
            g.on_disconnected(true),
            Gate::Hold,
            "first report: start the grace"
        );
        assert_eq!(
            g.on_disconnected(true),
            Gate::Ignore,
            "repeats during the grace do nothing"
        );
        assert_eq!(
            g.on_timer(),
            Gate::Apply,
            "grace over, still down: apply the disconnect"
        );
        assert_eq!(g.on_timer(), Gate::Ignore, "a stray tick does nothing");
    }

    #[test]
    fn a_reconnect_within_the_grace_cancels_it() {
        let mut g = OfflineGrace::default();
        assert_eq!(g.on_disconnected(true), Gate::Hold);
        assert!(g.on_connected(), "the timer must be cancelled");
        assert_eq!(g.on_timer(), Gate::Ignore);
        assert!(!g.on_connected(), "nothing pending: nothing to cancel");
    }

    #[test]
    fn a_disconnect_while_already_offline_applies_at_once() {
        let mut g = OfflineGrace::default();
        assert_eq!(g.on_disconnected(false), Gate::Apply);
    }
}
