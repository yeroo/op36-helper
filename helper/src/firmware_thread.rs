//! Flash one half from the bundled firmware: poll removable drives for the OP36's UF2 bootloader,
//! copy the half's file, wait for the half to reboot. Steps are posted to the UI thread as boxed
//! `FlashStep`s (`WM_APP_FLASH`), like the HID thread's events.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

/// `GetDriveTypeW` result for removable media (`DRIVE_REMOVABLE` in winbase.h).
const DRIVE_REMOVABLE: u32 = 2;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::PCWSTR;

use crate::firmware::{Bundle, FlashMachine, FlashStep, is_nrf52840_uf2, uf2_for};
use crate::vial::protocol::Side;
use crate::win::msgwindow::WM_APP_FLASH;

const POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
struct UiWindow(isize);
// SAFETY: an HWND is an opaque integer handle; PostMessageW is thread-safe.
unsafe impl Send for UiWindow {}

/// Start flashing `side`; drop or send on the returned sender to cancel.
pub fn spawn(ui_hwnd: HWND, side: Side, bundle: Bundle) -> Sender<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ui = UiWindow(ui_hwnd.0 as isize);
    std::thread::Builder::new()
        .name("op36-flash".into())
        .spawn(move || run(ui, side, bundle, rx))
        .expect("spawn flash thread");
    tx
}

fn run(ui: UiWindow, side: Side, bundle: Bundle, cancel: Receiver<()>) {
    let start = Instant::now();
    // A bootloader drive already open now may be the other half: ignored until it reappears.
    let mut machine = FlashMachine::new(side, 0, find_uf2_drive());
    let mut copy_result: Option<Result<(), String>> = None;
    let mut last: Option<FlashStep> = None;
    loop {
        match cancel.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => {
                post(ui, FlashStep::Cancelled);
                return;
            }
            Err(TryRecvError::Empty) => {}
        }
        let drive = find_uf2_drive();
        let step = machine.step(
            start.elapsed().as_millis() as u64,
            drive.clone(),
            copy_result.take(),
        );
        if let FlashStep::Copying(target) = &step {
            if last.as_ref() != Some(&step) {
                let src = bundle.dir.join(uf2_for(side));
                copy_result = Some(
                    std::fs::copy(&src, target)
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                );
            }
        }
        if last.as_ref() != Some(&step) {
            post(ui, step.clone());
            last = Some(step.clone());
        }
        if step.is_final() {
            return;
        }
        std::thread::sleep(POLL);
    }
}

/// The root of a removable drive whose `INFO_UF2.TXT` names an nRF52840, if one is present.
fn find_uf2_drive() -> Option<PathBuf> {
    // SAFETY: plain Win32 queries; `GetDriveTypeW` reads a NUL-terminated path we own.
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let kind = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
        if kind != DRIVE_REMOVABLE {
            continue;
        }
        let info = PathBuf::from(&root).join("INFO_UF2.TXT");
        if let Ok(text) = std::fs::read_to_string(&info)
            && is_nrf52840_uf2(&text)
        {
            return Some(PathBuf::from(root));
        }
    }
    None
}

fn post(ui: UiWindow, step: FlashStep) {
    let boxed = Box::new(step);
    // SAFETY: the UI thread takes ownership back with `take_step`.
    unsafe {
        let _ = PostMessageW(
            Some(HWND(ui.0 as *mut _)),
            WM_APP_FLASH,
            WPARAM(0),
            LPARAM(Box::into_raw(boxed) as isize),
        );
    }
}

/// UI thread: reclaim a step posted by `post`.
pub fn take_step(lparam: LPARAM) -> Box<FlashStep> {
    // SAFETY: `lparam` is a pointer produced by `Box::into_raw` in `post`.
    unsafe { Box::from_raw(lparam.0 as *mut FlashStep) }
}
