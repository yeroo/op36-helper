# Dynamic Master Half — Firmware Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Both OP36 halves run a dual-role firmware: the half a USB host enumerates becomes master, the other half follows over the split link, and both halves report USB power so the helper can draw a charging bolt.

**Architecture:** The vendor RMK fork gets a `[split] dynamic_role = true` option. The macros then emit, per half binary, an `as_central` module (today's central main, with the right half's variant using USB-only host transport) and an `as_peripheral` module (today's peripheral main plus a USB device so host enumeration is observable), and a tiny `main` that picks one from `POWER.GPREGRET2`. A pure, host-tested `role` state machine decides take-over / hand-back / yield from USB-host, VBUS, link and claim inputs; a new `SplitMessage::RoleHandoff` carries the swap over the link; a non-connectable "master claim" advertisement settles swaps made while the link was down.

**Tech Stack:** Rust no_std, embassy, nrf-sdc/trouble-host, RMK fork at `omega36/rmk` (branch `op36-layer-notify`), host tests with `cargo nextest --target x86_64-pc-windows-msvc`, firmware build via `firmware/tools/build-op36.sh`, UF2 flashing by Boris.

**Spec:** `docs/superpowers/specs/2026-10-29-dynamic-master-design.md`

## Global Constraints

- Work happens in `C:\Users\boris_kudriashov\Source\omega36\rmk` on branch `op36-layer-notify` (Boris's fork of the vendor fork); commit after every task; the tree must be clean before `build-op36.sh` runs (it refuses a dirty tree).
- Host tests: `cd rmk && cargo nextest run --target x86_64-pc-windows-msvc -p rmk <filter>` (plain `cargo test` is rejected). Firmware build: `bash firmware/tools/build-op36.sh` from `omega36`, producing `firmware/op36-layer-notify/op36_left.uf2` and `op36_right.uf2`.
- Append new `SplitMessage` variants at the end of the enum (postcard discriminants must not move).
- Timings from the spec: USB-loss grace 5 s; claim bursts while a central has no peripheral link; a peripheral takes over only on USB **host enumeration** (`Configured`), never on VBUS alone.
- Default role with no `GPREGRET2` hint: left = central, right = peripheral. `GPREGRET2` magic: `0xC3` central, `0xA5` peripheral, anything else = none. `GPREGRET` stays untouched (bootloader).
- Right half as central: USB host transport only (no `BleTransport`), the split BLE stack still runs.
- Peer address storage slot 0 is "the other half" in both roles (`storage::read_peer_address(0)`); the swap must not rewrite it.
- No changes to the Vial battery reply bytes 0–6 beyond flags bits 2/3; `ERGOHAVEN_BATTERY_HALVES_VERSION` stays 1.
- Each on-device test round needs Boris to put a half into its UF2 drive; batch device checks into Task 8.

## Review Focus

1. Peripheral role receives the central's synced `ConnectionStatus` (`usb: Configured`) over the link; the role machine must use the **local** USB device event, not `current_usb_state()`, or every peripheral on a battery would take over the moment the master enumerates. Pinned in Task 4 (`LOCAL_USB_CONFIGURED` signal) and Task 3 test `peripheral_ignores_synced_central_usb_state`.
2. Both halves in a wall charger, link up: no decision ever fires (VBUS without enumeration). Task 2 test `vbus_without_host_never_takes_over`.
3. Two cables, link up: the peripheral sees the central's status `usb: Configured` and must not take over ("first master stays"). Task 2 test `peripheral_defers_when_central_has_usb_host`.
4. Right half as central loses VBUS for 2 s (KVM switch) and gets it back: no hand-back. Task 2 test `usb_loss_shorter_than_grace_is_ignored`.
5. A half alone (other half switched off) as central with a Bluetooth host: never yields, never resets, keeps typing. Task 2 test `central_without_claim_never_yields`.

---

### Task 1: Charging state from VBUS on both halves

**Files:**
- Modify: `rmk/keyboards/op36/src/battery_nrf.rs`

**Interfaces:**
- Produces: `pub(crate) fn usb_powered() -> bool` (reads `embassy_nrf::pac::POWER.usbregstatus().read().vbusdetect()`, same as `keyboards/k04/src/battery_nrf.rs`), and `publish_sample` publishing `BatteryStatus::Available { charge_state, level: Some(percent) }` with `charge_state = ChargeState::Charging` when `usb_powered()`, else `ChargeState::Discharging` (never `Unknown`). The level is the live measurement in both states (Boris's choice).

- [ ] **Step 1: Add `usb_powered()` and the charge state to `publish_sample`** (copy the K04 `usb_power_status` pattern; keep the SAADC timeout logic unchanged).
- [ ] **Step 2: Build the keyboard crate for both bins to check it compiles**

Run: `cd rmk/keyboards/op36 && cargo build --release --bin central --bin peripheral 2>&1 | tail -3` (with the same `BINDGEN_EXTRA_CLANG_ARGS` the build script exports; or run `build-op36.sh` after committing).
Expected: `Finished` with no errors.

- [ ] **Step 3: Commit**

```bash
git -C rmk add keyboards/op36/src/battery_nrf.rs
git -C rmk commit -m "feat(op36): report charging state from VBUS with live level"
```

---

### Task 2: Role state machine (pure, host-tested)

**Files:**
- Create: `rmk/rmk/src/split/role.rs`
- Modify: `rmk/rmk/src/split/mod.rs` (add `pub mod role;`)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Role { Central, Peripheral }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Side { Left, Right }
  pub const USB_LOSS_GRACE_MS: u64 = 5_000;
  pub const HANDOFF_SETTLE_MS: u64 = 300;
  pub fn default_role(side: Side) -> Role;              // Left → Central, Right → Peripheral
  pub fn central_is_left(side: Side, role: Role) -> bool;
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub struct RoleInputs {
      pub now_ms: u64,
      pub local_usb_configured: bool,   // this half's own USB device reached Configured
      pub vbus: bool,                   // this half's VBUSDETECT
      pub link_up: bool,                // split link to the other half is connected
      pub central_has_usb_host: bool,   // peripheral role only: synced ConnectionStatus.usb == Configured
      pub peer_claim_seen: bool,        // central role only: claim advert from the stored peer address
      pub ble_host_connected: bool,     // central role only: informational, must not change decisions
  }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Decision { Stay, TakeOver, HandBack, Yield }
  pub struct RoleMachine { /* side, role, vbus_lost_at_ms: Option<u64> */ }
  impl RoleMachine {
      pub fn new(side: Side, role: Role) -> Self;
      pub fn step(&mut self, inputs: RoleInputs) -> Decision;
  }
  ```
  Semantics: `TakeOver` = this half becomes central (send `RoleHandoff { new_central_is_left }` if `link_up`, then reset as central). `HandBack` = this half (non-default central) becomes peripheral (send handoff, then reset). `Yield` = this central becomes peripheral silently (claim seen, link down). Rules: Peripheral: `local_usb_configured && !central_has_usb_host` → `TakeOver`, else `Stay`. Central: `peer_claim_seen && !local_usb_configured` → `Yield`; right side (non-default) and `!vbus` continuously for ≥ `USB_LOSS_GRACE_MS` → `HandBack`; everything else `Stay`. `vbus` true resets the loss timer.

- [ ] **Step 1: Write the failing tests** in `role.rs` `#[cfg(test)] mod tests`:

```rust
fn inputs(now_ms: u64) -> RoleInputs { RoleInputs { now_ms, local_usb_configured: false, vbus: false, link_up: true, central_has_usb_host: false, peer_claim_seen: false, ble_host_connected: false } }

#[test] fn defaults_are_left_central_right_peripheral() {
    assert_eq!(default_role(Side::Left), Role::Central);
    assert_eq!(default_role(Side::Right), Role::Peripheral);
    assert!(central_is_left(Side::Left, Role::Central));
    assert!(!central_is_left(Side::Right, Role::Central));
    assert!(central_is_left(Side::Right, Role::Peripheral));
}
#[test] fn peripheral_enumerated_by_host_takes_over() {
    let mut m = RoleMachine::new(Side::Right, Role::Peripheral);
    assert_eq!(m.step(RoleInputs { local_usb_configured: true, vbus: true, ..inputs(0) }), Decision::TakeOver);
}
#[test] fn vbus_without_host_never_takes_over() {
    let mut m = RoleMachine::new(Side::Right, Role::Peripheral);
    for t in (0..60_000).step_by(250) { assert_eq!(m.step(RoleInputs { vbus: true, ..inputs(t) }), Decision::Stay); }
}
#[test] fn peripheral_defers_when_central_has_usb_host() {
    let mut m = RoleMachine::new(Side::Left, Role::Peripheral);
    assert_eq!(m.step(RoleInputs { local_usb_configured: true, vbus: true, central_has_usb_host: true, ..inputs(0) }), Decision::Stay);
}
#[test] fn peripheral_ignores_synced_central_usb_state() {
    // central_has_usb_host alone (no local enumeration) must never trigger anything
    let mut m = RoleMachine::new(Side::Right, Role::Peripheral);
    assert_eq!(m.step(RoleInputs { central_has_usb_host: true, ..inputs(0) }), Decision::Stay);
}
#[test] fn right_central_hands_back_after_grace_without_vbus() {
    let mut m = RoleMachine::new(Side::Right, Role::Central);
    assert_eq!(m.step(RoleInputs { vbus: false, ..inputs(0) }), Decision::Stay);
    assert_eq!(m.step(RoleInputs { vbus: false, ..inputs(4_999) }), Decision::Stay);
    assert_eq!(m.step(RoleInputs { vbus: false, ..inputs(5_000) }), Decision::HandBack);
}
#[test] fn usb_loss_shorter_than_grace_is_ignored() {
    let mut m = RoleMachine::new(Side::Right, Role::Central);
    m.step(RoleInputs { vbus: false, ..inputs(0) });
    m.step(RoleInputs { vbus: true, ..inputs(2_000) });
    assert_eq!(m.step(RoleInputs { vbus: false, ..inputs(6_000) }), Decision::Stay, "timer restarted at 6 s");
    assert_eq!(m.step(RoleInputs { vbus: false, ..inputs(11_000) }), Decision::HandBack);
}
#[test] fn left_central_without_vbus_stays_master_over_bluetooth() {
    let mut m = RoleMachine::new(Side::Left, Role::Central);
    for t in (0..120_000).step_by(1_000) { assert_eq!(m.step(RoleInputs { vbus: false, ble_host_connected: true, ..inputs(t) }), Decision::Stay); }
}
#[test] fn central_yields_to_peer_claim_unless_on_usb_host() {
    let mut m = RoleMachine::new(Side::Left, Role::Central);
    assert_eq!(m.step(RoleInputs { peer_claim_seen: true, link_up: false, ble_host_connected: true, ..inputs(0) }), Decision::Yield);
    assert_eq!(m.step(RoleInputs { peer_claim_seen: true, link_up: false, local_usb_configured: true, vbus: true, ..inputs(1) }), Decision::Stay);
}
#[test] fn central_without_claim_never_yields() {
    let mut m = RoleMachine::new(Side::Left, Role::Central);
    for t in (0..600_000).step_by(5_000) { assert_eq!(m.step(RoleInputs { link_up: false, ble_host_connected: true, ..inputs(t) }), Decision::Stay); }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd rmk && cargo nextest run --target x86_64-pc-windows-msvc -p rmk role::tests`
Expected: compile error (module missing).

- [ ] **Step 3: Implement `role.rs`** with the types and rules above (no embassy/pac dependencies in this task; keep the file `no_std`-clean).
- [ ] **Step 4: Run the tests to verify they pass**

Run: same command. Expected: 10 passed.

- [ ] **Step 5: Commit**

```bash
git -C rmk add rmk/src/split/role.rs rmk/src/split/mod.rs
git -C rmk commit -m "feat(split): role state machine for the dynamic master half"
```

---

### Task 3: Boot role, side registry, handoff message and claim signals

**Files:**
- Modify: `rmk/rmk/src/split/role.rs` (device-side helpers behind `#[cfg(all(feature = "_nrf_ble", not(test)))]` or the crate's existing nrf cfg; host-test stubs otherwise)
- Modify: `rmk/rmk/src/split/mod.rs` (`SplitMessage::RoleHandoff { new_central_is_left: bool }` appended as the **last** variant)
- Modify: `rmk/rmk/src/split/driver.rs` (`process_peripheral_message`: handle `RoleHandoff`), `rmk/rmk/src/split/peripheral.rs` (central → peripheral loop: handle `RoleHandoff`; also send one when `ROLE_HANDOFF_TX` is signalled)
- Modify: `rmk/rmk/src/host/via/mod.rs:621` (`crate::SPLIT_CENTRAL_IS_LEFT` → `crate::split::role::central_is_left_now()`)
- Test: `rmk/rmk/src/split/role.rs` tests, `rmk/rmk/src/host/via/mod.rs` existing battery tests

**Interfaces:**
- Produces (role.rs):
  ```rust
  pub fn init(side: Side, role: Role);               // stores both in atomics at boot
  pub fn current_side() -> Side; pub fn current_role() -> Role;
  pub fn central_is_left_now() -> bool;              // central_is_left(current_side(), current_role())
  pub fn read_boot_role() -> Option<Role>;           // GPREGRET2: 0xC3 → Central, 0xA5 → Peripheral
  pub fn reset_into(role: Role) -> !;                // write GPREGRET2 magic, cortex_m::peripheral::SCB::sys_reset()
  pub static ROLE_HANDOFF_TX: Signal<RawMutex, bool>;   // request the split loop to send RoleHandoff { new_central_is_left }
  pub static PEER_CLAIM_SEEN: Signal<RawMutex, ()>;     // set by the central scanner (Task 5)
  pub static LOCAL_USB_CONFIGURED: AtomicBool;          // set/cleared by the USB device handler (Task 4)
  pub async fn apply_handoff_received(new_central_is_left: bool) -> !;  // compute own new role from current_side(), wait HANDOFF_SETTLE_MS, reset_into
  ```
- Consumes: Task 2 types.

- [ ] **Step 1: Write the failing test** `handoff_received_maps_to_own_role` (pure helper `role_after_handoff(side: Side, new_central_is_left: bool) -> Role`: Left + true → Central, Left + false → Peripheral, Right + true → Peripheral, Right + false → Central) and `boot_role_magic_round_trips` (pure `role_from_magic(u8) -> Option<Role>` / `magic_for(Role) -> u8` with 0xC3/0xA5, 0x00 and 0x57 → None).
- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rmk && cargo nextest run --target x86_64-pc-windows-msvc -p rmk role::tests`
Expected: FAIL, functions missing.

- [ ] **Step 3: Implement the helpers, the statics, `init`/`current_*`, `read_boot_role`/`reset_into` (nrf-only cfg), `apply_handoff_received`**.
- [ ] **Step 4: Append `SplitMessage::RoleHandoff { new_central_is_left: bool }`; handle it in `driver.rs` (central side) and `peripheral.rs` (peripheral side) by calling `apply_handoff_received`. In `peripheral.rs`'s `read_message_to_send` select, add `ROLE_HANDOFF_TX.wait()` → `SplitMessage::RoleHandoff { new_central_is_left }`; in the central driver's outgoing select (`driver.rs:170-195`) add the same arm.**
- [ ] **Step 5: Replace `crate::SPLIT_CENTRAL_IS_LEFT` in `via/mod.rs` with `crate::split::role::central_is_left_now()`; keep `battery_halves_for_split`'s bool parameter and its tests.**
- [ ] **Step 6: Run the full rmk host test suite**

Run: `cd rmk && cargo nextest run --target x86_64-pc-windows-msvc -p rmk`
Expected: all pass (count previously ≥ the number before this task; note the number in the commit).

- [ ] **Step 7: Commit**

```bash
git -C rmk add rmk/src/split rmk/src/host/via/mod.rs
git -C rmk commit -m "feat(split): RoleHandoff message, boot-role register and runtime side registry"
```

---

### Task 4: Dual-role codegen (`dynamic_role`) with USB enumeration in the peripheral role

**Files:**
- Modify: `rmk/rmk-config/src/lib.rs:849-855` (`SplitConfig.dynamic_role: Option<bool>`)
- Modify: `rmk/rmk-macro/src/codegen/orchestrator.rs` (`expand_main` → also a variant returning the body as `pub async fn run(spawner)` inside `mod as_central`), `rmk/rmk-macro/src/codegen/split/peripheral.rs` (same for `mod as_peripheral`, plus USB device + role task), `rmk/rmk-macro/src/codegen/entry.rs` (role task joined into the central task list), `rmk/rmk-macro/src/lib.rs` or wherever `rmk_central`/`rmk_peripheral` dispatch
- Modify: `rmk/rmk/src/usb/mod.rs:632-670` (`UsbDeviceHandler::configured` → `role::LOCAL_USB_CONFIGURED.store(true)`, `reset`/`suspended`/`disabled` → `false`)
- Create: `rmk/rmk/src/split/role_task.rs` (`pub async fn run_role_task(side: Side, role: Role) -> !`)
- Modify: `rmk/keyboards/op36/keyboard.toml` (`[split] dynamic_role = true`)
- Test: `rmk/rmk-macro` unit tests for the config view swap

**Interfaces:**
- Produces (rmk-macro, pure): `fn swapped_split_view(cfg: &SplitConfig) -> SplitConfig` returning a config whose `central` is `cfg.peripheral[0]` (pins, rows, cols, `row_offset` 3) and whose `peripheral[0]` is `cfg.central` (`row_offset` 0); `fn left_as_peripheral_view(cfg) -> SplitConfig` whose `peripheral[0]` is `cfg.central`.
- Produces (generated code shape, both bins):
  ```rust
  mod as_central { /* imports, bind_interrupts!(struct Irqs ...), statics */ pub async fn run(spawner: Spawner) -> ! { /* today's central main body + ::rmk::split::role_task::run_role_task(SIDE, Role::Central) joined */ } }
  mod as_peripheral { /* ... */ pub async fn run(spawner: Spawner) -> ! { /* today's peripheral body + usb device task + run_role_task(SIDE, Role::Peripheral) */ } }
  #[::embassy_executor::main] async fn main(spawner: Spawner) {
      let role = ::rmk::split::role::read_boot_role().unwrap_or(::rmk::split::role::default_role(SIDE));
      ::rmk::split::role::init(SIDE, role);
      match role { Role::Central => as_central::run(spawner).await, Role::Peripheral => as_peripheral::run(spawner).await }
  }
  ```
  `SIDE` is `Side::Left` for the `rmk_central` bin and `Side::Right` for the `rmk_peripheral(id = 0)` bin. The right half's `as_central` uses `CommunicationConfig::Usb` for `transport_setup` (no `BleTransport`), while the chip init still builds the SDC stack for the split link.
- Produces (role_task.rs): polls every 250 ms: `local_usb_configured = LOCAL_USB_CONFIGURED.load()`, `vbus = POWER.usbregstatus().read().vbusdetect()`, `link_up` = central: `crate::split::ble::central::all_peripherals_connected()`; peripheral: a new `pub(crate) static PERIPHERAL_LINK_UP: AtomicBool` set in `split/peripheral.rs` on connect/disconnect, `central_has_usb_host = current_usb_state() == Configured` (peripheral role only), `peer_claim_seen = PEER_CLAIM_SEEN.try_take().is_some()`. On `TakeOver`/`HandBack`: if `link_up` signal `ROLE_HANDOFF_TX` with `new_central_is_left`, wait `HANDOFF_SETTLE_MS`, then `reset_into(new role)`; on `Yield`: `reset_into(Peripheral)`. Logs each decision at info level ("role: take over", "role: hand back", "role: yield").
- Consumes: Tasks 2–3.

- [ ] **Step 1: Write failing rmk-macro tests** `swapped_view_moves_right_pins_to_central_with_offset_3` and `left_as_peripheral_view_keeps_left_pins_with_offset_0` (build a `SplitConfig` with two distinct `MatrixConfig`s and assert pins/offsets on the views).
- [ ] **Step 2: Run them to verify they fail**

Run: `cd rmk && cargo test -p rmk-macro views` (rmk-macro is a host proc-macro crate; plain cargo test is fine there).
Expected: FAIL, functions missing.

- [ ] **Step 3: Implement the views and `dynamic_role` parsing; emit the `as_central`/`as_peripheral` modules and the dispatching `main` when `dynamic_role` is true (unchanged output when false).** Peripheral module additions: `usb_init` (same driver codegen as the central, `usb_config_default`), a `::rmk::usb::UsbTransport::new(driver, device_config).run()` task so the host enumerates a HID device, and the role task.
- [ ] **Step 4: Wire `LOCAL_USB_CONFIGURED` in `usb/mod.rs` and implement `role_task.rs`; add `pub mod role_task;` to `split/mod.rs`.**
- [ ] **Step 5: Run macro tests and the rmk host suite**

Run: `cd rmk && cargo test -p rmk-macro && cargo nextest run --target x86_64-pc-windows-msvc -p rmk`
Expected: all pass.

- [ ] **Step 6: Set `dynamic_role = true` in `keyboards/op36/keyboard.toml`, build both bins**

Run: `cd rmk/keyboards/op36 && cargo build --release --bin central --bin peripheral 2>&1 | grep -E "error|warning: unused|Finished" | head`
Expected: `Finished`; fix any name clashes between the two generated modules (each module owns its own `Irqs`, statics and `use` lines).

- [ ] **Step 7: Commit**

```bash
git -C rmk add rmk-config rmk-macro rmk/src keyboards/op36/keyboard.toml
git -C rmk commit -m "feat(split): dynamic_role dual-role firmware with boot-time role selection"
```

---

### Task 5: Master-claim advertisement and recognition

**Files:**
- Modify: `rmk/rmk/src/split/ble/central.rs` (`on_adv_reports` at line ~491: parse manufacturer data; signal `PEER_CLAIM_SEEN` when the report address equals the stored peer address and the payload is a claim), new `pub async fn run_master_claim(stack, ...)` task
- Modify: `rmk/rmk-macro/src/codegen/entry.rs` (join `run_master_claim` into the central task list when `dynamic_role`)
- Test: `rmk/rmk/src/split/ble/central.rs` tests

**Interfaces:**
- Produces: `pub(crate) const CLAIM_MAGIC: [u8; 4] = *b"OP36";` claim payload = `ManufacturerSpecificData { company_identifier: <same id the split peripheral advert uses at peripheral.rs:444>, payload: [CLAIM_MAGIC, product_id_lo, product_id_hi, side_byte(0 = left, 1 = right)] }`; `pub(crate) fn parse_claim(ad: &[u8]) -> Option<Side>` (pure); `run_master_claim`: while `current_role() == Central` and `!all_peripherals_connected()`, advertise non-connectable, non-scannable, undirected for 1 s every 5 s; idle otherwise.
- Consumes: Task 3 statics.

- [ ] **Step 1: Write the failing test** `parse_claim_accepts_only_this_product_and_magic` (valid payload → `Some(Side::Right)`; wrong magic, wrong product id, short payload → `None`).
- [ ] **Step 2: Run it to verify it fails**

Run: `cd rmk && cargo nextest run --target x86_64-pc-windows-msvc -p rmk parse_claim`
Expected: FAIL.

- [ ] **Step 3: Implement `parse_claim`, the `on_adv_reports` hook (report address must equal `addrs[0]`), and `run_master_claim`.** If the SDC cannot run a second advertising set next to the host advertisement on the left half, fall back to embedding the claim manufacturer data into the existing host advertisement (`ble/mod.rs:949-960`) while `!all_peripherals_connected()`; the right half (USB-only central) uses the dedicated non-connectable set. State which path was taken in the commit message.
- [ ] **Step 4: Run tests and build both bins**

Run: `cd rmk && cargo nextest run --target x86_64-pc-windows-msvc -p rmk && cd keyboards/op36 && cargo build --release --bin central --bin peripheral 2>&1 | tail -2`
Expected: tests pass, `Finished`.

- [ ] **Step 5: Commit**

```bash
git -C rmk add rmk/src/split/ble/central.rs rmk-macro/src/codegen/entry.rs
git -C rmk commit -m "feat(split): master-claim advertisement settles swaps made while the link was down"
```

---

### Task 6: Vial charging bits and layer-notify handoff notification

**Files:**
- Modify: `rmk/rmk/src/host/via/mod.rs:604-630` (flags bits 2/3)
- Modify: `rmk/rmk/src/host/layer_notify.rs` (`SUB_HANDOFF = 0x02`, `encode_handoff`, `notify_handoff`, `fw_layout` bit 7)
- Modify: `rmk/rmk/src/split/role_task.rs` (call `notify_handoff` before resetting)
- Test: both modules' test sections

**Interfaces:**
- Produces: `pub(crate) fn battery_flags(left: BatteryStatus, right: BatteryStatus) -> u8` (bit0/1 present = `battery_level_byte(..).is_some()`, bit2/3 = `charge_state == ChargeState::Charging`); the reply writes `report.input_data[4] = battery_flags(left, right)`.
- Produces (layer_notify): `pub(crate) const SUB_HANDOFF: u8 = 0x02;` `pub(crate) fn encode_handoff(new_central_is_left: bool) -> [u8; 32]` = `E9 02 side` (side byte 0 = left, 1 = right), `pub(crate) fn notify_handoff(new_central_is_left: bool, now_ms: u64)` sent to the active subscriber if any, otherwise to the current host transport; `pub(crate) const FW_LAYOUT_CENTRAL_RIGHT: u8 = 0x80;` and `encode_state` sets it from `role::central_is_left_now()`.

- [ ] **Step 1: Write failing tests** `battery_flags_mark_presence_and_charging` (left 85 % charging, right 40 % discharging → `0b0000_0111`; right unavailable → `0b0000_0101`) and `encode_handoff_layout` (`encode_handoff(false)[..3] == [0xE9, 0x02, 1]`) and `encode_state_sets_central_right_bit_when_right_is_central`.
- [ ] **Step 2: Run to verify they fail**

Run: `cd rmk && cargo nextest run --target x86_64-pc-windows-msvc -p rmk battery_flags encode_handoff encode_state`
Expected: FAIL.

- [ ] **Step 3: Implement; call `notify_handoff` from the role task right before `reset_into` on `TakeOver`/`HandBack`.**
- [ ] **Step 4: Run the suite** — Expected: all pass.
- [ ] **Step 5: Commit**

```bash
git -C rmk add rmk/src/host
git -C rmk commit -m "feat(host): charging flags in the battery reply, handoff notification and central-side bit"
```

---

### Task 7: Build, runbook, flash both halves, write the layout into the right half

**Files:**
- Modify: `firmware/FLASHING.md` (dual-role section: default roles, cable rule, both halves must be flashed, `--write-keymap-zmk` once per half while it is master, how to force a role for diagnosis: hold the cable in the half you want)
- Modify: `firmware/tools/build-op36.sh` (no change expected; confirm both bins build with `dynamic_role`)

- [ ] **Step 1: Build**

Run: `cd omega36 && bash firmware/tools/build-op36.sh 2>&1 | tail -4`
Expected: two UF2 files written, sizes printed, no errors.

- [ ] **Step 2: Flash the left half, then the right half** (Boris: Bootloader key or double reset; copy `firmware/op36-layer-notify/op36_left.uf2` to the left drive, `op36_right.uf2` to the right drive). Confirm both reboot and the overlay reconnects.
- [ ] **Step 3: Write the layout into the right half**: plug the cable into the right half, wait for it to become master (helper shows the USB glyph on the right), exit the tray helper, run `helper\target\release\op36-helper.exe --write-keymap-zmk`, expect `verification: 0 mismatches`, restart the helper.
- [ ] **Step 4: Update `FLASHING.md` and commit (omega36 repo)**

```bash
git add firmware/FLASHING.md firmware/op36-layer-notify
git commit -m "docs(firmware): dual-role flashing runbook and new builds"
```

---

### Task 8: On-device acceptance

Record results in `docs/superpowers/plans/2026-10-30-dynamic-master-acceptance.md`, one row per spec acceptance item 1–9 (incl. 7b), with the firmware log lines (`role: ...`) captured over USB serial from the master half (`.venv/Scripts/python -I firmware/tools/serial_log.py COMx 60`).

- [ ] **Step 1: Items 1, 2, 3, 4** (cold start; cable right; cable left; unplug right) — expected times from the spec (swap ≤ 4 s, hand-back ≤ 7 s).
- [ ] **Step 2: Items 5, 6** (wall charger: no swap, bolt shows; KVM host switching: no swap).
- [ ] **Step 3: Items 7, 7b** (right asleep, link down, cable in → claim path ≤ 25 s; work setup).
- [ ] **Step 4: Items 8, 9** (`le` ordering on both masters; combos; logs).
- [ ] **Step 5: Commit the acceptance record.** Any failure goes back to the owning task with the log excerpt.
