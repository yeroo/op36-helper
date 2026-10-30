//! The updater's worker: one release check 30 s after start and every 12 h while the setting is
//! on, plus "check now" and "update now" on request. Results reach the UI thread as boxed
//! `UpdateEvent`s (`WM_APP_UPDATE`). This is the only module that talks to the network.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::update::{
    API_URL, Channel, Release, detect_channel, parse_release, pick_asset, refuse_reason,
    updates_dir, verify_sha256,
};
use crate::version::{VERSION, is_newer};
use crate::win::msgwindow::WM_APP_UPDATE;

/// Test seam: a local file path here is read as the API response, and asset URLs that are local
/// paths are copied instead of downloaded (hermetic end-to-end runs, like agwinterm's).
pub const API_ENV: &str = "OP36_UPDATE_API";
const FIRST_CHECK: Duration = Duration::from_secs(30);
pub const CHECK_EVERY: Duration = Duration::from_secs(12 * 3600);

pub enum UpdateCmd {
    CheckNow,
    UpdateNow,
    /// The automatic check is on or off (from settings).
    AutoCheck(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateEvent {
    /// A newer release exists.
    Available(String),
    UpToDate,
    /// The check could not be made (offline, rate limit, odd document); quiet by design.
    CheckFailed,
    Downloading(String),
    /// Payload verified; the UI thread starts `helper` and exits.
    Verified {
        payload: PathBuf,
        helper: PathBuf,
        channel: Channel,
    },
    Failed(String),
}

#[derive(Clone, Copy)]
struct UiWindow(isize);
// SAFETY: an HWND is an opaque integer handle; PostMessageW is thread-safe.
unsafe impl Send for UiWindow {}

pub fn spawn(ui_hwnd: HWND, auto_check: bool) -> Sender<UpdateCmd> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ui = UiWindow(ui_hwnd.0 as isize);
    std::thread::Builder::new()
        .name("op36-update".into())
        .spawn(move || run(ui, rx, auto_check))
        .expect("spawn update thread");
    tx
}

fn run(ui: UiWindow, rx: Receiver<UpdateCmd>, mut auto_check: bool) {
    let mut next_check = Instant::now() + FIRST_CHECK;
    loop {
        let wait = if auto_check {
            next_check.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(3600)
        };
        match rx.recv_timeout(wait) {
            Err(RecvTimeoutError::Disconnected) => return,
            Ok(UpdateCmd::AutoCheck(on)) => auto_check = on,
            Ok(UpdateCmd::CheckNow) => post(ui, check()),
            Ok(UpdateCmd::UpdateNow) => update(ui),
            Err(RecvTimeoutError::Timeout) => {
                if auto_check && Instant::now() >= next_check {
                    post(ui, check());
                    next_check = Instant::now() + CHECK_EVERY;
                }
            }
        }
    }
}

fn check() -> UpdateEvent {
    match fetch_latest() {
        Some(r) if is_newer(&r.version, VERSION) => UpdateEvent::Available(r.version),
        Some(_) => UpdateEvent::UpToDate,
        None => UpdateEvent::CheckFailed,
    }
}

fn update(ui: UiWindow) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return post(
                ui,
                UpdateEvent::Failed(format!("cannot resolve the running exe: {e}")),
            );
        }
    };
    let local_app_data = dirs::data_local_dir().unwrap_or_default();
    let channel = detect_channel(&exe, &local_app_data);
    if channel == Channel::PackageManager {
        return post(
            ui,
            UpdateEvent::Failed(
                "this copy is managed by a package manager; update it there".into(),
            ),
        );
    }
    let Some(release) = fetch_latest() else {
        return post(
            ui,
            UpdateEvent::Failed(
                "update check failed (offline or rate-limited); try again later".into(),
            ),
        );
    };
    if !is_newer(&release.version, VERSION) {
        return post(ui, UpdateEvent::UpToDate);
    }
    let Some(asset) = pick_asset(&release, channel) else {
        return post(
            ui,
            UpdateEvent::Failed(format!(
                "release {} has no {} asset",
                release.version,
                channel.asset_kind()
            )),
        );
    };
    if let Some(reason) = refuse_reason(asset) {
        return post(
            ui,
            UpdateEvent::Failed(format!("refusing an unverifiable update: {reason}")),
        );
    }
    post(ui, UpdateEvent::Downloading(release.version.clone()));
    let dir = updates_dir();
    let payload = dir.join(&asset.name);
    if let Err(e) = download(&asset.url, &payload) {
        let _ = std::fs::remove_file(&payload);
        return post(ui, UpdateEvent::Failed(format!("download failed: {e}")));
    }
    if !verify_sha256(&payload, asset.sha256.as_deref().unwrap_or("")) {
        let _ = std::fs::remove_file(&payload);
        return post(
            ui,
            UpdateEvent::Failed(
                "download failed SHA-256 verification; nothing was installed".into(),
            ),
        );
    }
    let helper = dir.join("apply-update.ps1");
    if let Err(e) = std::fs::write(&helper, crate::update::apply_script()) {
        return post(
            ui,
            UpdateEvent::Failed(format!("cannot write the update helper: {e}")),
        );
    }
    post(
        ui,
        UpdateEvent::Verified {
            payload,
            helper,
            channel,
        },
    );
}

/// The latest release document, from the API or the `OP36_UPDATE_API` test seam.
fn fetch_latest() -> Option<Release> {
    let api = std::env::var(API_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| API_URL.to_string());
    let json = if !api.starts_with("http") && std::path::Path::new(&api).is_file() {
        std::fs::read_to_string(&api).ok()?
    } else {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .user_agent("op36-helper")
            .build();
        agent
            .get(&api)
            .set("Accept", "application/vnd.github+json")
            .call()
            .ok()?
            .into_string()
            .ok()?
    };
    parse_release(&json)
}

/// Download (or copy, for a local test path) `url` to `dest`.
fn download(url: &str, dest: &PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(dest.parent().ok_or("bad destination")?).map_err(|e| e.to_string())?;
    if !url.starts_with("http") && std::path::Path::new(url).is_file() {
        std::fs::copy(url, dest)
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(600))
            .user_agent("op36-helper")
            .build();
        let resp = agent.get(url).call().map_err(|e| e.to_string())?;
        let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
        std::io::copy(&mut resp.into_reader(), &mut file)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

fn post(ui: UiWindow, event: UpdateEvent) {
    let boxed = Box::new(event);
    // SAFETY: the UI thread takes ownership back with `take_event`.
    unsafe {
        let _ = PostMessageW(
            Some(HWND(ui.0 as *mut _)),
            WM_APP_UPDATE,
            WPARAM(0),
            LPARAM(Box::into_raw(boxed) as isize),
        );
    }
}

/// UI thread: reclaim an event posted by `post`.
pub fn take_event(lparam: LPARAM) -> Box<UpdateEvent> {
    // SAFETY: `lparam` is a pointer produced by `Box::into_raw` in `post`.
    unsafe { Box::from_raw(lparam.0 as *mut UpdateEvent) }
}
