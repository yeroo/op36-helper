//! The firmware pair bundled next to the exe (`firmware\op36_left.uf2`, `op36_right.uf2`,
//! `BUILD_COMMIT.txt`), how each half's reported build compares to it, and the pure state machine
//! behind "Flash this half": wait for the bootloader drive, copy, wait for the reboot.

use std::path::{Path, PathBuf};

use crate::vial::protocol::Side;

/// Where a bundle lives relative to the exe, and the files it must contain.
pub const BUNDLE_DIR: &str = "firmware";
pub const BUILD_COMMIT_FILE: &str = "BUILD_COMMIT.txt";
/// Waiting for the bootloader drive, then for the half to reboot, before giving up.
pub const DRIVE_TIMEOUT_MS: u64 = 60_000;
pub const REBOOT_TIMEOUT_MS: u64 = 10_000;

/// The firmware pair shipped next to the exe and the build id it was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub dir: PathBuf,
    pub build: String,
}

/// `None` unless both UF2 files and `BUILD_COMMIT.txt` are present under `<exe dir>/firmware`.
pub fn bundle_at(exe_dir: &Path) -> Option<Bundle> {
    let dir = exe_dir.join(BUNDLE_DIR);
    for side in [Side::Left, Side::Right] {
        if !dir.join(uf2_for(side)).is_file() {
            return None;
        }
    }
    let build = std::fs::read_to_string(dir.join(BUILD_COMMIT_FILE)).ok()?.trim().to_string();
    (!build.is_empty()).then_some(Bundle { dir, build })
}

pub fn uf2_for(side: Side) -> &'static str {
    match side {
        Side::Left => "op36_left.uf2",
        Side::Right => "op36_right.uf2",
    }
}

/// The drive is an nRF52840 UF2 bootloader (the OP36's), judged from its `INFO_UF2.TXT`.
pub fn is_nrf52840_uf2(info_uf2_txt: &str) -> bool {
    info_uf2_txt.to_ascii_lowercase().contains("nrf52840")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalfStatus {
    UpToDate,
    UpdateAvailable,
    /// The half reported no build (older firmware, or not reported yet).
    Unknown,
}

pub fn half_status(reported: Option<&str>, bundled: &str) -> HalfStatus {
    match reported {
        None | Some("") => HalfStatus::Unknown,
        Some(r) if r == bundled => HalfStatus::UpToDate,
        Some(_) => HalfStatus::UpdateAvailable,
    }
}

/// Both halves reported a build and they disagree (they share the split protocol, so this is a
/// state to leave quickly).
pub fn halves_differ(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(l), Some(r)) if !l.is_empty() && !r.is_empty() && l != r)
}

/// The half to flash first: the one that lags the bundle; the left when nothing tells them apart.
pub fn first_to_flash(left: Option<&str>, right: Option<&str>, bundled: &str) -> Side {
    match (half_status(left, bundled), half_status(right, bundled)) {
        (HalfStatus::UpToDate, HalfStatus::UpdateAvailable) | (HalfStatus::UpToDate, HalfStatus::Unknown) => Side::Right,
        (HalfStatus::Unknown, HalfStatus::UpdateAvailable) => Side::Right,
        _ => Side::Left,
    }
}

/// What the Flash flow is doing, as shown in the Options dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashStep {
    WaitingForDrive,
    Copying(PathBuf),
    WaitingForReboot,
    Done,
    TimedOut,
    Cancelled,
    Failed(String),
}

impl FlashStep {
    pub fn is_final(&self) -> bool {
        !matches!(self, FlashStep::WaitingForDrive | FlashStep::Copying(_) | FlashStep::WaitingForReboot)
    }

    /// The sentence the dialog shows for this step.
    pub fn text(&self, side: Side) -> String {
        let name = match side {
            Side::Left => "left",
            Side::Right => "right",
        };
        match self {
            FlashStep::WaitingForDrive => {
                format!("Double-tap the reset button on the {name} half; waiting for its drive…")
            }
            FlashStep::Copying(_) => format!("Copying the {name} half's firmware…"),
            FlashStep::WaitingForReboot => "Copied; waiting for the half to restart…".to_string(),
            FlashStep::Done => format!("Flashed the {name} half; it reconnects in a few seconds."),
            FlashStep::TimedOut => format!("No bootloader drive appeared within {} s.", DRIVE_TIMEOUT_MS / 1000),
            FlashStep::Cancelled => "Flash cancelled.".to_string(),
            FlashStep::Failed(e) => format!("Flash failed: {e}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Copying,
    Rebooting,
    Finished,
}

/// Pure driver of one flash: fed with the clock, the bootloader drive seen (if any) and the
/// result of the copy once it ran.
pub struct FlashMachine {
    side: Side,
    started_ms: u64,
    copy_started_ms: u64,
    phase: Phase,
}

impl FlashMachine {
    pub fn new(side: Side, now_ms: u64) -> Self {
        Self { side, started_ms: now_ms, copy_started_ms: 0, phase: Phase::Waiting }
    }

    pub fn step(&mut self, now_ms: u64, drive: Option<PathBuf>, copy_result: Option<Result<(), String>>) -> FlashStep {
        match self.phase {
            Phase::Waiting => match drive {
                Some(root) => {
                    self.phase = Phase::Copying;
                    FlashStep::Copying(root.join(uf2_for(self.side)))
                }
                None if now_ms.saturating_sub(self.started_ms) >= DRIVE_TIMEOUT_MS => {
                    self.phase = Phase::Finished;
                    FlashStep::TimedOut
                }
                None => FlashStep::WaitingForDrive,
            },
            Phase::Copying => match copy_result {
                Some(Ok(())) => {
                    self.phase = Phase::Rebooting;
                    self.copy_started_ms = now_ms;
                    FlashStep::WaitingForReboot
                }
                Some(Err(e)) => {
                    self.phase = Phase::Finished;
                    FlashStep::Failed(e)
                }
                None => FlashStep::Copying(drive.unwrap_or_default().join(uf2_for(self.side))),
            },
            Phase::Rebooting => {
                if drive.is_none() || now_ms.saturating_sub(self.copy_started_ms) >= REBOOT_TIMEOUT_MS {
                    // The drive vanishing is the reboot; a drive that stays is still a copied file.
                    self.phase = Phase::Finished;
                    FlashStep::Done
                } else {
                    FlashStep::WaitingForReboot
                }
            }
            Phase::Finished => FlashStep::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vial::protocol::Side;
    use std::path::PathBuf;

    #[test]
    fn unknown_reported_build_is_not_an_update() {
        assert_eq!(half_status(None, "v0.2.0-23-g74ed094c"), HalfStatus::Unknown);
        assert_eq!(half_status(Some(""), "v0.2.0-23-g74ed094c"), HalfStatus::Unknown);
        assert_eq!(half_status(Some("v0.2.0-23-g74ed094c"), "v0.2.0-23-g74ed094c"), HalfStatus::UpToDate);
        assert_eq!(half_status(Some("v0.2.0-21-g4bf2fa45"), "v0.2.0-23-g74ed094c"), HalfStatus::UpdateAvailable);
    }

    #[test]
    fn only_an_nrf52840_uf2_drive_is_accepted() {
        assert!(is_nrf52840_uf2(
            "UF2 Bootloader 0.9.2 lib/nrfx\r\nModel: Nordic nRF52840 DK\r\nBoard-ID: nRF52840-pca10056-v1\r\n"
        ));
        assert!(!is_nrf52840_uf2("UF2 Bootloader v3.0\r\nModel: Raspberry Pi RP2\r\nBoard-ID: RPI-RP2\r\n"));
        assert!(!is_nrf52840_uf2(""));
    }

    #[test]
    fn differing_halves_are_flagged() {
        assert!(halves_differ(Some("a"), Some("b")));
        assert!(!halves_differ(Some("a"), Some("a")));
        assert!(!halves_differ(None, Some("a")), "unknown is not a difference");
        assert_eq!(first_to_flash(Some("v23"), Some("v21"), "v23"), Side::Right);
        assert_eq!(first_to_flash(None, None, "v23"), Side::Left);
    }

    #[test]
    fn flash_machine_waits_copies_and_times_out() {
        let mut m = FlashMachine::new(Side::Left, 0);
        assert_eq!(m.step(500, None, None), FlashStep::WaitingForDrive);
        assert_eq!(
            m.step(1_000, Some(PathBuf::from(r"E:\")), None),
            FlashStep::Copying(PathBuf::from(r"E:\op36_left.uf2"))
        );
        assert_eq!(m.step(1_500, Some(PathBuf::from(r"E:\")), Some(Ok(()))), FlashStep::WaitingForReboot);
        assert_eq!(m.step(2_000, None, None), FlashStep::Done, "drive vanished = rebooted");
        let mut m = FlashMachine::new(Side::Right, 0);
        assert_eq!(m.step(60_000, None, None), FlashStep::TimedOut);
        let mut m = FlashMachine::new(Side::Right, 0);
        m.step(100, Some(PathBuf::from(r"E:\")), None);
        assert_eq!(
            m.step(200, Some(PathBuf::from(r"E:\")), Some(Err("denied".into()))),
            FlashStep::Failed("denied".into())
        );
    }

    #[test]
    fn bundle_reads_build_commit_next_to_the_exe() {
        let dir = tempfile::tempdir().unwrap();
        assert!(bundle_at(dir.path()).is_none());
        std::fs::create_dir(dir.path().join("firmware")).unwrap();
        for f in ["op36_left.uf2", "op36_right.uf2"] {
            std::fs::write(dir.path().join("firmware").join(f), b"x").unwrap();
        }
        assert!(bundle_at(dir.path()).is_none(), "no BUILD_COMMIT yet");
        std::fs::write(dir.path().join("firmware/BUILD_COMMIT.txt"), "v0.2.0-23-g74ed094c\n").unwrap();
        let b = bundle_at(dir.path()).unwrap();
        assert_eq!(b.build, "v0.2.0-23-g74ed094c");
        assert_eq!(b.dir, dir.path().join("firmware"));
        assert_eq!(uf2_for(Side::Left), "op36_left.uf2");
        assert_eq!(uf2_for(Side::Right), "op36_right.uf2");
    }
}
