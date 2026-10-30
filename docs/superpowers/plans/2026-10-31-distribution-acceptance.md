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
