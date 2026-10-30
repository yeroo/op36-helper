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
