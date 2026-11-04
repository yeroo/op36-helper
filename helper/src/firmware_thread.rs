//! Flash one half from the bundled firmware: poll removable drives for the OP36's UF2 bootloader,
//! copy the half's file, wait for the half to reboot. Steps reach the UI thread through a [`Sink`],
//! like the HID thread's events.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};


use crate::firmware::{Bundle, FlashMachine, FlashStep, is_nrf52840_uf2, uf2_for};
use crate::vial::protocol::Side;
use crate::sink::Sink;

const POLL: Duration = Duration::from_millis(500);

type UiWindow = Sink<FlashStep>;

/// Start flashing `side`; drop or send on the returned sender to cancel.
pub fn spawn(ui: Sink<FlashStep>, side: Side, bundle: Bundle) -> Sender<()> {
    let (tx, rx) = std::sync::mpsc::channel();
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
                post(&ui, FlashStep::Cancelled);
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
                copy_result = Some(copy_uf2(&src, target));
            }
        }
        if last.as_ref() != Some(&step) {
            post(&ui, step.clone());
            last = Some(step.clone());
        }
        if step.is_final() {
            return;
        }
        std::thread::sleep(POLL);
    }
}

/// The root of a removable drive whose `INFO_UF2.TXT` names an nRF52840, if one is present.
#[cfg(windows)]
fn find_uf2_drive() -> Option<PathBuf> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows::core::PCWSTR;
    /// `GetDriveTypeW` result for removable media (`DRIVE_REMOVABLE` in winbase.h).
    const DRIVE_REMOVABLE: u32 = 2;
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

/// Linux: a mounted FAT volume (the desktop automounts the bootloader drive under `/media/$USER`
/// or `/run/media/$USER`) whose `INFO_UF2.TXT` names an nRF52840.
#[cfg(target_os = "linux")]
fn find_uf2_drive() -> Option<PathBuf> {
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    uf2_mount_points(&mounts).into_iter().find(|root| {
        std::fs::read_to_string(root.join("INFO_UF2.TXT"))
            .map(|text| is_nrf52840_uf2(&text))
            .unwrap_or(false)
    })
}

/// FAT mount points from a `/proc/mounts` listing (octal escapes such as ` ` decoded).
#[cfg(any(target_os = "linux", test))]
pub fn uf2_mount_points(mounts: &str) -> Vec<PathBuf> {
    mounts
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (_dev, dir, fstype) = (f.next()?, f.next()?, f.next()?);
            matches!(fstype, "vfat" | "msdos" | "fat").then(|| PathBuf::from(unescape_mount(dir)))
        })
        .collect()
}

#[cfg(any(target_os = "linux", test))]
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Copy the UF2 onto the bootloader drive. On Linux the write is flushed before returning: the
/// bootloader reboots once it has the whole file, and a page-cached copy would never reach it.
fn copy_uf2(src: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    let data = std::fs::read(src).map_err(|e| e.to_string())?;
    let dest = if target.is_dir() {
        target.join(src.file_name().ok_or("bad firmware file name")?)
    } else {
        target.to_path_buf()
    };
    let mut file = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, &data).map_err(|e| e.to_string())?;
    match file.sync_all() {
        Ok(()) => Ok(()),
        // The bootloader may reboot (and the drive vanish) the moment the last block lands.
        Err(e) => {
            log::info!("flash: sync after copy: {e} (the half usually restarts here)");
            Ok(())
        }
    }
}

fn post(ui: &UiWindow, step: FlashStep) {
    ui(step);
}

/// Windows: reclaim a step posted for `WM_APP_FLASH`.
#[cfg(windows)]
pub fn take_step(lparam: windows::Win32::Foundation::LPARAM) -> Box<FlashStep> {
    crate::sink::take_boxed(lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fat_mounts_are_found_and_unescaped() {
        let mounts = "/dev/nvme0n1p2 / ext4 rw 0 0
/dev/sda /media/me/NICENANO vfat rw,flush 0 0
/dev/sdb /media/me/MY\\040DISK vfat rw 0 0
";
        assert_eq!(
            uf2_mount_points(mounts),
            vec![
                PathBuf::from("/media/me/NICENANO"),
                PathBuf::from("/media/me/MY DISK")
            ]
        );
    }
}
