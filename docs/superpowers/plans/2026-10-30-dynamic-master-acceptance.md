# Dynamic master half — on-device acceptance

Recorded 2026-10-31 with Boris at the keyboard, both halves on build commit v0.2.0-20
(`firmware/op36-layer-notify`), helper f335b88. Times from the helper log (`%APPDATA%\op36-helper\log.txt`).

| # | Check (spec acceptance) | Result |
|---|---|---|
| 1 | Cold start, no cable: left master over Bluetooth | Pass — after the right-half flash the left came up as master over BLE (02:42:06) |
| 2 | Cable into right half: swap, USB glyph and bolt on the right, batteries on the correct sides | Pass — plugging the right in for flashing took over by itself: BLE master gone 02:43:45, right on USB 02:43:47; overlay captured (`_scratch/device_master_right.png`) |
| 3 | Cable right → left: swap, glyph moves to the left | Pass — swap observed; helper first reconnected over the left's Bluetooth link (bug: glyph said BT while typing went over USB), fixed in f335b88: BLE session upgrades to USB within 2 s |
| 4 | Unplug with the right as master: left is master over Bluetooth again | Pass — unplug 02:57:39, left over BLE 02:57:49 (10 s: 5 s grace + resets + 2 s reconnect poll) |
| 3b | Unplug with the left as master: left stays master over Bluetooth | Pass — 02:57:00 → BLE 02:57:02 |
| 5 | Wall charger into either half: no swap, bolt only | Not run — no charger at hand |
| 6 | KVM host switching with the right as master: no swap | Pending — home setup |
| 7 | Right half off, cable in, power on: comes up as master; left yields via the claim | Pass (03:0x) — Boris: came up as master, left joined by itself |
| 8 | `le` typing order and combos on both masters | Pass — `lelele`, words and the U+I / I+O bracket combos on both masters |
| 9 | `role:` lines in the master's serial log | Not captured (the serial backlog had flushed by the time the port was read); role changes verified through the helper log instead |

Layout written into the right half with `--write-keymap-zmk`: `verification: 0 mismatches` (02:45).

## Second round (offline half, build commit v0.2.0-21)

Boris asked for a visible offline half after the first round. Firmware reports the split link in
layer-notify `fw_layout` bit 6 and pushes it on every link change; the helper washes that half out
and writes `offline` where its battery sits (`_scratch/snap_half_offline.png`). Needs both halves
reflashed; device check pending.

Device check 2026-10-31 ~04:00, both halves on v0.2.0-21: left half switched off while the right
was master → the left went grey with `offline` within seconds; switched on again it came back after
a while (a powered-off half loses `GPREGRET2`, boots as the default central, and yields to the
right's master claim before re-linking as a peripheral). Boris: "it works".

## Third round (firmware build ids, build commit v0.2.0-23)

Both halves flashed 2026-10-31 05:31-05:33. The Options dialog shows `Firmware: left …, right …`
from the keyboard's own reports (Vial `08 E9 03 side`; the peripheral announces its build over the
split link after the handshake).
