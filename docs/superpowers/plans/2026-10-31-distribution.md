# Distribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the OP36 helper and its firmware from a fresh public repository: per-user installer, portable exe, tag-driven GitHub Releases with attestations, in-app update check and self-update, and firmware flashing from the Options dialog.

**Architecture:** Three new helper modules keep the risky parts pure and tested: `version` (one version string from Cargo), `firmware` (bundled files, bootloader-drive detection, flash state machine) and `update` (channel detection, release parsing, asset choice, digest check, apply script). Two worker threads do IO (drive polling and copying; HTTP) and report to the UI thread through the existing `WM_APP` handler window. The installer is Inno Setup per-user, built by a PowerShell script that the release workflow also runs; the firmware pair is built on Linux with the vendor fork's own CI recipe and attached to the release together with the installer, the portable exe and checksums. A one-off export script creates the public repository's initial commit without history or vendor binaries.

**Tech Stack:** Rust (windows 0.62, `ureq` 2 with `rustls` for HTTPS, `winresource` for the version resource), Inno Setup 6, PowerShell 7, GitHub Actions (`windows-latest`, `ubuntu-latest`), `actions/attest-build-provenance@v1`, `softprops/action-gh-release@v2`.

**Spec:** `docs/superpowers/specs/2026-10-31-distribution-design.md`

## Global Constraints

- Public repository name `yeroo/op36-helper` (Boris to confirm before Task 7; everything that embeds the name reads it from one constant/variable so a rename is one edit): helper `update::REPO = "yeroo/op36-helper"`, workflow `${{ github.repository }}`, README links.
- No `Co-Authored-By` or "Generated with" lines in commits, tags or release notes of the public repository (Boris's instruction, 2026-10-31).
- Per-user, admin-free: install dir `%LOCALAPPDATA%\Programs\OP36 Helper`, `PrivilegesRequired=lowest`; the only registry writes are Inno's own uninstall entries.
- Network only from `update.rs`: the release check (`https://api.github.com/repos/<REPO>/releases/latest`, headers `User-Agent: op36-helper`, `Accept: application/vnd.github+json`) and a user-requested download. Check failures are silent; update failures show a message.
- Fail closed on updates: no `digest` on the asset, digest mismatch, or any IO error → nothing applied, downloaded file deleted.
- Version: `helper/Cargo.toml` `version` is the single source; tags are `v<version>`; the release workflow fails on mismatch.
- Firmware build id: `git -C rmk describe --tags --always` (default abbrev, full-history checkout) both in the firmware (`RMK_FIRMWARE_BUILD`) and in the bundled `firmware/BUILD_COMMIT.txt`.
- Assets per release: `op36-helper-setup-<ver>.exe`, `op36-helper-portable-<ver>.exe`, `op36_left.uf2`, `op36_right.uf2`, `BUILD_COMMIT.txt`, `SHA256SUMS.txt`.
- Excluded from the public import: `1.png`–`5.png`, `firmware/op36-v0.2.0*`, `firmware/zmk-rollback`, `firmware/backup-zmk-*`, `firmware/settings-reset`, `_scratch`, `_ref`, `.superpowers`, `helper/target`, `.venv`.
- Settings file is never edited by hand; the new `update_check` field defaults to `true` and is read through `serde(default)` so old files load.

## Review Focus

1. A keyboard on firmware that predates the build-id query (bits absent, `E9 03` unanswered) must show "unknown — flash both halves" and never "update available". Task 2 test `unknown_reported_build_is_not_an_update`.
2. A removable drive that is a UF2 bootloader of a different board (INFO_UF2.TXT without `nRF52840`) or a plain USB stick must be ignored, not flashed. Task 2 test `only_an_nrf52840_uf2_drive_is_accepted`.
3. GitHub API unreachable, rate-limited (403 JSON body), or returning a release without assets: the check stays silent and the model keeps `update_available = None`. Task 3 test `malformed_or_empty_release_is_no_update`.
4. A release whose asset lacks a `digest` must be refused at update time with a visible message. Task 3 test `missing_digest_refuses_update`.
5. The two halves report different builds (one flashed, one not): Options says "halves differ" and the Flash flow proposes the lagging half first. Task 2 test `differing_halves_are_flagged`.

---

### Task 1: One version everywhere

**Files:**
- Modify: `helper/Cargo.toml` (`version = "0.2.0"`, add `winresource = "0.1"` to `[build-dependencies]`)
- Create: `helper/build.rs` (version resource with `FileVersion`/`ProductVersion` from `CARGO_PKG_VERSION`, icon `assets/icon_on.ico`)
- Create: `helper/src/version.rs`
- Modify: `helper/src/main.rs` (`--version` CLI arm; Options status text gains `Helper: X.Y.Z`)
- Test: `helper/src/version.rs`

**Interfaces:**
- Produces: `pub const VERSION: &str = env!("CARGO_PKG_VERSION");` `pub fn version_line() -> String` → `"op36-helper 0.2.0"`; `pub fn parse_semver(s: &str) -> Option<(u64, u64, u64)>` accepting an optional leading `v`; `pub fn is_newer(candidate: &str, current: &str) -> bool`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn version_line_names_the_cargo_version() { assert_eq!(version_line(), format!("op36-helper {}", env!("CARGO_PKG_VERSION"))); }
#[test] fn semver_parses_with_or_without_v() {
    assert_eq!(parse_semver("v0.2.1"), Some((0, 2, 1)));
    assert_eq!(parse_semver("0.2.1"), Some((0, 2, 1)));
    assert_eq!(parse_semver("0.2"), None);
    assert_eq!(parse_semver("0.2.1-rc1"), None, "prereleases are not update targets");
}
#[test] fn newer_compares_numerically() {
    assert!(is_newer("v0.10.0", "0.9.9"));
    assert!(!is_newer("v0.2.0", "0.2.0"));
    assert!(!is_newer("garbage", "0.2.0"));
}
```
- [ ] **Step 2: Run to verify they fail** — `cd helper && cargo test version` → compile error (module missing).
- [ ] **Step 3: Implement `version.rs`, `build.rs`, the `--version` arm (prints `version_line()`), the status text line, bump Cargo to 0.2.0.**
- [ ] **Step 4: Run `cargo test` and `cargo build --release`; check the resource**: `powershell -c "(Get-Item helper/target/release/op36-helper.exe).VersionInfo.FileVersion"` → `0.2.0.0`.
- [ ] **Step 5: Commit** — `git add helper && git commit -m "feat(helper): one version from Cargo, --version and a version resource"`

---

### Task 2: Firmware bundle, bootloader drive detection and flashing from Options

**Files:**
- Create: `helper/src/firmware.rs` (pure) and `helper/src/firmware_thread.rs` (worker)
- Modify: `helper/src/model.rs` (`firmware_status()`), `helper/src/main.rs` (`WM_APP_FLASH` events, `App::start_flash(side)`, `App::cancel_flash`), `helper/src/options/dialog.rs` (Firmware group), `helper/src/win/msgwindow.rs` (`WM_APP_FLASH = WM_APP + 6`)
- Test: `helper/src/firmware.rs`

**Interfaces:**
- Consumes: `Model.firmware: [Option<String>; 2]` (Task 1 of the dynamic-master helper plan), `Side`.
- Produces (pure):
  ```rust
  pub struct Bundle { pub dir: PathBuf, pub build: String }                 // <exe dir>\firmware, BUILD_COMMIT.txt trimmed
  pub fn bundle_at(exe_dir: &Path) -> Option<Bundle>;                      // None when files are missing
  pub fn uf2_for(side: Side) -> &'static str;                              // "op36_left.uf2" / "op36_right.uf2"
  pub fn is_nrf52840_uf2(info_uf2_txt: &str) -> bool;                      // contains "nRF52840" (any case)
  #[derive(PartialEq, Debug)] pub enum HalfStatus { UpToDate, UpdateAvailable, Unknown }
  pub fn half_status(reported: Option<&str>, bundled: &str) -> HalfStatus;
  pub fn halves_differ(left: Option<&str>, right: Option<&str>) -> bool;   // both known and different
  pub fn first_to_flash(left: Option<&str>, right: Option<&str>, bundled: &str) -> Side; // the lagging half, else Left
  #[derive(Clone, Debug, PartialEq)] pub enum FlashStep { WaitingForDrive, Copying(PathBuf), WaitingForReboot, Done, TimedOut, Cancelled, Failed(String) }
  pub struct FlashMachine { .. }  // new(side, started_ms); step(now_ms, drive: Option<PathBuf>, copy_result: Option<Result<(), String>>) -> FlashStep; timeout 60 s waiting for the drive, 10 s for the reboot
  ```
- Produces (worker): `firmware_thread::spawn(ui: UiWindow, side: Side, bundle: Bundle) -> Sender<()> /* cancel */`; polls `GetLogicalDrives` + `GetDriveTypeW == DRIVE_REMOVABLE` every 500 ms, reads `<drive>\INFO_UF2.TXT`, copies the UF2, waits for the drive to vanish, posts `HidEvent`-style boxed `FlashStep` through `WM_APP_FLASH`.
- Produces (UI): Options "Firmware" group: `Left: v… (up to date | update available | unknown)`, `Right: …`, `Bundled: v…`, `Flash left half…`, `Flash right half…`, a status line showing the `FlashStep` text ("Double-tap reset on the left half; waiting for its drive…", "Copying…", "Flashed; the half reconnects in a few seconds", "No drive appeared within 60 s"), `Cancel` while waiting; buttons disabled during a flash. "Halves differ" shown in the status line when applicable.

- [ ] **Step 1: Write the failing tests** (`firmware.rs`):

```rust
#[test] fn unknown_reported_build_is_not_an_update() {
    assert_eq!(half_status(None, "v0.2.0-23-g74ed094c"), HalfStatus::Unknown);
    assert_eq!(half_status(Some(""), "v0.2.0-23-g74ed094c"), HalfStatus::Unknown);
    assert_eq!(half_status(Some("v0.2.0-23-g74ed094c"), "v0.2.0-23-g74ed094c"), HalfStatus::UpToDate);
    assert_eq!(half_status(Some("v0.2.0-21-g4bf2fa45"), "v0.2.0-23-g74ed094c"), HalfStatus::UpdateAvailable);
}
#[test] fn only_an_nrf52840_uf2_drive_is_accepted() {
    assert!(is_nrf52840_uf2("UF2 Bootloader 0.9.2 lib/nrfx\r\nModel: Nordic nRF52840 DK\r\nBoard-ID: nRF52840-pca10056-v1\r\n"));
    assert!(!is_nrf52840_uf2("UF2 Bootloader v3.0\r\nModel: Raspberry Pi RP2\r\nBoard-ID: RPI-RP2\r\n"));
    assert!(!is_nrf52840_uf2(""));
}
#[test] fn differing_halves_are_flagged() {
    assert!(halves_differ(Some("a"), Some("b")));
    assert!(!halves_differ(Some("a"), Some("a")));
    assert!(!halves_differ(None, Some("a")), "unknown is not a difference");
    assert_eq!(first_to_flash(Some("v23"), Some("v21"), "v23"), Side::Right);
    assert_eq!(first_to_flash(None, None, "v23"), Side::Left);
}
#[test] fn flash_machine_waits_copies_and_times_out() {
    let mut m = FlashMachine::new(Side::Left, 0);
    assert_eq!(m.step(500, None, None), FlashStep::WaitingForDrive);
    assert_eq!(m.step(1_000, Some(PathBuf::from("E:\\")), None), FlashStep::Copying(PathBuf::from("E:\\op36_left.uf2")));
    assert_eq!(m.step(1_500, Some(PathBuf::from("E:\\")), Some(Ok(()))), FlashStep::WaitingForReboot);
    assert_eq!(m.step(2_000, None, None), FlashStep::Done, "drive vanished = rebooted");
    let mut m = FlashMachine::new(Side::Right, 0);
    assert_eq!(m.step(60_000, None, None), FlashStep::TimedOut);
}
#[test] fn bundle_reads_build_commit_next_to_the_exe() {
    let dir = tempfile::tempdir().unwrap();
    assert!(bundle_at(dir.path()).is_none());
    std::fs::create_dir(dir.path().join("firmware")).unwrap();
    for f in ["op36_left.uf2", "op36_right.uf2"] { std::fs::write(dir.path().join("firmware").join(f), b"x").unwrap(); }
    std::fs::write(dir.path().join("firmware/BUILD_COMMIT.txt"), "v0.2.0-23-g74ed094c\n").unwrap();
    assert_eq!(bundle_at(dir.path()).unwrap().build, "v0.2.0-23-g74ed094c");
}
```
- [ ] **Step 2: Run to verify they fail** — `cargo test firmware::` → compile errors.
- [ ] **Step 3: Implement `firmware.rs`, then `firmware_thread.rs`, the `WM_APP_FLASH` plumbing in `main.rs`, and the Options group.** The dialog grows one group (move OK/Cancel down; keep one page). `App` holds `flash: Option<(Side, Sender<()>)>`; the dialog's status line is refreshed through a new `OptionsDialog::set_firmware_status(&str)` on each `FlashStep`.
- [ ] **Step 4: Run `cargo test`; then a live check with the keyboard**: Options → Flash right half → double-tap reset → the file is copied from `<exe dir>\firmware` (copy the current UF2 pair + `BUILD_COMMIT.txt` there for the test) → "Flashed" → the half reconnects; Options shows both halves up to date.
- [ ] **Step 5: Commit** — `git add helper && git commit -m "feat(helper): flash a half from Options; per-half firmware status"`

---

### Task 3: Update check and self-update

**Files:**
- Create: `helper/src/update.rs` (pure) and `helper/src/update_thread.rs` (HTTP worker)
- Modify: `helper/Cargo.toml` (`ureq = { version = "2", default-features = false, features = ["tls", "json"] }`), `helper/src/settings.rs` (`update_check: bool`, `#[serde(default = "default_true")]`), `helper/src/model.rs` (`update_available: Option<String>`), `helper/src/main.rs` (`WM_APP_UPDATE`, balloon once per version, `App::update_now`, exit path), `helper/src/tray.rs` (`balloon(title, text)` via `NIF_INFO`), `helper/src/options/dialog.rs` ("Updates" controls: checkbox, status line, `Update now`), `helper/src/win/msgwindow.rs` (`WM_APP_UPDATE = WM_APP + 7`), `helper/assets/apply-update.ps1` (embedded with `include_str!`)
- Test: `helper/src/update.rs`

**Interfaces:**
- Consumes: `version::{VERSION, is_newer}` (Task 1).
- Produces:
  ```rust
  pub const REPO: &str = "yeroo/op36-helper";
  pub const API_URL: &str = "https://api.github.com/repos/yeroo/op36-helper/releases/latest";
  pub const CHECK_EVERY: Duration = Duration::from_secs(12 * 3600);  // first check 30 s after start
  #[derive(PartialEq, Debug)] pub enum Channel { Installed, Portable, PackageManager }
  pub fn detect_channel(exe: &Path, local_app_data: &Path) -> Channel;   // Programs\OP36 Helper → Installed; \scoop\apps\ → PackageManager; else Portable
  pub struct Asset { pub name: String, pub url: String, pub sha256: Option<String> }
  pub struct Release { pub version: String, pub assets: Vec<Asset> }
  pub fn parse_release(json: &str) -> Option<Release>;                   // tag_name + assets[].{name, browser_download_url, digest "sha256:…"}
  pub fn pick_asset<'a>(r: &'a Release, ch: Channel) -> Option<&'a Asset>; // setup for Installed, portable for Portable, None otherwise
  pub fn verify_sha256(path: &Path, expected_hex: &str) -> bool;
  pub fn apply_script() -> &'static str;                                 // include_str!("../assets/apply-update.ps1")
  pub fn updates_dir() -> PathBuf;                                       // %LOCALAPPDATA%\op36-helper\updates
  pub fn cleanup_leftovers(exe: &Path);                                  // delete exe.old and the updates dir, best effort
  ```
  Worker: `update_thread::spawn(ui) -> Sender<UpdateCmd>` with `UpdateCmd::{CheckNow, UpdateNow}`; it posts `UpdateEvent::{Available(version), UpToDate, CheckFailed, Downloading, Verified(payload_path, helper_path), Failed(String)}`. On `Verified` the UI thread starts `powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File <helper> -ProcId <pid> -Channel installed|portable -Payload <file> -Exe <exe>`, saves settings, drops tray and app bar, exits. The script is agwinterm's helper adapted (wait for pid, `/VERYSILENT /NORESTART /SUPPRESSMSGBOXES` or rename-swap, relaunch).
  SHA-256 is computed with the `sha2` crate (add to Cargo).

- [ ] **Step 1: Write the failing tests** (`update.rs`):

```rust
const SAMPLE: &str = r#"{"tag_name":"v0.2.1","assets":[{"name":"op36-helper-setup-0.2.1.exe","browser_download_url":"https://x/setup.exe","digest":"sha256:ab12"},{"name":"op36-helper-portable-0.2.1.exe","browser_download_url":"https://x/portable.exe"}]}"#;
#[test] fn release_parses_version_assets_and_digests() {
    let r = parse_release(SAMPLE).unwrap();
    assert_eq!(r.version, "0.2.1");
    assert_eq!(pick_asset(&r, Channel::Installed).unwrap().sha256.as_deref(), Some("ab12"));
    assert_eq!(pick_asset(&r, Channel::Portable).unwrap().sha256, None);
    assert!(pick_asset(&r, Channel::PackageManager).is_none());
}
#[test] fn malformed_or_empty_release_is_no_update() {
    assert!(parse_release(r#"{"message":"API rate limit exceeded"}"#).is_none());
    assert!(parse_release("not json").is_none());
    let r = parse_release(r#"{"tag_name":"v9.9.9","assets":[]}"#).unwrap();
    assert!(pick_asset(&r, Channel::Installed).is_none());
}
#[test] fn missing_digest_refuses_update() {
    let r = parse_release(SAMPLE).unwrap();
    let portable = pick_asset(&r, Channel::Portable).unwrap();
    assert!(portable.sha256.is_none(), "the worker must refuse this asset");
}
#[test] fn channel_detection_from_exe_path() {
    let lad = Path::new(r"C:\Users\b\AppData\Local");
    assert_eq!(detect_channel(Path::new(r"C:\Users\b\AppData\Local\Programs\OP36 Helper\op36-helper.exe"), lad), Channel::Installed);
    assert_eq!(detect_channel(Path::new(r"C:\Users\b\scoop\apps\op36-helper\current\op36-helper.exe"), lad), Channel::PackageManager);
    assert_eq!(detect_channel(Path::new(r"D:\tools\op36-helper.exe"), lad), Channel::Portable);
}
#[test] fn sha256_verification_fails_closed() {
    let f = tempfile::NamedTempFile::new().unwrap(); std::fs::write(f.path(), b"abc").unwrap();
    assert!(verify_sha256(f.path(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    assert!(!verify_sha256(f.path(), "00"));
    assert!(!verify_sha256(Path::new("Z:\\missing"), "00"));
}
```
- [ ] **Step 2: Run to verify they fail** — `cargo test update::` → compile errors.
- [ ] **Step 3: Implement the pure module, the worker, the setting, the balloon, the Options controls and the exit path.** The first check runs 30 s after start on the worker's own timer; `settings.update_check == false` suspends checks (the worker still serves `UpdateNow`). Balloon text: `OP36 Helper 0.2.1 is out (you have 0.2.0) — Options → Update now`.
- [ ] **Step 4: Run `cargo test`; dry-run with the test seam** `OP36_UPDATE_API=<path to a local JSON file>` (local paths as `browser_download_url` are copied instead of downloaded, like agwinterm): point it at a copy of the current exe with its real digest → the helper exits, the script swaps, the helper relaunches; then with a wrong digest → "download failed SHA-256 verification", nothing applied.
- [ ] **Step 5: Commit** — `git add helper && git commit -m "feat(helper): update check against GitHub releases and verified self-update"`

---

### Task 4: Installer and portable build

**Files:**
- Create: `installer/op36-helper.iss`, `installer/build.ps1`, `installer/build-portable.ps1`, `installer/README.md`
- Modify: `.gitignore` (`installer/Output/`, `installer/stage/`)

**Interfaces:**
- Produces: `installer/build.ps1 [-Version <auto from Cargo.toml>] [-FirmwareDir <path>]` → stages `op36-helper.exe`, `assets\*.ico`, `firmware\{op36_left.uf2,op36_right.uf2,BUILD_COMMIT.txt}`, `LICENSE`, `README.md`; fails if any is missing; runs ISCC; writes `installer/Output/op36-helper-setup-<ver>.exe`. `build-portable.ps1` → `installer/Output/op36-helper-portable-<ver>.exe`. The `.iss` takes `/DAppVersion=<ver>` so the script, not the file, owns the version.
- Inno: `AppId={{7D2C6E1A-4B8F-4F2E-9C3A-OP36HELPER01}` (a fixed GUID generated once), `DefaultDirName={localappdata}\Programs\OP36 Helper`, `PrivilegesRequired=lowest`, `CloseApplications=yes`, `[Tasks]` `desktopicon` (unchecked), `startup` ("Start OP36 Helper with Windows", checked) creating `{userstartup}\OP36 Helper.lnk`, `[Run]` post-install launch `skipifsilent`, `[UninstallDelete]` `{app}`.

- [ ] **Step 1: Write the scripts and the `.iss`; build locally**

Run: `pwsh installer/build.ps1 -FirmwareDir firmware/op36-layer-notify` then `pwsh installer/build-portable.ps1`
Expected: `installer/Output/op36-helper-setup-0.2.0.exe` and `op36-helper-portable-0.2.0.exe` exist; setup size > 2 MB (firmware included).

- [ ] **Step 2: Install it on this machine** (`/SILENT`), confirm `%LOCALAPPDATA%\Programs\OP36 Helper\firmware\BUILD_COMMIT.txt` exists, the Start Menu shortcut works, Options shows `Helper: 0.2.0`, channel Installed, Firmware group reads the bundle; the Startup checkbox in Options reflects the task. Then uninstall and confirm `%APPDATA%\op36-helper` survives.
- [ ] **Step 3: Commit** — `git add installer .gitignore && git commit -m "build: per-user Inno Setup installer and portable exe"`

---

### Task 5: Public-facing repository content

**Files:**
- Create: `LICENSE` (MIT, Boris Kudriashov, 2026), `README.md` (root, user-facing), `docs/img/overlay.png` (capture of the helper's overlay, light or dark), `tools/export-public.ps1`
- Modify: `helper/README.md` (developer notes only; link to root), `firmware/FLASHING.md` (becomes the manual fallback; stock firmware links point to Ergohaven's downloads instead of files in this repo), `.gitignore`

**Interfaces:**
- Produces: `tools/export-public.ps1 -Dest <dir>` copies the working tree minus the Global Constraints exclusion list and `.git`, keeps `.gitmodules` and the `rmk` submodule pointer, and prints the file count. Root README sections: What it is · Screenshot · Install (setup / portable, SmartScreen note) · First run · Flashing the firmware from Options (both halves, left then right) · Migrating from the vendor firmware (manual, link to FLASHING.md) · Layout (Entropy/Vial; Boris's `--write-keymap-zmk` is a developer tool) · Updates · Settings and logs locations · Build from source (`git clone --recurse-submodules`, Rust, Inno) · License.

- [ ] **Step 1: Write LICENSE, README, capture `docs/img/overlay.png` (`--snapshot` at scale 1.0, EN, layer 0), update the two existing docs, write the export script.**
- [ ] **Step 2: Dry-run the export**: `pwsh tools/export-public.ps1 -Dest $env:TEMP\op36-public` then `ls` → no `*.png` at the root, no `firmware/op36-v0.2.0*`, `rmk` present as an empty dir with `.gitmodules` referencing it.
- [ ] **Step 3: Commit** — `git add LICENSE README.md docs tools helper/README.md firmware/FLASHING.md .gitignore && git commit -m "docs: public README, MIT license, export script"`

---

### Task 6: CI and release workflows

**Files:**
- Create: `.github/workflows/ci.yml`, `.github/workflows/release.yml`

**Interfaces:**
- `ci.yml` on push/PR: job `helper` (windows-latest: checkout with submodules, `dtolnay/rust-toolchain@stable`, `cargo test` in `helper/`), job `firmware-host-tests` (ubuntu-latest: `cargo nextest` in `rmk/rmk` with `--target x86_64-unknown-linux-gnu --no-default-features --features=split,vial,storage,async_matrix,_ble,layer_notify`; skip if the suite needs Windows-only pieces — note the outcome in the ledger).
- `release.yml` on `v*` tags and `workflow_dispatch`: job `firmware` (ubuntu-latest: the fork's toolchain steps copied verbatim — `gcc-arm-none-eabi binutils-arm-none-eabi libnewlib-arm-none-eabi clang libclang-dev`, Rust with `thumbv7em-none-eabihf`, `flip-link`, `uf2conv.py`; `BINDGEN_EXTRA_CLANG_ARGS` for the Debian layout; build `central`/`peripheral` in `rmk/keyboards/op36`; objcopy + uf2conv with `--base 0x26000 --family 0xADA52840`; `git -C rmk describe --tags --always > BUILD_COMMIT.txt` from a full-history submodule checkout; upload artifact `firmware`), job `release` (windows-latest, needs firmware: checkout with submodules, version check `cargo metadata` vs tag, `cargo test`, download artifact to `installer/firmware`, `choco install innosetup -y --no-progress`, `installer/build.ps1 -FirmwareDir installer/firmware`, `build-portable.ps1`, copy the three firmware files into `installer/Output`, `SHA256SUMS.txt` via `Get-FileHash`, `actions/attest-build-provenance@v1` on all assets, `softprops/action-gh-release@v2` with `generate_release_notes: true`, `fail_on_unmatched_files: true`). Permissions `contents: write`, `id-token: write`, `attestations: write`.

- [ ] **Step 1: Write both workflows; validate the YAML** — `python -c "import yaml,sys; [yaml.safe_load(open(f)) for f in sys.argv[1:]]" .github/workflows/*.yml` → no error.
- [ ] **Step 2: Commit** — `git add .github && git commit -m "ci: test workflow and tag-driven release with firmware, installer and attestations"`

---

### Task 7: Create the public repository and the first release

- [ ] **Step 1: Confirm the name with Boris** (`op36-helper` unless he says otherwise), then `gh repo create yeroo/op36-helper --public --description "Windows overlay and dual-role firmware for the Ergohaven OmegaPoint36" --license mit` is NOT used (we ship our own LICENSE): create it empty with `gh repo create yeroo/op36-helper --public --description "..."`.
- [ ] **Step 2: Export and push**: `pwsh tools/export-public.ps1 -Dest ..\op36-helper`; in that dir `git init -b main`, `git add -A`, `git commit -m "OP36 helper 0.2.0: overlay, dual-role firmware, installer"` (no attribution lines), `git remote add origin https://github.com/yeroo/op36-helper.git`, `git push -u origin main`; `git submodule update --init` must work in a fresh clone (`gh repo clone yeroo/op36-helper -- --recurse-submodules` into `%TEMP%`).
- [ ] **Step 3: Tag and watch**: `git tag v0.2.0 && git push origin v0.2.0`; `gh run watch` until green; `gh release view v0.2.0` lists the six assets; `gh attestation verify op36-helper-setup-0.2.0.exe --repo yeroo/op36-helper` passes on a downloaded copy; `SHA256SUMS.txt` matches.
- [ ] **Step 4: Record the first release in `docs/superpowers/plans/2026-10-31-distribution-acceptance.md` (private repo) and point the private repo's README at the public one.**

---

### Task 8: Acceptance on this machine

- [ ] **Step 1 (spec 1, 7)**: install `op36-helper-setup-0.2.0.exe` from the release page (SmartScreen → Run anyway), Start with Windows checked → tray icon, overlay, Options shows versions; uninstall keeps `%APPDATA%\op36-helper`; reinstall.
- [ ] **Step 2 (spec 2)**: Flash left then right from Options with the release's firmware; both halves report the bundled build.
- [ ] **Step 3 (spec 4)**: bump Cargo to 0.2.1 in the public repo (a doc-only change is enough), tag `v0.2.1`; after the release is live, restart the helper → balloon within 30 s → Update now → 0.2.1 running; repeat with the portable exe.
- [ ] **Step 4 (spec 5, 6)**: disconnect the network → check silent, Update now shows the failure; test seam with a corrupted digest → refused.
- [ ] **Step 5**: record results; commit the acceptance file in the private repo.
