# Dynamic Master Half — Helper Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The overlay shows the USB/Bluetooth glyph on whichever half is master, a charging bolt on any half on USB power, and a "switching master" state while the halves swap roles.

**Architecture:** Pure protocol parsing gains the two charging bits, the central-side bit and the handoff notification; the model tracks link side, per-half charging and a switching timestamp; the renderer draws the bolt, places the glyph by side, and animates the switch on a timer. No settings change.

**Tech Stack:** Rust, windows crate 0.62, Direct2D/DirectWrite; tests with `cargo test` in `omega36/helper`; visual check with `--snapshot` and the PrintWindow capture script (`_scratch/capture_and_dialog.ps1`).

**Spec:** `docs/superpowers/specs/2026-10-29-dynamic-master-design.md` (sections "What the host and the helper see", "Charging indicator", "Helper changes")

## Global Constraints

- Protocol bytes (from the firmware plan): battery reply `08 E8 01 ver flags left right`, flags bit0/1 present, bit2/3 left/right on USB power. Layer state `E9 01 active default mask_lo mask_hi fw_layout`, `fw_layout` bit 7 (`0x80`) = central is right. Handoff notification `E9 02 side` (0 = left becomes master, 1 = right).
- Switching state lasts until the next `Connected`, or 15 s (`OFFLINE_GRACE_MS`), whichever comes first; the existing offline grace must not show "offline" during a swap.
- Overlay size never changes (no jumping). Snapshot CLI stays offline-renderable.
- Do not modify `%APPDATA%\op36-helper\settings.json` by hand or via tests.

## Review Focus

1. Firmware without the new bits (today's build) must render exactly as now: no bolt, glyph on the left, `fw_layout` bit 7 clear. Task 1 test `old_firmware_reply_has_no_charging_or_right_central`.
2. Handoff packet arrives, then the USB device disappears for 3 s, then reconnects: the overlay must show switching, never "offline", and end in the connected state with the new side. Task 2 test `switching_hides_offline_until_reconnect`.
3. Handoff packet arrives but nothing reconnects (swap failed): after 15 s the normal offline state must appear. Task 2 test `switching_times_out_to_offline`.
4. Battery for a half can be present and charging but the other half absent (right half off): bolt only on the present half, no crash on `None`. Task 3 snapshot check with `--snapshot ... charging=left`.
5. The handoff packet may arrive while the HID thread is inside a battery request: it must be routed to the model, not dropped (uses the stray-packet path from `request_collect`). Task 2 test `handoff_in_stray_packets_is_delivered` (hid_thread routing function made pure).

---

### Task 1: Protocol parsing — charging bits, central side, handoff

**Files:**
- Modify: `helper/src/vial/protocol.rs` (`BatteryHalves`, `parse_battery_halves`, `LayerState`, `parse_layer_state`, new `parse_handoff`)
- Modify: `helper/src/vial/layer_client.rs` (`on_packet` returns `Option<LayerPacket>`)
- Test: same files

**Interfaces:**
- Produces:
  ```rust
  pub struct BatteryHalves { pub left: Option<u8>, pub right: Option<u8>, pub left_charging: bool, pub right_charging: bool }
  pub struct LayerState { pub active: u8, pub default: u8, pub mask: u16, pub fw_layout: u8 /* bit 7 stripped */, pub central_is_left: bool }
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Side { Left, Right }
  pub const LAYER_NOTIFY_SUB_HANDOFF: u8 = 0x02;
  pub fn parse_handoff(pkt: &Packet) -> Option<Side>;        // E9 02 side
  pub enum LayerPacket { State(LayerState), Handoff(Side) }
  impl LayerClient { pub fn on_packet(&mut self, pkt: &Packet) -> Option<LayerPacket>; }  // Handoff also marks supported = Some(true)
  ```

- [ ] **Step 1: Write the failing tests** in `protocol.rs`:

```rust
#[test] fn battery_reply_carries_charging_bits() {
    let mut r = [0u8; 32]; r[..7].copy_from_slice(&[0x08, 0xE8, 0x01, 1, 0b0000_0111, 85, 40]);
    let b = parse_battery_halves(&r).unwrap();
    assert_eq!((b.left, b.right, b.left_charging, b.right_charging), (Some(85), Some(40), true, true));
}
#[test] fn old_firmware_reply_has_no_charging_or_right_central() {
    let mut r = [0u8; 32]; r[..7].copy_from_slice(&[0x08, 0xE8, 0x01, 1, 0b11, 85, 40]);
    let b = parse_battery_halves(&r).unwrap();
    assert!(!b.left_charging && !b.right_charging);
    let mut s = [0u8; 32]; s[..7].copy_from_slice(&[0xE9, 0x01, 2, 0, 0x04, 0x00, 1]);
    let st = parse_layer_state(&s).unwrap();
    assert!(st.central_is_left); assert_eq!(st.fw_layout, 1);
}
#[test] fn layer_state_bit7_means_right_half_is_central() {
    let mut s = [0u8; 32]; s[..7].copy_from_slice(&[0xE9, 0x01, 0, 0, 0x01, 0x00, 0x81]);
    let st = parse_layer_state(&s).unwrap();
    assert!(!st.central_is_left); assert_eq!(st.fw_layout, 1);
}
#[test] fn handoff_packet_names_the_new_master() {
    let mut p = [0u8; 32]; p[..3].copy_from_slice(&[0xE9, 0x02, 1]);
    assert_eq!(parse_handoff(&p), Some(Side::Right));
    p[2] = 0; assert_eq!(parse_handoff(&p), Some(Side::Left));
    p[1] = 0x01; assert_eq!(parse_handoff(&p), None);
}
```
and in `layer_client.rs`: `on_packet_distinguishes_state_and_handoff`.

- [ ] **Step 2: Run to verify they fail** — `cd helper && cargo test protocol::tests layer_client` → compile errors on new fields.
- [ ] **Step 3: Implement** (strip bit 7 into `central_is_left`; `LayerPacket`; update every construction of `BatteryHalves`/`LayerState` in `main.rs` snapshot code and tests).
- [ ] **Step 4: Run `cargo test`** — Expected: all pass (count ≥ 120 + 5).
- [ ] **Step 5: Commit**

```bash
git add helper/src/vial
git commit -m "feat(helper): parse charging bits, central side and the handoff notification"
```

---

### Task 2: Model — link side, charging flags, switching state

**Files:**
- Modify: `helper/src/model.rs` (`HidEvent::Switching(Side)`, `Model.switching: Option<u64 /*ms*/>`, `Model.central_is_left: bool`, `ModelEvent::Tick(u64)` or explicit `switching_expired()`), `helper/src/overlay/frame.rs` (`Frame.link_side: Side`, `Frame.switching: bool`, `Frame.battery_left_charging: bool`, `Frame.battery_right_charging: bool`), `helper/src/hid_thread.rs` (route `LayerPacket::Handoff` → `HidEvent::Switching`, also from the stray packets of `request_collect`)
- Test: `model.rs` tests, `hid_thread.rs` new pure routing test

**Interfaces:**
- Produces:
  ```rust
  // model.rs
  pub enum HidEvent { ..., Switching(Side) }
  pub const SWITCHING_TIMEOUT_MS: u64 = 15_000;
  impl Model {
      pub fn apply_at(&mut self, event: ModelEvent, now_ms: u64);   // apply() = apply_at(event, 0) for existing tests
      pub fn is_switching(&self, now_ms: u64) -> bool;              // switching set and now - started < timeout and !connected-since
  }
  // Semantics: Switching(side) sets switching = Some(now), pending_side = side, and suppresses
  // Disconnected → offline for SWITCHING_TIMEOUT_MS; Connected clears switching and sets
  // central_is_left from the next LayerState (until then from pending_side).
  // hid_thread.rs
  pub fn route_layer_packet(pkt: &LayerPacket) -> HidEvent;        // State → Layer, Handoff → Switching
  ```
  Frame: `link_side` = `Side::Left` if `central_is_left` else `Side::Right`; `switching` = `is_switching(now)`; `status_line()` says `switching master…` while switching; battery charging flags copied from the last `BatteryHalves` (only while connected).
- Consumes: Task 1 types.

- [ ] **Step 1: Write failing tests** in `model.rs`:

```rust
#[test] fn switching_hides_offline_until_reconnect() {
    let mut m = connected_model();
    m.apply_at(ModelEvent::Hid(HidEvent::Switching(Side::Right)), 1_000);
    m.apply_at(ModelEvent::Hid(HidEvent::Disconnected), 1_500);
    assert!(m.is_switching(4_000));
    assert!(!m.status_line_at(4_000).contains("offline"), "{}", m.status_line_at(4_000));
    assert!(m.status_line_at(4_000).contains("switching"));
    m.apply_at(ModelEvent::Hid(HidEvent::Connected(Transport::Usb)), 4_500);
    assert!(!m.is_switching(4_600));
    assert_eq!(m.frame_at(&FakeCharSource::default(), 4_600).link_side, Side::Right);
}
#[test] fn switching_times_out_to_offline() {
    let mut m = connected_model();
    m.apply_at(ModelEvent::Hid(HidEvent::Switching(Side::Right)), 0);
    m.apply_at(ModelEvent::Hid(HidEvent::Disconnected), 100);
    assert!(m.is_switching(14_999));
    assert!(!m.is_switching(15_000));
    assert!(m.status_line_at(15_000).ends_with("offline"), "{}", m.status_line_at(15_000));
}
#[test] fn layer_state_bit_moves_the_link_side() {
    let mut m = connected_model();
    m.apply(ModelEvent::Hid(HidEvent::Layer(LayerState { active: 0, default: 0, mask: 1, fw_layout: 1, central_is_left: false })));
    assert_eq!(m.frame(&FakeCharSource::default()).link_side, Side::Right);
}
#[test] fn charging_flags_reach_the_frame_only_while_connected() { /* left charging true → frame.battery_left_charging; Disconnected → both false */ }
```
and in `hid_thread.rs`: `handoff_in_stray_packets_is_delivered` (`route_layer_packet(&LayerPacket::Handoff(Side::Left)) == HidEvent::Switching(Side::Left)`).

- [ ] **Step 2: Run to verify they fail** — `cargo test model::tests hid_thread` → compile errors.
- [ ] **Step 3: Implement** (`apply_at`, `status_line_at`, `frame_at`; keep `apply`, `status_line`, `frame` as `now = 0` wrappers so the 120 existing tests stay untouched). In `hid_thread.rs` use `route_layer_packet` both in the main read loop and for `request_collect` strays.
- [ ] **Step 4: Run `cargo test`** — Expected: all pass.
- [ ] **Step 5: Commit**

```bash
git add helper/src/model.rs helper/src/overlay/frame.rs helper/src/hid_thread.rs
git commit -m "feat(helper): switching state, link side and charging flags in the model"
```

---

### Task 3: Renderer — bolt, glyph by side, switching indicator; main-thread timer

**Files:**
- Modify: `helper/src/overlay/render_d2d.rs` (`draw_battery(hand, percent, charging)`, `draw_link(link, side)`, `draw_switching(phase)`), `helper/src/overlay/geometry.rs` (`pub fn link_glyph_rect(hand: Hand) -> (f32, f32, f32, f32)` beside the battery on either half, mirrored), `helper/src/main.rs` (`TIMER_SWITCH = 6`, 500 ms while `frame.switching`, `redraw` passes `now_ms`; `--snapshot` gains optional tokens `charging=left|right|both`, `master=right`, `switching`)
- Test: `geometry.rs` test for the mirrored glyph rect; visual check via snapshot and live capture

**Interfaces:**
- Consumes: `Frame.link_side`, `Frame.switching`, `Frame.battery_*_charging` from Task 2.
- Produces (visual): bolt = white filled polygon (7 points, ~8×14 px) at the left inside the battery body, number shifted right by 6 px when charging; glyph drawn at `link_glyph_rect(hand of link_side)`; switching = both glyph slots empty and a pulsing double arrow `⇄` (Segoe UI, 22 px) centred in the gap at the battery row, alpha alternating 1.0/0.4 per 500 ms tick; tooltip text comes from `status_line_at`.

- [ ] **Step 1: Write the failing geometry test** `link_glyph_rect_mirrors_like_the_battery` (`l.0 + l.2 + r.0 == 2*HALF_W + GAP`, both inside `case_outline` hull with `point_in_convex`).
- [ ] **Step 2: Run to verify it fails** — `cargo test geometry`.
- [ ] **Step 3: Implement geometry, renderer and the timer; extend the snapshot CLI.**
- [ ] **Step 4: Run `cargo test`, then render three snapshots and look at them**

Run:
```bash
cd helper && cargo run -q -- --snapshot ../_scratch/snap_charging.png EN 0 1.0 charging=both
cargo run -q -- --snapshot ../_scratch/snap_master_right.png EN 0 1.0 master=right charging=right
cargo run -q -- --snapshot ../_scratch/snap_switching.png EN 0 1.0 switching
```
Expected: bolts inside both batteries with the number still readable; USB glyph beside the right battery in the second; no glyph and the arrow in the gap in the third; overlay size unchanged (1210×410 at scale 1.0).

- [ ] **Step 5: Build release, relaunch, capture live** (`taskkill //IM op36-helper.exe //F; cargo build --release; start the release exe; powershell -File ../_scratch/capture_and_dialog.ps1`) — Expected: today's firmware renders exactly as before (Review Focus 1).
- [ ] **Step 6: Commit**

```bash
git add helper/src
git commit -m "feat(helper): charging bolt, link glyph on the master half, switching indicator"
```

---

### Task 4: Docs and acceptance hook

**Files:**
- Modify: `helper/README.md` (bolt, glyph side, switching state, snapshot tokens), `docs/superpowers/plans/2026-10-28-helper-acceptance.md` (append rows: bolt, glyph side, switching) — filled in during firmware Task 8.

- [ ] **Step 1: Update the docs and commit**

```bash
git add helper/README.md docs/superpowers/plans/2026-10-28-helper-acceptance.md
git commit -m "docs(helper): dynamic master and charging indicator"
```
