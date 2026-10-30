# Distribution — acceptance record

Spec: `docs/superpowers/specs/2026-10-31-distribution-design.md`. Plan: `2026-10-31-distribution.md`.

## First public release (plan Task 7), 2026-10-30

Public repository: https://github.com/yeroo/op36-helper (created empty, MIT `LICENSE` from the tree).
Exported with `tools/export-public.ps1` from private commit f0b3a64 plus the Task 7 edits; one
initial commit `d3ef97e` "OP36 Helper 0.2.0: overlay, dual-role firmware, installer", 85 files,
no history, no vendor firmware or screenshots (the only PNGs are `docs/img/overlay.png` and the
two tray-icon previews). No attribution trailers in any commit or in the release notes.

| Check | Result |
|---|---|
| `rmk` submodule pointer in the first commit | First push lost it: `git add -A` drops a gitlink whose directory is empty. Fixed by staging the gitlink with `git update-index --cacheinfo 160000,<sha>,rmk` after `git add -A`, amended and force-pushed before anything else referenced it; the export script's header now says so. Pins `74ed094c` (`v0.2.0-23-g74ed094c`), the build both halves run. |
| Fresh recursive clone into `%TEMP%` | Pass — `git clone --recurse-submodules` checks out `rmk` at `74ed094c`. |
| Tag `v0.2.0` → Release workflow run 41039398668 | Pass first time: firmware job (ubuntu) 2 m 26 s, installer job (windows) ~9 min, release published. |
| Six assets on the release | Pass — `op36-helper-setup-0.2.0.exe` (3.6 MB), `op36-helper-portable-0.2.0.exe` (2.4 MB), `op36_left.uf2`, `op36_right.uf2`, `BUILD_COMMIT.txt` = `v0.2.0-23-g74ed094c`, `SHA256SUMS.txt`; GitHub publishes a `sha256:` digest for each. |
| `gh attestation verify` on downloaded copies | Pass — setup, portable and left UF2 each verify against one Sigstore attestation for `yeroo/op36-helper` (exit 0, subject digest matches). The command prints nothing when stdout is not a terminal; use `--format json` to see the statement. |
| `SHA256SUMS.txt` matches | Digests match all five files, but the file had CRLF endings (`Set-Content` on the Windows runner), so `sha256sum -c` fails on Linux until `tr -d '\r'`. Fixed in `release.yml` (writes LF); the v0.2.0 asset stays as published, 0.2.1 carries the fix. |
| Portable exe `--version` | `op36-helper 0.2.0` |
| CI on `main` | First run red: the firmware host tests inherited `rmk/.cargo/config.toml`'s default target (`thumbv7em-none-eabihf`, "can't find crate for std"). Fixed with `--target x86_64-unknown-linux-gnu` (what the vendor fork's `test.sh` does); fix commit `8176e01` on the public repo, same edit in this repo. Run 41040869077 on `8176e01`: both jobs green (helper tests on Windows, 843 firmware host tests on Linux). |

Private repo README now points at the public mirror and the export script.

## Acceptance on this machine (plan Task 8), 2026-10-30

Run with the release assets downloaded from GitHub; the helper's own log (`%APPDATA%\op36-helper\log.txt`)
is the source for times. Boris was away, so the two steps that need hands on the keyboard or a mouse
on a SmartScreen prompt are listed as pending.

| Step | Check | Result |
|---|---|---|
| 1 | Install `op36-helper-setup-0.2.0.exe`, Start with Windows ticked | Pass (silent `/VERYSILENT /TASKS=startup`, 5 s): program in `%LOCALAPPDATA%\Programs\OP36 Helper` with `firmware\` bundle `v0.2.0-23-g74ed094c`, HKCU uninstall entry 0.2.0, Start Menu and Startup shortcuts point at the installed exe. Tray icon, overlay and Options came up; Options showed Helper 0.2.0, both halves `v0.2.0-23-g74ed094c (up to date)`, bundled the same, "Helper 0.2.0 — up to date" from the live GitHub check. |
| 1 | SmartScreen "More info → Run anyway" on first interactive run | Pending — needs a mouse; the downloaded file carried no mark-of-the-web through `gh release download`, so Boris should download it with a browser to see the real prompt. |
| 1 | Uninstall keeps `%APPDATA%\op36-helper`, reinstall | Pass: settings, keymap cache, icons and log stayed; shortcuts and the registry entry went; reinstall 5 s. |
| 1 | Install or uninstall **while the helper is running** | **Failed on 0.2.0, fixed in 0.2.1.** Inno's `CloseApplications` asks through Restart Manager (WM_QUERYENDSESSION / WM_ENDSESSION on every top-level window); the message window and the overlay left those to `DefWindowProc`, so Setup waited 30 s, reported "unable to automatically close all applications" and cancelled (exit 5). The uninstaller has no Restart Manager step at all and left `op36-helper.exe` behind. Fix (private commits a12cba8, e6552f3; public fccefda): `win::msgwindow::system_request` answers those messages and WM_CLOSE by leaving the message loop (tests `restart_manager_close_request_exits`, `cancelled_end_session_and_other_messages_are_not_requests`, RED→GREEN), and the uninstaller's `InitializeUninstall` posts WM_CLOSE to `Op36HelperMsg` and waits up to 5 s. Verified with the 0.2.1 installer: install over a running 0.2.1 helper 6 s with "exit requested by the system" / "app bar released" in the log; uninstall over a running helper 5 s, directory gone, settings kept. A running **0.2.0** still cannot be closed by the installer: upgrading 0.2.0 by hand means exiting it first (the in-app update exits by itself, so that path is unaffected). |
| 2 | Flash left then right from Options with the release firmware | Pending — needs the reset button on each half. Both halves already run `v0.2.0-23-g74ed094c`, the build id the release carries, so Options reports them up to date and the buttons are not required. Note: the CI-built UF2s are not byte-identical to the locally built ones in `firmware/op36-layer-notify` (different toolchain paths), only the build id matches. |
| 3 | 0.2.1 release → in-app update of the installed 0.2.0 | Pass: public tag `v0.2.1` (run 41047901209), six assets, `sha256sum -c SHA256SUMS.txt` passes as published (LF), attestations verify for setup and portable. Installed 0.2.0 started 11:36:31, "0.2.1 available" at 11:37:02 (31 s), Update now → Downloading 11:37:34 → Verified 11:37:36 → exited; 0.2.1 running from the install directory, HKCU DisplayVersion 0.2.1, `updates\` folder empty. |
| 3 | Portable exe | Pass: portable 0.2.0 in a scratch folder → Update now → swapped in place and relaunched as 0.2.1 (`--version` 0.2.1), no `.old` left. |
| 3 | Relaunch gap | Observation: 30 s (portable) and 44 s (installed) between "exiting" and the new version's "starting". The apply script has no such wait and the helper's own exit is immediate (Restart Manager closed it in under 3 s). Most likely Defender's first-run inspection of a never-seen exe plus PowerShell start-up (8 s on this machine today); not verified. The tray icon is absent for that long after an update. |
| 4 | Offline: `OP36_UPDATE_API=http://127.0.0.1:9/latest` | Pass: automatic check at 32 s logged "update check failed (offline?)", no balloon; Update now showed "Update failed: update check failed (offline or rate-limited); try again later". |
| 4 | Corrupted digest (`_scratch/upd/latest_bad.json`) | Pass: "9.9.9 available" → Downloading → "Update failed: download failed SHA-256 verification; nothing was installed"; nothing in `updates\`. |

End state: 0.2.1 installed and running from `%LOCALAPPDATA%\Programs\OP36 Helper`, Startup shortcut
points at it (the dev-build shortcut is gone), settings untouched.

## Final review and 0.2.2, 2026-10-30

A fresh reviewer read the whole branch (02e0c10..34a962e): no Critical, five Important, eight Minor
findings. All five Important ones and one re-graded Minor were fixed with tests first (158 helper tests
now) and shipped as public release **v0.2.2** (run 41069491007; private commits d7ecf95, a7b0f99):

| Finding | Fix | Verified by |
|---|---|---|
| A release published before its assets are uploaded counted as "available" | `update::available_update` requires a channel asset with a verifiable digest | unit tests; live update below |
| Opening Options contacted GitHub with automatic checks off | `UpdateCmd::OptionsOpened` is gated on the setting; the unconditional check command is gone | unit test |
| Apply-script failures left no helper running; `-Path` treated `[ ]` as wildcards | `-LiteralPath` everywhere, setup exit code checked, every failure relaunches the old exe | script text test; bogus payload in `p [x]` → rolled back and relaunched; real swap in `p [x]` → new exe in place, back in 7 s |
| "Start with Windows" ignored the installer's shortcut; silent upgrades re-created a removed one | `autostart::is_enabled` at start-up; installer `Check: StartupWanted` keeps the current state on silent upgrades | unit test; shortcut removed stays removed, present stays present, fresh silent install with the task creates it |
| Release workflow ran `uf2conv.py` from microsoft/uf2 master | uses the repo's hash-pinned `firmware/tools/uf2conv.py` | v0.2.2 firmware job green; UF2s carry `v0.2.0-23-g74ed094c` |
| (re-graded) a user-chosen install folder could be deleted whole on uninstall | `DisableDirPage=yes` | 0.2.2 installer builds and installs |

Live update **0.2.1 → 0.2.2**: Options open → "0.2.2 available" → Update now → Downloading 14:20:04 →
Verified 14:20:06 → exited → 0.2.2 running from the install directory at 14:20:23 (17 s), Startup
shortcut kept, no leftovers. `sha256sum -c` and the setup attestation pass on the downloaded 0.2.2
assets. The UF2 bytes differ between the 0.2.1 and 0.2.2 CI builds (same source, same build id):
the firmware build is not bit-reproducible across runs, which is why the helper compares build ids,
not hashes.

Deferred minors (reviewer's list, Boris's call): a UF2 drive already mounted when Flash is clicked is
written at once; `fs::copy` may report an error after a good write when the bootloader reboots
early; Update now is not disabled after a click; update/flash events are dropped while the app state
is borrowed; the `OP36_UPDATE_API` seam is live in release builds and a `v`-tag that is not `v<digit>`
skips the version check; `--dirty` only locally and the id is cut at 28 bytes; two stray UI states
(disconnected keyboard wording, reopening Options mid-flash).

Still pending with Boris at the machine: SmartScreen on a browser-downloaded setup exe, and one real
flash from Options (both halves already run the release build id).

## Deferred minors done, 0.2.3, 2026-10-30

Boris asked for the reviewer's minors as well. All seven are in private commits after a7b0f99 and
ship as public **v0.2.3** (165 helper tests):

| Minor | Change | Verified by |
|---|---|---|
| A bootloader drive already mounted at the click was flashed at once | `FlashMachine::new(side, now, mounted)` ignores that drive until it has gone away once; new step `AlreadyMounted` tells the user to double-tap reset on the named half | `a_drive_already_mounted_at_the_click_is_not_flashed_until_it_reappears` |
| `fs::copy` error after a good write reported "Flash failed" | copy errors enter a `CopyFailed` phase: drive gone within 3 s = Done, drive still there = Failed with the error | two machine tests; the old immediate-failure test updated on purpose |
| Update now not disabled after a click | `App.update_busy`; the dialog disables the button on click and until a terminal event (`update_in_progress_after`) | unit test; debug build against a non-routable host: button off right after the click and 6 s later |
| Update/flash events dropped while `App` was borrowed | `win::deferred::Deferred` parks them and the next message replays them in order | `parked_events_replay_in_order_once` |
| `OP36_UPDATE_API` seam live in release builds; a stray `v` tag could publish | the seam is `cfg!(debug_assertions)` only; release.yml rejects any `v` tag that is not `vX.Y.Z` equal to the Cargo version | debug build still honours the seam; release 0.2.3 run passes the check |
| Build id `--dirty` only locally; helper compared the full bundled id with the firmware's 24-byte one | release.yml uses `--dirty` too; `half_status` compares at `FIRMWARE_BUILD_MAX` (24) | `bundled_build_id_is_compared_at_the_length_the_firmware_can_report` |
| Disconnected keyboard said "unknown — flash both halves" + "flash left first"; reopening Options mid-flash lost the progress and Cancel | `firmware::describe` words a disconnected keyboard as "not connected" with no flash advice; `FirmwareView` carries the running flash step so a reopened dialog shows it with Cancel enabled | `disconnected_keyboard_gets_no_flash_advice`; dev build: flash started, dialog closed and reopened → step text and Cancel present, cancel works |

## Keyboard off: show, hide after a minute, restore (0.2.4), 2026-10-30

Boris asked: switching the keyboard off (master half included) should be shown, the overlay should
hide after one minute, and it should come back when the halves are on again. Done as
`helper/src/auto_hide.rs` (pure machine: ArmTimer / CancelTimer / Hide / Restore, tests for the
happy path, an early reconnect, and the user's own toggle winning) wired to the connection
transitions in `App::on_model_event`, timer `TIMER_OFF_HIDE` (60 s), the app-bar reservation and
"only while held" mode. The renderer washes both halves out with "off" while disconnected
(`--snapshot … keyboard-off` renders it). Starting the helper with the keyboard off counts as
switched off too: the log shows "keyboard off: overlay hides in 60 s" and then "keyboard connected"
two seconds later when it is on (the countdown is cancelled). The hide and the restore after a real
power-off were not exercised here (the keyboard was on throughout); Boris's first evening with it is
the device test. Shipped as public v0.2.4.
