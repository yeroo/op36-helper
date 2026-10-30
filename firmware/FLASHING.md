# Flashing the OP36 (RMK)

Vendor procedure as performed on 2026-10-28, plus the layer-notify custom build.

## Files

| Purpose | File |
|---|---|
| Stock RMK v0.2.0 (rollback), settings reset, vendor ZMK release | Ergohaven's downloads (docs.ergohaven.xyz, github.com/ergohaven/rmk releases); not redistributed here. Boris keeps local copies in the private archive. |

## Source

The firmware source is the submodule `rmk/` (github.com/yeroo/rmk, branch `op36-layer-notify`), pinned
to the commit the UF2 files were built from. Clone with `git clone --recurse-submodules`, or run
`git submodule update --init` in an existing checkout; `firmware/tools/build-op36.sh` builds both
halves from it. The helper's `rmk-types` path dependency also points into this submodule.

## Procedure (one half at a time)

1. Plug the USB-C cable into the half you are flashing.
2. Press the small Reset button on the underside **twice quickly**.
3. A removable drive `NRF52BOOT` (32 MB, FAT) appears, usually as `D:`. It contains
   `CURRENT.UF2`, `INDEX.HTM`, `INFO_UF2.TXT` (bootloader 0.9.2, SoftDevice S140 6.1.1).
4. Copy the `.uf2` onto the drive. The drive disappears within a few seconds while the
   half reboots. A Windows copy/eject error dialog at this point can be ignored.
5. Windows enumerates the half as `VID_E126&PID_0036` ("Ergohaven OP36"): keyboard,
   mouse/consumer, a vendor-defined HID collection (Vial, usage page 0xFF60) and a serial port.

Since the dual-role build (2026-10-30) **both** halves run the custom firmware: `op36_left.uf2`
on the left half, `op36_right.uf2` on the right half. Flash them one after the other.

## Dual-role firmware: which half is the master

- The half a USB **host** enumerates is the master. It types over USB and the other half follows
  over the split link. Moving the cable swaps the roles within a few seconds (the helper shows
  "switching master" meanwhile). A wall charger or power bank never changes roles.
- No cable anywhere: the left half is the master over Bluetooth, as before.
- The right half as master is USB-only: it never pairs with Bluetooth hosts.
- Vial storage is per half. After the first flash, write the layout into **each** half while it is
  the master: cable into that half, wait for the overlay's USB glyph to sit on it, exit the tray
  helper, run `helper\target\release\op36-helper.exe --write-keymap-zmk`, expect
  `verification: 0 mismatches`, restart the helper. Any later layout change goes to both halves.
- The helper's Options dialog shows each half's firmware build id (`Firmware: left …, right …`),
  so a half that missed a flash is easy to spot.
- Diagnosis: the master half's serial port prints `role: ...` lines (take over, hand back, yield,
  handoff received). `GPREGRET2` holds the role across a soft reset (0xC3 central, 0xA5 peripheral);
  a battery pull clears it and the defaults apply.
- If both halves ever end up master (two cables and the link was down), remove one cable: the
  right half hands back within five seconds of losing power, or within 30 s of losing its host;
  the left yields to the right's master claim within about 10 s.

## After flashing both halves (first migration only)

- Press Reset once on both halves at the same time so they pair with each other (30 s window).
- Remove the old Bluetooth pairing in Windows and pair again; the name changes with the firmware.
- If the layout looks wrong, import the vendor's default `.entlayout` for OP36 in Entropy.

## Rollback

Flash the vendor's stock `op36_left.uf2` / `op36_right.uf2` (from Ergohaven's downloads) with the same
procedure. If a half misbehaves after a firmware change, flash the vendor's `settings_reset.uf2`
first, then the firmware again.

## Verifying layer notify

```
.venv/Scripts/python -I firmware/tools/layer_notify_probe.py
```
Stock firmware echoes the request (`unexpected reply: 07e9011e…`). The custom firmware answers
`reply: active=0 default=0 mask=0b0 fw_layout=0` and prints a `notify:` line on every layer
change. Observed on 2026-10-29 with the factory keymap: red thumb `active=1 mask=0b10`, blue thumb
`active=2 mask=0b100`, red then blue `active=3 mask=0b1010` (the Nav layer's blue position is `MO(3)`,
so layer 3 is entered directly, not through the tri-layer rule), red released while blue held
`active=3 mask=0b1000`. The probe renews every 20 s with TTL 60 like the helper; a one-shot 30 s
subscription was observed to go silent after expiry. `--listen` runs without subscribing.

## Layout and tap-hold rules (no reflash needed)

`helper\target\release\op36-helper.exe --write-keymap-zmk` (exit the tray helper first; the CLI needs the keyboard to itself) writes Boris's ZMK layout (4 layers, the
U+I / I+O bracket combos) and the tap-hold rules into the keyboard over Vial; `--write-taphold`
writes only the rules. The rules fix the "l then e arrives as el" problem seen on 2026-10-29: RMK
sent the key that interrupted an undecided home-row mod-tap immediately and the mod-tap afterwards.
With permissive hold the interrupting key is queued behind the decision, unilateral tap resolves
same-hand rolls at once, and flow tap (prior idle 150 ms) resolves a home-row key pressed within
150 ms of the previous key as a plain tap immediately. Hold timeout 280 ms. The rules live both in
each home-row key's MorseProfile (inside the keymap) and in the global behaviour settings.
