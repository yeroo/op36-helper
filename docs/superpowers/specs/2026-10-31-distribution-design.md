# OP36 helper distribution: public repository, installer, releases, in-app updates — design

Status: approved by Boris 2026-10-31 (fresh public repository, name to confirm). Follows the agwinterm approach (github.com/yeroo/agwinterm:
`installer/agwinterm.iss`, `src/Agwinterm.Pty/AppUpdate.cs`, `.github/workflows/release.yml`),
adapted to a Rust tray app that also ships keyboard firmware.

## Intent

Boris wants the helper and the dual-role firmware to be installable by anyone with an Ergohaven
OmegaPoint36, to stop depending on this laptop's toolchain and drive letters, and to get updates
the way agwinterm users do: the app tells you a new version is out, you click, it updates and
comes back. Firmware updates belong to the same flow: the helper carries the firmware it was built
against and flashes a half for you when you put it into its bootloader.

## Goals

1. A **fresh public repository** (`github.com/yeroo/op36-helper`, name to confirm) under MIT holds
   the helper, docs, release pipeline and the firmware submodule, created from the current tree as
   one initial commit: no history, no vendor screenshots, no stock or backup firmware files. The
   private `yeroo/omega36` stays as the archive. The firmware fork `yeroo/rmk` stays Apache-2.0. A
   user-facing README explains install, flash, layout.
2. Tagging `vX.Y.Z` on `main` builds, attests and publishes a GitHub Release with:
   `op36-helper-setup-X.Y.Z.exe` (per-user Inno installer, bundles the firmware),
   `op36-helper-portable-X.Y.Z.exe` (the bare exe), `op36_left.uf2`, `op36_right.uf2`,
   `SHA256SUMS.txt`, and auto-generated release notes.
3. The helper checks GitHub for a newer release once at start-up and every 12 h (a setting,
   default on), tells the user through a tray balloon and the Options dialog, and updates itself on
   request: verified download, silent re-install, relaunch. Portable copies swap the exe in place.
4. Options gets a Firmware section: the build each half reports, the bundled build, a Flash button
   per half that waits for the bootloader drive and copies the right file, and an "update
   available" mark when a half reports an older build than the bundled one.
5. Everything stays per-user and admin-free; the only registry writes are the ones Inno Setup makes
   for its own uninstaller under the user's hive.

## Non-goals

- Code signing (Authenticode). SmartScreen's "unknown publisher" warning on first run is accepted
  and documented; releases carry Sigstore build-provenance attestations like agwinterm.
- Package managers (scoop, winget, Chocolatey). Agwinterm's channel detection is kept so a later
  scoop bucket needs no updater change, but nothing is submitted now.
- Automatic installs. The update is always applied on the user's click, never in the background.
- Writing a layout for strangers. `--write-keymap-zmk` is Boris's layout; others configure the
  keyboard with Entropy or Vial as the vendor documents.
- Walking a newcomer through the first migration from the vendor's ZMK or stock RMK (Bluetooth
  re-pairing, split pairing). The README documents it once by hand per `firmware/FLASHING.md`; the
  Flash button serves any half already in its bootloader.
- Mac/Linux.

## Anti-goals

- A failed or tampered download must never be installed: no digest, digest mismatch, any IO error →
  abort with a visible message, nothing applied.
- Flashing the wrong half. The Flash button names the half and the user confirms; the helper must
  not pick a half by itself.
- Losing the user's settings or layout on update: settings stay in `%APPDATA%\op36-helper`; the
  layout lives in the keyboard; a firmware flash never touches it.
- Network traffic beyond the release check and the download the user asked for.

## Constraints and facts

- Helper: Rust, single exe, `windows` crate; Cargo version is 0.1.0 and must become the release
  version (`Cargo.toml` is the single source; the tag must match or the release fails).
- Inno Setup 6 is installed locally (`iscc` on PATH via scoop) and installable on `windows-latest`
  with `choco install innosetup`.
- Firmware builds need the ARM toolchain and nrf-sdc bindgen flags; the fork's own CI
  (`yeroo/rmk` `.github/workflows/build.yml`, matrix entry `op36`) already does this on
  `ubuntu-latest` with `BINDGEN_EXTRA_CLANG_ARGS` for the Debian `gcc-arm-none-eabi` 13.2.1 layout.
- The firmware source is the submodule `rmk/` pinned to the released commit; the helper's
  `rmk-types` path dependency points into it, so the release checkout needs `submodules: true`.
- The firmware reports each half's build id over Vial (`08 E9 03 side`) as `git describe` of the
  submodule; the helper already shows it in Options.
- Bootloader: double-tap reset → `NRF52BOOT` FAT drive with `INFO_UF2.TXT` containing
  `Board-ID: nRF52840`; copying the UF2 reboots the half and the drive disappears.
- The helper is offline today by rule; this design opens exactly two network paths (release check,
  user-requested download) and makes the check a setting.
- GitHub release assets carry a `digest: "sha256:…"` field in the API; it is the integrity gate.

## Design

### Repository and licensing

- `LICENSE` (MIT, Boris Kudriashov) at the root; a note that `rmk/` is Apache-2.0 upstream.
- Root `README.md` for users: what it is, screenshot, install (setup or portable), first run
  (SmartScreen note), flashing both halves from Options, writing a layout (Entropy/Vial), how
  updates work, where settings and logs live, building from source (`--recurse-submodules`).
  `helper/README.md` keeps the developer notes; `firmware/FLASHING.md` keeps the manual procedure
  and becomes the "if the Flash button does not work" fallback.
- The README screenshot is a capture of the helper's own overlay (`docs/img/overlay.png`); vendor
  screenshots are never redistributed.
- The public repository is created first (clean import), then the release tag is pushed there, so
  the first release already exercises the update check and public attestations. Left out of the
  import: `1.png`–`5.png`, `firmware/op36-v0.2.0*`, `firmware/zmk-rollback`, `firmware/backup-zmk-*`,
  `firmware/settings-reset` (vendor and personal binaries; the README links to the vendor's downloads
  for the stock firmware), `_scratch`, `_ref`, `.superpowers`. The update check and the release
  workflow use the new repository's name everywhere.

### Versioning

- One version for the whole release: `helper/Cargo.toml` `version` = tag without `v`. The release
  workflow fails if they differ.
- The helper exposes `env!("CARGO_PKG_VERSION")` as `--version` output and in Options
  ("Helper: X.Y.Z").
- Firmware build id stays `git describe` of the submodule; the installer ships
  `firmware/BUILD_COMMIT.txt` next to the UF2s so the helper knows its bundled id without a build
  dependency.

### Installer (`installer/op36-helper.iss`, `installer/build.ps1`)

- Per-user: `PrivilegesRequired=lowest`, `DefaultDirName={localappdata}\Programs\OP36 Helper`,
  `UninstallDisplayIcon`, `ArchitecturesInstallIn64BitMode=x64compatible`, lzma2.
- Files: `op36-helper.exe`, `assets\*.ico`, `firmware\op36_left.uf2`, `firmware\op36_right.uf2`,
  `firmware\BUILD_COMMIT.txt`, `LICENSE`, `README.md`.
- Tasks: Start Menu shortcut (always), optional desktop shortcut, optional **Start with Windows**
  (creates `{userstartup}\OP36 Helper.lnk`, the very shortcut the helper's own autostart setting
  manages, so the Options checkbox reflects it).
- `[Run]` launch on finish (skipped for silent installs). Silent re-install is what the updater
  uses: `/VERYSILENT /NORESTART /SUPPRESSMSGBOXES`.
- Before installing files, setup closes a running helper (Inno `CloseApplications`); the updater
  never relies on it because the helper exits itself first.
- Uninstall removes files and shortcuts; settings and the keymap cache in `%APPDATA%\op36-helper`
  are left (documented).
- `build.ps1`: `cargo build --release` for the helper, stage files, verify the firmware pair and
  `BUILD_COMMIT.txt` are present, run ISCC; output `installer/Output/op36-helper-setup-<ver>.exe`.
  `build-portable.ps1` copies the exe to `op36-helper-portable-<ver>.exe`.

### Release workflow (`.github/workflows/release.yml`)

Triggered by tags `v*`, `permissions: contents: write, id-token: write, attestations: write`.

1. `firmware` job (ubuntu-latest): checkout with submodules, the fork CI's toolchain steps, build
   `central`/`peripheral` of `rmk/keyboards/op36`, objcopy + uf2conv → `op36_left.uf2`,
   `op36_right.uf2`, `BUILD_COMMIT.txt` (`git -C rmk describe --tags --always`), upload artifact.
2. `installer` job (windows-latest, needs firmware): checkout with submodules, Rust stable,
   `cargo test` in `helper/`, version check (Cargo.toml == tag), download the firmware artifact into
   `installer/firmware/`, `choco install innosetup`, `installer/build.ps1`, `build-portable.ps1`,
   `SHA256SUMS.txt` over all assets, `actions/attest-build-provenance@v1` on the assets,
   `softprops/action-gh-release@v2` with `generate_release_notes: true` and
   `fail_on_unmatched_files: true`.
3. `ci.yml` (push and pull request): `cargo test` for the helper and the firmware host suite
   (`cargo nextest` with the project's feature set) so `main` stays releasable.

### In-app update (helper)

Module `helper/src/update.rs` (pure parts tested) plus a worker thread:

- `Channel::detect(exe_path, local_app_data)`: `Installed` when the exe is under
  `%LOCALAPPDATA%\Programs\OP36 Helper\`, `PackageManager` under `\scoop\apps\` (future),
  else `Portable`.
- `ReleaseInfo::parse(json) -> Option<ReleaseInfo { version, assets: [{name, url, sha256}] }>`
  from `https://api.github.com/repos/yeroo/omega36/releases/latest`, digest `sha256:` only; any
  shape mismatch → `None`. Version comparison is semver on the tag.
- Asset selection: `op36-helper-setup-*.exe` for Installed, `op36-helper-portable-*.exe` for
  Portable, none for PackageManager.
- Check: once ~30 s after start and every 12 h while `settings.update_check` is on; result into
  the model (`update_available: Option<String>`), shown as a tray balloon once per version and a
  line in Options ("Update: X.Y.Z available — Update now").
- Update now: download to `%LOCALAPPDATA%\op36-helper\updates\`, verify SHA-256 (fail closed),
  write `apply-update.ps1` (agwinterm's helper script adapted: wait for our PID, run setup silently
  or rename-swap the portable exe, relaunch the same path), start it hidden, then the helper saves
  settings, removes its tray icon and app bar and exits. Start-up housekeeping deletes `.old`
  leftovers and the updates folder.
- Transport: `windows` crate WinHTTP or the `ureq` crate with rustls; the choice is the builder's
  (offline failures must be silent at check time and a visible message at update time).
- Options gains two controls: "Check for updates automatically" (bool) and the Update now button
  with its status line.

### Firmware flashing from the helper

- Bundled files resolve relative to the exe: `<exe dir>\firmware\op36_{left,right}.uf2` and
  `BUILD_COMMIT.txt`. A portable exe without the folder shows "no bundled firmware".
- Options "Firmware" section: `left vA (reported) · right vB (reported) · bundled vC`; a
  **Flash left half…** / **Flash right half…** button each. Clicking shows the instruction
  "Double-tap the reset button on the <side> half; waiting for its drive…" and polls removable
  drives every 500 ms for `INFO_UF2.TXT` containing `nRF52840`; when found, copies the half's UF2,
  waits for the drive to vanish (reboot), and reports "flashed; the half reconnects in a few
  seconds". Cancel stops the wait. Timeout 60 s.
- "Update available" mark next to a half whose reported build differs from the bundled one (string
  inequality; the bundled one is by definition the matching helper's). A half that reports no build
  shows "unknown — flash both halves". Both halves must run the same build (they share the split
  protocol): Options warns "halves differ" when the two reports disagree, and the Flash flow guides
  "left half, then right half".
- Pure, tested pieces: UF2 drive detection from an `INFO_UF2.TXT` body, side → file name,
  reported-vs-bundled comparison, drive polling state machine with injected clock.

## Acceptance

1. Fresh Windows user account: run the setup, accept SmartScreen, check "Start with Windows" →
   tray icon appears, overlay works, Options shows Helper version, Firmware line and Update line.
2. Flash both halves from Options with the keyboard's current firmware (same build): the drive is
   found, the file copied, the halves come back; the reported builds equal the bundled one.
3. Tag `v0.2.0` → GitHub Release with all six assets, attestations verify with
   `gh attestation verify`, SHA256SUMS match the downloads.
4. Install v0.2.0, tag v0.2.1: the running helper shows the balloon within 12 h (immediately after
   restart), Update now installs 0.2.1 and relaunches; the portable exe does the same by swap.
5. Pull the network: the check stays silent; Update now reports the failure and installs nothing.
6. Corrupt a downloaded setup (test seam: local file path as API) → verification fails, nothing
   runs, the file is deleted.
7. Uninstall removes the program and shortcuts, leaves `%APPDATA%\op36-helper`.

## Left to the builder

- HTTP client choice and the exact tray balloon wording.
- Options layout for the new sections (dialog grows; keep it one page).
- Whether the firmware job reuses the fork's workflow via `workflow_call` or copies its steps
  (copying keeps the release self-contained; reuse avoids drift).
- The order of README sections and the screenshot used.

## Open questions settled by Boris

- Updates follow the agwinterm approach (automatic check, manual apply): yes.
- Licenses: MIT for the helper repository, Apache-2.0 for the firmware fork.
- Public repository: yes, with installer and firmware releases.

## Calls made by the author after the builder check

- Migration from vendor firmware is manual and documented; the Flash button does not re-pair.
- Mixed builds across the halves are flagged and the flash flow is guided left-then-right.
- No vendor imagery in the public repository.
- Build id normalisation: both the firmware's reported id and the bundled `BUILD_COMMIT.txt` use
  plain `git describe --tags --always` from a full-history checkout (the current local file uses
  `--abbrev=40`; the release workflow generates it the firmware's way).
