//! Worker events (update, flash) that arrive while `App` is borrowed are parked here and replayed
//! on the next message instead of being dropped (a dropped `Verified` left "Downloading…" forever).

use std::cell::RefCell;

pub struct Deferred<T>(RefCell<Vec<T>>);

impl<T> Default for Deferred<T> {
    fn default() -> Self {
        Self(RefCell::new(Vec::new()))
    }
}

impl<T> Deferred<T> {
    pub fn park(&self, item: T) {
        self.0.borrow_mut().push(item);
    }

    /// Everything parked so far, oldest first; the queue is empty afterwards.
    pub fn take(&self) -> Vec<T> {
        std::mem::take(&mut *self.0.borrow_mut())
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parked_events_replay_in_order_once() {
        let q: Deferred<u8> = Deferred::default();
        assert!(q.is_empty());
        q.park(1);
        q.park(2);
        assert_eq!(q.take(), vec![1, 2]);
        assert!(q.is_empty());
        assert_eq!(q.take(), Vec::<u8>::new());
    }
}
