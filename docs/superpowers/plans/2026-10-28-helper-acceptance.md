# Helper acceptance record

Checklist from the helper plan, Task 11. Recorded 2026-10-29 with the left half on USB running the
patched firmware (layer notify, 7.5 ms split link).

| # | Check | Result |
|---|---|---|
| 1 | Clean start: bundled keymap first, live keymap within 2 s | Pass — log `keymap source at start: Bundled` → `keymap: live (16 layers), cached` within 1 s |
| 2 | Hold red / blue / both thumb keys: overlay recolours, status names the layer | Pass — probe and overlay; layer names still `Layer n` (Vial layer-name read not implemented) |
| 3 | Win+Space (Boris uses Alt+Shift) RU/EN: legends switch | Pass — language events EN→RU→EN in log; RU renders Cyrillic centred |
| 4 | Shift on laptop and home-row D: legends swap | Pass — Alt+Shift tracked; home-row mods arrive as ordinary modifiers |
| 5 | Ctrl held: EN letters shown | Pass (legend rule test + visual) |
| 6 | Find a key: halo + recipe, expires | Pass — `_scratch/snap_find.png`; recipe strip in the gap between halves |
| 7 | Unplug keyboard: tray grey, status offline, overlay kept; replug: live | Pass in design; verified only via disconnect handling tests and reconnect loop logs |
| 8 | Entropy coexistence | Not run — Entropy not installed |
| 9 | Options round-trip through restart | Pass — settings loaded identically after relaunch (log 15:32) |
| 10 | Second launch toggles the first instance | Pass |
| 11 | Idle CPU 0 %, no redraws without events | Pass by design (frame comparison); not measured with Task Manager |

Beyond the plan, verified with Boris on 2026-10-29: Bluetooth-independent typing order fix (tap-hold
rules), dark theme, battery pictograms per half, link glyph (USB / Bluetooth) on the central half,
reserve-space mode, Ctrl+Alt+Arrow nudging, Options dialog Esc/Tab (IsDialogMessageW), release-exe
CLI output. Bluetooth transport tested on the device (layer notify, keymap read, battery over BLE).
Whole-branch review (0e0a910..5acb967) closed in 854b6d3; see the ledger for accepted minors.

## Dynamic master and charging indicator (plan 2026-10-30-dynamic-master-helper)

| Check | Result |
|---|---|
| Charging bolt on a half on USB power, number still readable | Snapshot `charging=both` OK; device check pending the firmware flash |
| Link glyph on the master half (right when the right half holds the cable) | Snapshot `master=right` OK; device check pending |
| "switching master" arrow and tooltip during a swap, offline after 15 s without reconnect | Snapshot `switching` OK, model tests; device check pending |
| Today's firmware renders unchanged (no bolt, glyph left) | Live capture 2026-10-30 OK |
