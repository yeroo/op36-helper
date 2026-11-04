//! How worker threads hand results to the UI thread, on every platform: a callback that must be
//! cheap and thread-safe (Windows posts a boxed message to the handler window; Linux sends into the
//! main loop's channel).

use std::sync::Arc;

pub type Sink<T> = Arc<dyn Fn(T) + Send + Sync>;

/// Windows: a sink that posts each value boxed to `hwnd` as `msg`; the UI thread reclaims it with
/// [`take_boxed`].
#[cfg(windows)]
pub fn post_boxed<T: Send + 'static>(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
) -> Sink<T> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    // An HWND is an opaque integer handle and PostMessageW is thread-safe.
    let raw = hwnd.0 as isize;
    Arc::new(move |value: T| {
        let boxed = Box::new(value);
        // SAFETY: the UI thread's handler takes ownership back with `take_boxed`.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(raw as *mut _)),
                msg,
                WPARAM(0),
                LPARAM(Box::into_raw(boxed) as isize),
            );
        }
    })
}

/// Windows: reclaim a value posted by [`post_boxed`]. Must be called exactly once per message.
#[cfg(windows)]
pub fn take_boxed<T>(lparam: windows::Win32::Foundation::LPARAM) -> Box<T> {
    // SAFETY: `lparam` is a pointer produced by `Box::into_raw` in `post_boxed` for this `T`.
    unsafe { Box::from_raw(lparam.0 as *mut T) }
}
