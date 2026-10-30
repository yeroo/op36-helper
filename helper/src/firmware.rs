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
    let build = std::fs::read_to_string(dir.join(BUILD_COMMIT_FILE))
        .ok()?
        .trim()
        .to_string();
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
    // The firmware reports at most FIRMWARE_BUILD_MAX bytes of the id; compare what it can say.
    let max = crate::vial::protocol::FIRMWARE_BUILD_MAX;
    let bundled = bundled.get(..max).unwrap_or(bundled);
    match reported {
        None | Some("") => HalfStatus::Unknown,
        Some(r) if r == bundled => HalfStatus::UpToDate,
        Some(_) => HalfStatus::UpdateAvailable,
    }
}

/// The three lines of the Options "Firmware" group: each half's build against the bundle, and the
/// advice line. A disconnected keyboard reports nothing, which is not a reason to flash anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirmwareWords {
    pub left: String,
    pub right: String,
    pub note: String,
}

pub fn describe(
    connected: bool,
    left: Option<&str>,
    right: Option<&str>,
    bundled: Option<&str>,
) -> FirmwareWords {
    if !connected {
        return FirmwareWords {
            left: "Left half: not connected".into(),
            right: "Right half: not connected".into(),
            note: "Connect the keyboard to see the firmware each half runs.".into(),
        };
    }
    let word = |reported: Option<&str>| match bundled {
        None => reported.unwrap_or("unknown").to_string(),
        Some(b) => match half_status(reported, b) {
            HalfStatus::UpToDate => format!("{} (up to date)", reported.unwrap_or("")),
            HalfStatus::UpdateAvailable => format!("{} (update available)", reported.unwrap_or("")),
            HalfStatus::Unknown => "unknown — flash both halves".to_string(),
        },
    };
    let mut note = String::new();
    if let Some(b) = bundled {
        let lagging = [left, right]
            .iter()
            .any(|r| half_status(*r, b) != HalfStatus::UpToDate);
        if lagging {
            let first = first_to_flash(left, right, b);
            note.push_str(&format!(
                "Flash the {} half first, then the other one.",
                if first == Side::Left { "left" } else { "right" }
            ));
        }
    }
    if halves_differ(left, right) {
        note.push_str(" The halves run different builds.");
    }
    FirmwareWords {
        left: format!("Left half: {}", word(left)),
        right: format!("Right half: {}", word(right)),
        note: note.trim().to_string(),
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
        (HalfStatus::UpToDate, HalfStatus::UpdateAvailable)
        | (HalfStatus::UpToDate, HalfStatus::Unknown) => Side::Right,
        (HalfStatus::Unknown, HalfStatus::UpdateAvailable) => Side::Right,
        _ => Side::Left,
    }
}

/// What the Flash flow is doing, as shown in the Options dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashStep {
    WaitingForDrive,
    /// A bootloader drive was already open when Flash was clicked; it is not trusted to be the
    /// half the user named until it goes away and comes back.
    AlreadyMounted(PathBuf),
    Copying(PathBuf),
    WaitingForReboot,
    Done,
    TimedOut,
    Cancelled,
    Failed(String),
}

impl FlashStep {
    pub fn is_final(&self) -> bool {
        !matches!(
            self,
            FlashStep::WaitingForDrive | FlashStep::Copying(_) | FlashStep::WaitingForReboot
        )
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
            FlashStep::AlreadyMounted(root) => format!(
                "{} was already open when you clicked. Double-tap reset on the {name} half so its drive appears fresh.",
                root.display()
            ),
            FlashStep::Copying(_) => format!("Copying the {name} half's firmware…"),
            FlashStep::WaitingForReboot => "Copied; waiting for the half to restart…".to_string(),
            FlashStep::Done => format!("Flashed the {name} half; it reconnects in a few seconds."),
            FlashStep::TimedOut => format!(
                "No bootloader drive appeared within {} s.",
                DRIVE_TIMEOUT_MS / 1000
            ),
            FlashStep::Cancelled => "Flash cancelled.".to_string(),
            FlashStep::Failed(e) => format!("Flash failed: {e}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Copying,
    /// The copy returned an error; the bootloader may simply have rebooted before the file was
    /// closed, so the drive gets a moment to vanish before this counts as a failure.
    CopyFailed,
    Rebooting,
    Finished,
}

/// How long a copy error waits for the drive to disappear before it is reported.
pub const COPY_FAIL_GRACE_MS: u64 = 3_000;

/// Pure driver of one flash: fed with the clock, the bootloader drive seen (if any) and the
/// result of the copy once it ran.
pub struct FlashMachine {
    side: Side,
    started_ms: u64,
    copy_started_ms: u64,
    phase: Phase,
    /// A drive that was already there at the click: ignored until it has gone away once.
    ignore: Option<PathBuf>,
    copy_error: Option<String>,
}

impl FlashMachine {
    /// `mounted`: the bootloader drive present at the moment of the click, if any.
    pub fn new(side: Side, now_ms: u64, mounted: Option<PathBuf>) -> Self {
        Self {
            side,
            started_ms: now_ms,
            copy_started_ms: 0,
            phase: Phase::Waiting,
            ignore: mounted,
            copy_error: None,
        }
    }

    pub fn step(
        &mut self,
        now_ms: u64,
        drive: Option<PathBuf>,
        copy_result: Option<Result<(), String>>,
    ) -> FlashStep {
        match self.phase {
            Phase::Waiting => match drive {
                Some(root) if self.ignore.as_ref() == Some(&root) => {
                    FlashStep::AlreadyMounted(root)
                }
                Some(root) => {
                    self.ignore = None;
                    self.phase = Phase::Copying;
                    FlashStep::Copying(root.join(uf2_for(self.side)))
                }
                None if self.ignore.is_some() => {
                    self.ignore = None;
                    FlashStep::WaitingForDrive
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
                    self.phase = Phase::CopyFailed;
                    self.copy_started_ms = now_ms;
                    self.copy_error = Some(e);
                    FlashStep::WaitingForReboot
                }
                None => FlashStep::Copying(drive.unwrap_or_default().join(uf2_for(self.side))),
            },
            Phase::CopyFailed => {
                if drive.is_none() {
                    // The write landed and the half rebooted before the file was closed.
                    self.phase = Phase::Finished;
                    FlashStep::Done
                } else if now_ms.saturating_sub(self.copy_started_ms) >= COPY_FAIL_GRACE_MS {
                    self.phase = Phase::Finished;
                    FlashStep::Failed(self.copy_error.take().unwrap_or_default())
                } else {
                    FlashStep::WaitingForReboot
                }
            }
            Phase::Rebooting => {
                if drive.is_none()
                    || now_ms.saturating_sub(self.copy_started_ms) >= REBOOT_TIMEOUT_MS
                {
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
        assert_eq!(
            half_status(None, "v0.2.0-23-g74ed094c"),
            HalfStatus::Unknown
        );
        assert_eq!(
            half_status(Some(""), "v0.2.0-23-g74ed094c"),
            HalfStatus::Unknown
        );
        assert_eq!(
            half_status(Some("v0.2.0-23-g74ed094c"), "v0.2.0-23-g74ed094c"),
            HalfStatus::UpToDate
        );
        assert_eq!(
            half_status(Some("v0.2.0-21-g4bf2fa45"), "v0.2.0-23-g74ed094c"),
            HalfStatus::UpdateAvailable
        );
    }

    #[test]
    fn only_an_nrf52840_uf2_drive_is_accepted() {
        assert!(is_nrf52840_uf2(
            "UF2 Bootloader 0.9.2 lib/nrfx\r\nModel: Nordic nRF52840 DK\r\nBoard-ID: nRF52840-pca10056-v1\r\n"
        ));
        assert!(!is_nrf52840_uf2(
            "UF2 Bootloader v3.0\r\nModel: Raspberry Pi RP2\r\nBoard-ID: RPI-RP2\r\n"
        ));
        assert!(!is_nrf52840_uf2(""));
    }

    #[test]
    fn differing_halves_are_flagged() {
        assert!(halves_differ(Some("a"), Some("b")));
        assert!(!halves_differ(Some("a"), Some("a")));
        assert!(
            !halves_differ(None, Some("a")),
            "unknown is not a difference"
        );
        assert_eq!(first_to_flash(Some("v23"), Some("v21"), "v23"), Side::Right);
        assert_eq!(first_to_flash(None, None, "v23"), Side::Left);
    }

    #[test]
    fn flash_machine_waits_copies_and_times_out() {
        let mut m = FlashMachine::new(Side::Left, 0, None);
        assert_eq!(m.step(500, None, None), FlashStep::WaitingForDrive);
        assert_eq!(
            m.step(1_000, Some(PathBuf::from(r"E:\")), None),
            FlashStep::Copying(PathBuf::from(r"E:\op36_left.uf2"))
        );
        assert_eq!(
            m.step(1_500, Some(PathBuf::from(r"E:\")), Some(Ok(()))),
            FlashStep::WaitingForReboot
        );
        assert_eq!(
            m.step(2_000, None, None),
            FlashStep::Done,
            "drive vanished = rebooted"
        );
        let mut m = FlashMachine::new(Side::Right, 0, None);
        assert_eq!(m.step(60_000, None, None), FlashStep::TimedOut);
        let mut m = FlashMachine::new(Side::Right, 0, None);
        m.step(100, Some(PathBuf::from(r"E:\")), None);
        // A copy error is held for the grace period: the bootloader may have rebooted mid-close.
        assert_eq!(
            m.step(200, Some(PathBuf::from(r"E:\")), Some(Err("denied".into()))),
            FlashStep::WaitingForReboot
        );
        assert_eq!(
            m.step(200 + COPY_FAIL_GRACE_MS, Some(PathBuf::from(r"E:\")), None),
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
        std::fs::write(
            dir.path().join("firmware/BUILD_COMMIT.txt"),
            "v0.2.0-23-g74ed094c\n",
        )
        .unwrap();
        let b = bundle_at(dir.path()).unwrap();
        assert_eq!(b.build, "v0.2.0-23-g74ed094c");
        assert_eq!(b.dir, dir.path().join("firmware"));
        assert_eq!(uf2_for(Side::Left), "op36_left.uf2");
        assert_eq!(uf2_for(Side::Right), "op36_right.uf2");
    }
}

#[cfg(test)]
mod minor_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_drive_already_mounted_at_the_click_is_not_flashed_until_it_reappears() {
        // The user may have double-tapped the other half first; writing the left firmware onto
        // a right half that happens to be in bootloader mode is the one thing Flash must not do.
        let e = PathBuf::from("E:\\");
        let mut m = FlashMachine::new(Side::Left, 0, Some(e.clone()));
        assert_eq!(
            m.step(500, Some(e.clone()), None),
            FlashStep::AlreadyMounted(e.clone())
        );
        assert_eq!(m.step(1000, None, None), FlashStep::WaitingForDrive);
        assert_eq!(
            m.step(1500, Some(e.clone()), None),
            FlashStep::Copying(e.join("op36_left.uf2"))
        );
    }

    #[test]
    fn a_copy_error_followed_by_the_drive_vanishing_is_a_successful_flash() {
        // UF2 bootloaders reboot as soon as the last block lands, often before Windows has
        // finished closing the file, so fs::copy can report an error after a good write.
        let e = PathBuf::from("E:\\");
        let mut m = FlashMachine::new(Side::Right, 0, None);
        assert_eq!(
            m.step(500, Some(e.clone()), None),
            FlashStep::Copying(e.join("op36_right.uf2"))
        );
        assert_eq!(
            m.step(1000, Some(e.clone()), Some(Err("device not ready".into()))),
            FlashStep::WaitingForReboot
        );
        assert_eq!(m.step(1500, None, None), FlashStep::Done);
    }

    #[test]
    fn a_copy_error_with_the_drive_still_there_is_a_failure() {
        let e = PathBuf::from("E:\\");
        let mut m = FlashMachine::new(Side::Right, 0, None);
        m.step(500, Some(e.clone()), None);
        assert_eq!(
            m.step(1000, Some(e.clone()), Some(Err("disk full".into()))),
            FlashStep::WaitingForReboot
        );
        assert_eq!(
            m.step(1000 + COPY_FAIL_GRACE_MS, Some(e.clone()), None),
            FlashStep::Failed("disk full".into())
        );
    }

    #[test]
    fn bundled_build_id_is_compared_at_the_length_the_firmware_can_report() {
        // The firmware carries at most FIRMWARE_BUILD_MAX bytes of `git describe`; a longer tag
        // comes back truncated and must still count as up to date.
        let long = "v0.10.0-123-gabcdef12-dirty-extra";
        assert!(long.len() > crate::vial::protocol::FIRMWARE_BUILD_MAX);
        let reported = &long[..crate::vial::protocol::FIRMWARE_BUILD_MAX];
        assert_eq!(half_status(Some(reported), long), HalfStatus::UpToDate);
        assert_eq!(
            half_status(Some("v0.10.0-122-g00000000"), long),
            HalfStatus::UpdateAvailable
        );
    }

    #[test]
    fn disconnected_keyboard_gets_no_flash_advice() {
        let w = describe(false, None, None, Some("v0.2.0-23-g74ed094c"));
        assert!(w.left.contains("not connected"), "{}", w.left);
        assert!(w.right.contains("not connected"), "{}", w.right);
        assert!(!w.note.to_lowercase().contains("flash"), "{}", w.note);
        let w = describe(true, None, None, Some("v0.2.0-23-g74ed094c"));
        assert!(w.left.contains("unknown"), "{}", w.left);
        assert!(w.note.contains("Flash the left half first"), "{}", w.note);
    }
}
