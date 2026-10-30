# OP36 dynamic master half and charging indicator — design

Status: approved by Boris 2026-10-30. Builds on the layer-notify firmware
(`rmk` fork, branch `op36-layer-notify`) and the helper in `helper/`.

## Intent

Boris uses one USB cable at his home desk. It comes from a KVM, so Bluetooth to the
host is not an option there, and he moves the cable between the halves to charge
whichever is low. Today only the left half can talk to a host; with the cable in the
right half the keyboard is dead. After this change **the half that holds the cable is the
master**: it types over USB and the other half reaches it over the split link, exactly as
the left half does today. Moving the cable swaps the roles within a few seconds and the
helper shows that a swap is in progress. Secondary: each half's battery pictogram shows a
charging bolt while that half is on USB power, with a live percentage.

## Goals

1. Plug the cable into either half: that half becomes master over USB; the other half
   keeps working over the split link. Works after any order of plugging/unplugging.
2. No cable anywhere: the left half is master over Bluetooth, unchanged from today (the
   laptop's Bluetooth bond and all current behaviour stay as they are).
3. A role swap costs at most a few seconds of dead keyboard; the helper shows "switching
   master" from the moment the swap starts until the new master is up.
4. Charging bolt per half while on USB power, with the live measured percentage.
5. Typing order, tap-hold rules, layer notify, battery readout, combos: no regressions.

## Non-goals

- Right half as master over **Bluetooth** to a host. The right half never needs a host
  bond; when no USB host is present the left is master.
- Automatic sync of the keymap between the halves. Each half keeps its own Vial storage;
  the layout is written to both once (see Constraints).
- Exact "battery full" detection. The OP36 has no charger status pin (checked in the
  vendor ZMK shield and RMK config); the firmware only knows "USB power present".

## Anti-goals

- Roles flipping on their own: a wall charger, a KVM host switch, or a brief power glitch
  must not start a swap. Only a USB **host enumerating** the half may make it master.
- Both halves ending up master, or both peripheral, for longer than the fallback window.
- Breaking today's left-master-over-Bluetooth setup at work.

## Constraints and facts (from the code)

- RMK compiles the role in: `#[rmk_central]` and `#[rmk_peripheral]` generate separate
  mains (`rmk-macro/src/codegen/entry.rs`, `orchestrator.rs`, `split/peripheral.rs`).
  Row/column offsets and matrix pins are const generics per binary. The two halves have
  different pins, so there are always two firmware files, one per physical half.
- The split link pairs by address at runtime: the peripheral stores the central's address
  (`storage::read_peer_address`), the central learns the peripheral's address during the
  pairing window (`split_pairing_timeout_seconds = 30`), and the `SplitMessage::Address`
  message already carries addresses across the link.
- The peripheral binary has no USB stack today. USB host presence is observable through
  RMK's `UsbState::Configured` (`rmk/src/state.rs`) once a USB device runs; VBUS alone is
  readable from `POWER.USBREGSTATUS.VBUSDETECT` (the vendor's K04 does this for charging).
- `POWER.GPREGRET` is used by the Adafruit bootloader (0x57 = enter DFU);
  `POWER.GPREGRET2` is free and survives a soft reset.
- The vendor Vial battery query (`08 E8 01`) answers flags byte (bit0 left present,
  bit1 right present) and two percent bytes; `SPLIT_CENTRAL_IS_LEFT` is a build-time
  constant from `rmk-types/build.rs`.
- Layer notify (my feature): namespace `0xE9`, sub `0x01` state notifications to a
  subscribed host, with a spare `fw_layout` byte.
- Flashing is manual: Bootloader key on the Adjust layer or double reset, then copy the
  UF2 onto the half's drive. Boris does this; each test round needs him present.

## Design

### Roles and the decision rule

Each half runs a **dual-role firmware**: one binary per half (left pins / right pins), each
containing both the central orchestration and the peripheral runtime. At boot the binary
reads its role from `GPREGRET2`; if the register holds no valid role (cold boot, battery
inserted, bootloader exit) the **default role** applies: left = central, right = peripheral.

Runtime rules, evaluated by a small `role` module on both halves:

1. **Peripheral enumerated by a USB host → it takes over.** A half running as peripheral
   also runs the USB device stack (same descriptors). When the host reaches `Configured`,
   the half starts a handoff (below) and reboots as central. **A USB host always beats a
   Bluetooth host**: the left half as central over Bluetooth yields too. Boris's rule:
   "the cable is the master", at home and at work alike.
2. **Non-default central without USB → hands back.** The right half, running as central,
   that sees no VBUS for longer than the *grace period* (5 s; absorbs a KVM that cuts the
   port while switching hosts) hands the role back and reboots as peripheral. The left
   half as central ignores this rule: without USB it is master over Bluetooth.
3. **Charger only is not a host.** VBUS without enumeration never changes roles.

### Handoff protocol

A new split message `RoleHandoff { new_central_addr: [u8; 6] }` (appended to the enum so
existing postcard discriminants do not move) carries the swap:

1. The half that must become central sends `RoleHandoff` over the live split link (from
   the peripheral side as a message to the central, or from the central side as a
   message to the peripheral), stores the peer's address as its future peripheral address,
   writes `GPREGRET2 = CENTRAL`, waits up to 300 ms for the link write to complete, and
   soft-resets.
2. The other half, on receiving `RoleHandoff`, stores the sender's address as its future
   central address, writes `GPREGRET2 = PERIPHERAL`, and soft-resets.
3. Both come up in the new roles within about two seconds and connect by address, no
   pairing window needed.

**Fallback when the link is down at swap time** (the other half asleep or out of range):
the half that must become central reboots as central anyway. For its first 20 s as central
it also sends a non-connectable **master-claim advertisement** (manufacturer data: product
id, "I am master", own address). Every central keeps scanning for its peer anyway; a
central that sees a claim *from its peer's address* while it is not on a USB host itself
yields: it stores the claimer as its central address and reboots as peripheral. A central
that is on a USB host ignores claims (two cables: the current master keeps the role, see
below). No pairing windows are involved, and a half left alone (peer switched off) never
drops its host. Two cables at once: the half that was master first stays master; the
other charges as peripheral.

### What the host and the helper see

- The master half always enumerates as the same OP36 (same VID/PID). Windows may install
  the driver once for the right half's serial number. Through the KVM this is invisible.
- The right half as master is **USB-only**: it never advertises to Bluetooth hosts, never
  pairs, and the Adjust-layer Bluetooth output keys do nothing on it. Only the left half
  has host bonds. (Builder-facing consequence: the non-default side's central variant
  starts the USB transport only; its `ConnectionStatus.ble` stays inactive.)
- Before resetting, the old central sends a layer-notify `0xE9 0x02` notification
  "handoff starting, new master = left/right" to the subscribed host. The helper shows a
  **switching state**: both link glyphs replaced by a pulsing arrow between the halves and
  the tooltip "switching master…", until the new master connects or 15 s pass (then the
  normal offline grace logic applies).
- The layer-notify state's spare `fw_layout` byte gains bit 7 = "central is right". The
  helper draws the USB/Bluetooth glyph on the central half.
- The Vial battery reply keeps bits 0/1 and the percent bytes as they are. The firmware
  maps central/peripheral to left/right at runtime (replacing the `SPLIT_CENTRAL_IS_LEFT`
  constant for this keyboard) so the helper's sides stay correct. Bits 2/3 of the flags
  byte become "left on USB power" / "right on USB power". Entropy ignores unknown bits.

### Charging indicator

- `battery_nrf.rs` on both halves reads `VBUSDETECT` with each sample and publishes
  `ChargeState::Charging` while power is present, `Discharging` otherwise. The measured
  percentage is published live in both states (Boris's choice; it reads near 100 during
  the final charging phase and that is accepted).
- Helper: the pictogram of a half on USB power shows a small lightning bolt left of the
  number; fill colour rules unchanged.

### Keymap and settings on both halves

Vial storage is per half. The ZMK layout, bracket combos and tap-hold rules are written
to each half while it is master (`op36-helper.exe --write-keymap-zmk`, once per half).
`FLASHING.md` documents that any future layout change goes to both halves.

### Firmware structure (vendor fork, feature `dynamic_role`)

- `keyboard.toml`: `[split] dynamic_role = true`. The macro then generates, for the
  `central` bin (left pins) and the `peripheral` bin (right pins), a main that: inits the
  chip, storage and BLE stack once; reads the role; branches into the existing central
  orchestration (own matrix with its offsets, peripheral manager with the peer offsets) or
  the existing peripheral runtime plus the USB device stack and the role watcher.
- `rmk/src/split/role.rs` (new): role enum, `GPREGRET2` read/write, decision state machine
  (pure, host-testable: inputs are USB state, VBUS, link state, elapsed time, side;
  outputs are "start handoff", "offer", "stay"), grace and fallback timers.
- `SplitMessage::RoleHandoff`, handled in `split/central.rs` and `split/peripheral.rs`.
- `host/layer_notify.rs`: handoff notification and the central-side bit.
- `host/via/mod.rs`: runtime left/right mapping and charging flag bits.
- Host tests (`cargo nextest`, `x86_64-pc-windows-msvc`) for the state machine, message
  encoding, Vial reply bits and layer-notify encoding. On-device verification per the
  acceptance list below.

### Helper changes

- `vial/protocol.rs`: charging bits; layer-notify `0x02` handoff packet; central-side bit.
- Model: `link_side` (left/right), `switching: Option<Instant>`; battery `charging` flags.
- Renderer: bolt in the pictogram; glyph on the central half; switching animation (timer
  redraw every 500 ms only while switching).
- Settings: none new.

## Acceptance (on the device, with Boris)

1. Cold start, no cable: left master over Bluetooth, overlay as today.
2. Cable into right half (through the KVM or laptop): within ~4 s the right half types over
   USB, overlay shows "switching master", then the USB glyph on the right half, batteries on
   the correct sides, right battery shows the bolt.
3. Move cable right → left: swap again, glyph moves to the left.
4. Unplug the cable with the right half master: within ~7 s the left is master over
   Bluetooth again (grace 5 s + reboot).
5. Wall charger into either half: no swap; bolt shows on that half.
6. Switch KVM hosts several times with the right half master: no swap, typing resumes on
   the new host.
7. Right half asleep (link down) when the cable goes into it: comes up as master; the left,
   even while typing to the laptop over Bluetooth, yields on the claim and joins within
   ~25 s without touching it.
7b. Work setup: left master over Bluetooth; right half plugged into the laptop's USB port
   → right becomes master over USB (cable rule). Unplug → left is master over Bluetooth
   again within ~7 s. Charging from a wall adapter at work: nothing changes.
8. `le` typing-order test and combos still pass on both masters.
9. Firmware logs over USB serial from the master half show the role decisions.

## Left to the builder

- Exact codegen structure for the dual-role main (how much of `orchestrator.rs` is reused
  vs duplicated), as long as both bins stay buildable with `build-op36.sh`.
- Timing constants other than the grace (5 s) and the claim window (20 s), which are the
  user-facing behaviour above.
- How the peripheral role tells "USB host configured" apart from the central's synced
  `ConnectionStatus` (which overwrites the local USB state today): use the local USB
  device's own event, not `current_usb_state()`.
- Whether the handoff notification reuses the layer-notify subscription TTL (120 s) or is
  sent regardless of subscription (either is acceptable; sent regardless is simpler).
- Appearance details of the bolt and the switching animation within the existing
  pictogram and gap area.

## Open questions settled by Boris

- A few seconds of dead keyboard during a swap: acceptable, provided the helper shows it.
- Live percentage while charging rather than a frozen one: yes.
- KVM power behaviour on host switch: unknown; the 5 s grace covers a cut port.
- Cable rule also at work (right half on the laptop's USB becomes master over USB,
  Bluetooth resumes when unplugged): confirmed 2026-10-30. A wall charger on either half
  must never take the master role.

## Calls made by the author after the builder check

- USB host beats Bluetooth host, everywhere (replaces "left on Bluetooth counts as on a
  host"); this is what makes the KVM case work even if the laptop's Bluetooth is on.
- Master-claim advertisement replaces the earlier "offer itself for a pairing window"
  fallback: no half ever drops its host while merely waiting for its peer.
- Right half as master is USB-only.
