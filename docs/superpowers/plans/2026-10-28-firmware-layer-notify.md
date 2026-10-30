# OP36 Firmware "Layer Notify" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the OP36's RMK firmware push its active layer to the host over the Vial raw-HID channel, opt-in with a keepalive, and flash it to the left half.

**Architecture:** A new feature-gated module `rmk/src/host/layer_notify.rs` owns the subscription (transport + deadline) and the 32-byte packet format. The keymap calls it whenever the resolved active layer changes; the Via dispatcher gains two tiny branches (subscribe/renew, get state). Nothing new subscribes to events, so no channel sizes change. Work happens in a full clone of the vendor fork `ergohaven/rmk`, branched from the exact commit the shipped v0.2.0 was built from, pushed to a fork under the user's GitHub account.

**Tech Stack:** Rust stable (1.99), `thumbv7em-none-eabihf`, embassy, RMK fork; host-side unit tests with `cargo test --target x86_64-pc-windows-msvc`; `llvm-objcopy` + `uf2conv.py` for the UF2; GitHub Actions in the fork as the fallback build.

**Spec:** `docs/superpowers/specs/2026-10-28-op36-layout-helper-design.md` (§3 facts, §5 firmware patch, §9 deferred items)

## Global Constraints

- Packet formats exactly as spec §5: host→kb `0x07 0xE9 0x01 ttl_s` (subscribe/renew, `ttl_s = 0` unsubscribes), host→kb `0x08 0xE9 0x01` (get state), kb→host `0xE9 0x01 active default lo hi fw_layout`, all 32 bytes, unused bytes zero.
- Namespace byte `0xE9`, sub-id `0x01`. Never touch bytes `0xE8`, `0xAA…0xAE`, `0x01…0x15`, `0xFE`.
- Helper renews every 20 s with TTL 60 s; firmware clamps TTL to 120 s max.
- Key path must stay non-blocking: only `try_send_host_reply`, drop on full queue.
- Minimal, rebase-friendly edits to existing files; new code behind Cargo feature `layer_notify` in the `rmk` crate, enabled only for `keyboards/op36`.
- Branch point: commit `347cf14f48cf994cb7f33ef2e9e026e59a7e2e4e` of `ergohaven/rmk` (the shipped v0.2.0 build). Branch name `op36-layer-notify`.
- Firmware repo clone lives at `C:\Users\boris_kudriashov\Source\omega36\rmk` (its own git repo; git-ignored by the parent `omega36` repo).
- Flash only the **left** half (central). The right half stays on stock v0.2.0.
- Rollback = stock `firmware/op36-v0.2.0/op36_left.uf2` in the `omega36` repo.
- Repo-wide `.cargo/config.toml` targets the nRF; every host test command must pass `--target x86_64-pc-windows-msvc`.
- Host tests must run under **`cargo nextest`** (embassy-time's mock driver is a process-global singleton; the test binary refuses to run under plain `cargo test`). Canonical command, from `rmk/rmk`:
  `cargo nextest run --target x86_64-pc-windows-msvc --config-file ../.config/nextest.toml --no-default-features --features=split,vial,storage,async_matrix,_ble[,layer_notify] -E 'test(<filter>)'`
  Every "Run:" line below that says `cargo test … --lib <filter>` means this command with `-E 'test(<filter>)'`.

## Review Focus

- A host that subscribes and then dies: after the TTL passes the keyboard must go silent again (Task 2 test `expired_subscription_is_dropped`).
- A subscribe packet with TTL above the cap, e.g. 255: must clamp to 120 s, not reject (Task 2 test `ttl_is_clamped_to_max`).
- Layer change while no host is subscribed: nothing is sent and nothing panics (Task 3 test `no_subscriber_sends_nothing`).
- Reply queue full when a layer changes: the notification is dropped, the key path continues (Task 3 test `full_reply_queue_drops_notification`).
- Tri-layer: holding both thumb keys must report `active = 3` and `mask = 0b1110` (Task 3 test `layer_mask_reports_all_active_layers`).

---

### Task 1: Repositories and branch

**Files:**
- Create: `C:\Users\boris_kudriashov\Source\omega36\.gitignore`
- Create: `C:\Users\boris_kudriashov\Source\omega36\rmk\` (clone, own repo)

**Interfaces:**
- Produces: the `omega36` git repo with the design, plans, firmware release files and ZMK backups committed; the `rmk` clone on branch `op36-layer-notify` at `347cf14f` with remotes `upstream` = ergohaven/rmk and `origin` = yeroo/rmk.

- [ ] **Step 1: Initialise the `omega36` repo**

```bash
cd /c/Users/boris_kudriashov/Source/omega36
git init -b main
printf '%s\n' '_ref/' '_scratch/' '.superpowers/' 'rmk/' 'helper/target/' '*.png.bak' > .gitignore
git add .gitignore docs firmware 1.png 2.png 3.png 4.png
git commit -m "docs: OP36 layout helper design, plans, firmware files and ZMK backups"
```

Expected: one commit; `git status` clean except ignored dirs.

- [ ] **Step 2: Fork and clone the vendor firmware**

```bash
cd /c/Users/boris_kudriashov/Source/omega36
gh repo fork ergohaven/rmk --clone=false
git clone https://github.com/yeroo/rmk.git rmk
cd rmk
git remote add upstream https://github.com/ergohaven/rmk.git
git fetch upstream
git checkout -b op36-layer-notify 347cf14f48cf994cb7f33ef2e9e026e59a7e2e4e
git log -1 --format='%H %s'
```

Expected: last line starts with `347cf14f48cf994cb7f33ef2e9e026e59a7e2e4e`.

- [ ] **Step 3: Confirm the host test baseline compiles and passes**

Run (from `rmk/rmk`): `cargo test --target x86_64-pc-windows-msvc --no-default-features --features=split,vial,storage,async_matrix,_ble --lib host::via`
Expected: `test result: ok.` (existing Via tests; first compile takes several minutes).

---

### Task 2: `layer_notify` module — packet encoding and subscription

**Files:**
- Create: `rmk/rmk/src/host/layer_notify.rs`
- Modify: `rmk/rmk/src/host/mod.rs` (add `#[cfg(feature = "layer_notify")] pub(crate) mod layer_notify;`)
- Modify: `rmk/rmk/Cargo.toml` `[features]` (add `layer_notify = ["host"]` with a one-line `##` doc comment like the neighbours)

**Interfaces:**
- Produces (all `pub(crate)` in `crate::host::layer_notify`):
  - `const NAMESPACE: u8 = 0xE9; const SUB_STATE: u8 = 0x01; const MAX_TTL_SECS: u8 = 120;`
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] struct LayerState { active: u8, default: u8, mask: u16, fw_layout: u8 }`
  - `fn encode_state(state: LayerState) -> [u8; 32]`
  - `fn subscribe(transport: HostTransport, ttl_secs: u8, now_ms: u64)` — `ttl_secs == 0` clears; otherwise stores `transport` and `deadline_ms = now_ms + min(ttl_secs, MAX_TTL_SECS) * 1000`.
  - `fn active_subscriber(now_ms: u64) -> Option<HostTransport>` — `None` and clears the store when `now_ms >= deadline_ms`.
  - `fn notify(state: LayerState, now_ms: u64)` — if `active_subscriber(now_ms)` is `Some(t)`, calls `crate::channel::try_send_host_reply(t, encode_state(state))`.
- Storage: one `static SUBSCRIPTION: embassy_sync::blocking_mutex::Mutex<CriticalSectionRawMutex, RefCell<Option<Subscription>>>` with `struct Subscription { transport: HostTransport, deadline_ms: u64 }` (same pattern as `host_data.rs`).

- [ ] **Step 1: Write the failing tests** (in `layer_notify.rs`, `#[cfg(test)] mod tests`)

```rust
#[test]
fn encode_state_matches_spec_layout() {
    let bytes = encode_state(LayerState { active: 3, default: 0, mask: 0b0000_1110, fw_layout: 1 });
    assert_eq!(&bytes[..7], &[0xE9, 0x01, 3, 0, 0x0E, 0x00, 1]);
    assert!(bytes[7..].iter().all(|b| *b == 0));
}

#[test]
fn subscription_is_active_until_deadline() {
    subscribe(HostTransport::Usb, 60, 1_000);
    assert_eq!(active_subscriber(60_999), Some(HostTransport::Usb));
    assert_eq!(active_subscriber(61_000), None);
}

#[test]
fn expired_subscription_is_dropped() {
    subscribe(HostTransport::Usb, 1, 0);
    assert_eq!(active_subscriber(1_000), None);
    assert_eq!(active_subscriber(0), None, "expiry clears the store, it is not time-reversible");
}

#[test]
fn ttl_zero_unsubscribes() {
    subscribe(HostTransport::Usb, 60, 0);
    subscribe(HostTransport::Usb, 0, 0);
    assert_eq!(active_subscriber(1), None);
}

#[test]
fn ttl_is_clamped_to_max() {
    subscribe(HostTransport::Usb, 255, 0);
    assert_eq!(active_subscriber(120_000 - 1), Some(HostTransport::Usb));
    assert_eq!(active_subscriber(120_000), None);
}
```

Tests share one static: serialise them with a `static LOCK: std::sync::Mutex<()>` guard taken at the top of each test (the Via tests use the same trick, `macro_signal_test_lock`).

- [ ] **Step 2: Run the tests to verify they fail**

Run (from `rmk/rmk`): `cargo test --target x86_64-pc-windows-msvc --no-default-features --features=split,vial,storage,async_matrix,_ble,layer_notify --lib host::layer_notify`
Expected: compile error, `layer_notify` not found (feature and module do not exist yet).

- [ ] **Step 3: Add the feature and module, implement the items in the Interfaces block**

`Cargo.toml`: `layer_notify = ["host"]`. `host/mod.rs`: gated `pub(crate) mod layer_notify;`. Implement `encode_state`, `subscribe`, `active_subscriber`, `notify` exactly as specified; `HostTransport` is `Copy`, so `Option<HostTransport>` is returned by value.

- [ ] **Step 4: Run the tests to verify they pass**

Same command as Step 2. Expected: `test result: ok. 5 passed`.

- [ ] **Step 5: Commit** (in the `rmk` clone)

```bash
git add rmk/Cargo.toml rmk/src/host/mod.rs rmk/src/host/layer_notify.rs
git commit -m "feat(host): layer_notify packet encoding and subscription store"
```

---

### Task 3: Emit notifications from the keymap

**Files:**
- Modify: `rmk/rmk/src/keymap.rs` — the two `publish_event(LayerChangeEvent::new(layer))` sites in `update_fn_layer_state` and `update_tri_layer` (lines ~297–311), plus a new accessor.
- Modify: `rmk/rmk/src/host/context.rs` — add `pub fn layer_mask(&self) -> u16` next to `active_layer()` (line ~352).
- Modify: `rmk/rmk/src/host_data.rs` — add `pub fn layout() -> Option<u8>` reading `HOST_LAYOUT` (same `known()` helper as `snapshot()`).
- Test: `rmk/rmk/src/host/layer_notify.rs` tests module (extend).

**Interfaces:**
- Consumes: Task 2 `notify`, `LayerState`, `subscribe`.
- Produces:
  - `KeyMap::layer_mask(&self) -> u16` (`pub`, bit `i` set when `layer_state[i]`, layers ≥ 16 ignored) in `keymap.rs`, and `KeyboardContext::layer_mask()` forwarding to it.
  - `KeyMap::default_layer()` already exists as `get_default_layer()`; use it.
  - A private `fn publish_layer_change(&mut self)` on the keymap inner type that replaces both duplicated `let layer = self.get_activated_layer(); publish_event(LayerChangeEvent::new(layer));` blocks and, under `#[cfg(feature = "layer_notify")]`, also calls `crate::host::layer_notify::notify(LayerState { active, default: self.behavior.default_layer, mask: <same bit fold>, fw_layout: crate::host_data::layout().unwrap_or(0) }, embassy_time::Instant::now().as_millis())`.
  - `fw_layout` byte: `0` when unknown/EN, `1` when RU, i.e. `host_data::layout().unwrap_or(0)`.

- [ ] **Step 1: Write the failing tests** (add to `layer_notify.rs` tests; build the keymap like `with_service` does in `via/mod.rs` tests, but with a 1×1×4 `KeymapData::new([[[KeyAction::No]]; 4])` so four layers exist)

```rust
#[test]
fn layer_mask_reports_all_active_layers() {
    let (keymap, _data, _behavior, _positional) = four_layer_keymap(); // test helper, returns KeyMap and the owned storage it borrows
    keymap.activate_layer(1);
    keymap.activate_layer(2);           // tri-layer rule turns layer 3 on
    assert_eq!(keymap.active_layer(), 3);
    assert_eq!(keymap.layer_mask(), 0b0000_1110);
}

#[test]
fn no_subscriber_sends_nothing() {
    drain_usb_replies();                // helper: loop over HOST_USB_REPLY.try_receive() until Err
    subscribe(HostTransport::Usb, 0, 0);
    notify(LayerState { active: 1, default: 0, mask: 0b10, fw_layout: 0 }, 0);
    assert!(crate::channel::HOST_USB_REPLY.try_receive().is_err());
}

#[test]
fn subscribed_layer_change_is_sent_on_usb() {
    drain_usb_replies();
    subscribe(HostTransport::Usb, 60, 0);
    notify(LayerState { active: 2, default: 0, mask: 0b100, fw_layout: 1 }, 10);
    let sent = crate::channel::HOST_USB_REPLY.try_receive().expect("one reply queued");
    assert_eq!(&sent[..7], &[0xE9, 0x01, 2, 0, 0x04, 0x00, 1]);
}

#[test]
fn full_reply_queue_drops_notification() {
    drain_usb_replies();
    subscribe(HostTransport::Usb, 60, 0);
    while crate::channel::HOST_USB_REPLY.try_send([0u8; 32]).is_ok() {}
    notify(LayerState { active: 1, default: 0, mask: 0b10, fw_layout: 0 }, 0); // must not panic
    assert_eq!(active_subscriber(0), Some(HostTransport::Usb), "dropping a packet does not unsubscribe");
}
```

`HOST_USB_REPLY` is `pub(crate)` in `channel.rs` under `#[cfg(not(feature = "_no_usb"))]`; the test features above keep USB enabled.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --target x86_64-pc-windows-msvc --no-default-features --features=split,vial,storage,async_matrix,_ble,layer_notify --lib host::layer_notify`
Expected: compile error, `layer_mask` not found.

- [ ] **Step 3: Implement `layer_mask`, `host_data::layout`, and `publish_layer_change` as specified**

- [ ] **Step 4: Run the tests to verify they pass**

Same command. Expected: `test result: ok. 9 passed`. Also run the baseline from Task 1 Step 3 to confirm nothing else broke.

- [ ] **Step 5: Commit**

```bash
git add rmk/src/keymap.rs rmk/src/host/context.rs rmk/src/host_data.rs rmk/src/host/layer_notify.rs
git commit -m "feat(keymap): notify a subscribed host on every layer change"
```

---

### Task 4: Via dispatcher — subscribe and get-state requests

**Files:**
- Modify: `rmk/rmk/src/host/via/mod.rs` — `VialService` struct and `new()` (line ~458), `CustomSetValue` / `CustomGetValue` branches (lines ~549–610), `Runnable::run` (line ~751), tests module.

**Interfaces:**
- Consumes: Task 2 `NAMESPACE`, `SUB_STATE`, `subscribe`, `encode_state`, `LayerState`; Task 3 `ctx.layer_mask()`, `ctx.active_layer()`, `ctx.default_layer()`, `host_data::layout()`.
- Produces:
  - `VialService` gains `last_transport: Option<HostTransport>` (gated on `layer_notify`), set in `run()` right after `HOST_REQUEST_CHANNEL.receive()` and before `process_via_packet`.
  - In `CustomSetValue`: when `output_data[1] == NAMESPACE && output_data[2] == SUB_STATE`: if `last_transport` is `Some(t)`, `subscribe(t, output_data[3], now_ms)`; then write `encode_state(current)` into `report.input_data` (whole 32 bytes, so the reply starts with `0xE9`, not the echoed command).
  - In `CustomGetValue`: same namespace/sub-id check, no subscribe, same reply.
  - `current = LayerState { active: ctx.active_layer(), default: ctx.default_layer(), mask: ctx.layer_mask(), fw_layout: host_data::layout().unwrap_or(0) }`.
  - Unknown sub-ids under `0xE9` fall through to the existing `warn!("Custom set value -- not supported")` path (reply = echo).

- [ ] **Step 1: Write the failing tests** (in `via/mod.rs` tests, using `with_service`; set `service.last_transport = Some(HostTransport::Usb)` inside the closure)

```rust
#[test]
fn layer_notify_subscribe_replies_with_state_and_subscribes() {
    let _g = layer_notify_test_lock();
    with_service(|service| {
        service.last_transport = Some(HostTransport::Usb);
        let mut report = raw_report([0x07, 0xE9, 0x01, 30]);
        block_on(service.process_via_packet(&mut report));
        assert_eq!(&report.input_data[..7], &[0xE9, 0x01, 0, 0, 0x00, 0x00, 0]); // mask excludes the default layer unless explicitly activated
        assert_eq!(crate::host::layer_notify::active_subscriber(29_999), Some(HostTransport::Usb));
        assert_eq!(crate::host::layer_notify::active_subscriber(30_000), None);
    });
}

#[test]
fn layer_notify_get_state_does_not_subscribe() {
    let _g = layer_notify_test_lock();
    with_service(|service| {
        service.last_transport = Some(HostTransport::Usb);
        crate::host::layer_notify::subscribe(HostTransport::Usb, 0, 0);
        let mut report = raw_report([0x08, 0xE9, 0x01, 0]);
        block_on(service.process_via_packet(&mut report));
        assert_eq!(&report.input_data[..2], &[0xE9, 0x01]);
        assert_eq!(crate::host::layer_notify::active_subscriber(0), None);
    });
}

#[test]
fn layer_notify_unknown_subid_is_echoed() {
    with_service(|service| {
        let mut report = raw_report([0x07, 0xE9, 0x02, 0]);
        block_on(service.process_via_packet(&mut report));
        assert_eq!(report.input_data[0], 0x07);
    });
}
```

`raw_report(prefix: [u8; 4]) -> ViaReport` is a new test helper mirroring `custom_report` but copying the four bytes verbatim. Note `active_subscriber` is evaluated against the `now_ms` the dispatcher used; make `now_ms` injectable by having the dispatcher call a `fn now_ms() -> u64` in `layer_notify` that returns `embassy_time::Instant::now().as_millis()` in firmware and `0` under `cfg(test)`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --target x86_64-pc-windows-msvc --no-default-features --features=split,vial,storage,async_matrix,_ble,layer_notify --lib host::via::tests::layer_notify`
Expected: compile error, `last_transport` not found.

- [ ] **Step 3: Implement the field, the `run()` assignment and the two dispatcher branches**

- [ ] **Step 4: Run the tests to verify they pass**

Same command. Expected: `3 passed`. Then run the full Via and layer_notify suites with and without the `layer_notify` feature to confirm the gate compiles both ways:
`cargo test --target x86_64-pc-windows-msvc --no-default-features --features=split,vial,storage,async_matrix,_ble --lib host::`
Expected: `test result: ok.` both times.

- [ ] **Step 5: Commit**

```bash
git add rmk/src/host/via/mod.rs rmk/src/host/layer_notify.rs
git commit -m "feat(via): layer_notify subscribe and get-state requests"
```

---

### Task 5: Enable for OP36 and build the UF2

**Files:**
- Modify: `rmk/keyboards/op36/Cargo.toml` — add `"layer_notify"` to the `rmk` dependency's feature list (line ~17–23).
- Create: `omega36/firmware/tools/build-op36.sh` (bash; runs from the `omega36` repo)
- Create: `omega36/firmware/tools/uf2conv.py`, `omega36/firmware/tools/uf2families.json` (downloaded, pinned by SHA256 noted in the script header)
- Output: `omega36/firmware/op36-layer-notify/op36_left.uf2` (+ `BUILD_COMMIT.txt`)

**Interfaces:**
- Produces: `firmware/tools/build-op36.sh [--ci]` that leaves `op36_left.uf2` in `firmware/op36-layer-notify/`. Default = local build; `--ci` = push the branch, run the vendor workflow in the fork with `gh workflow run build.yml --repo yeroo/rmk --ref op36-layer-notify`, wait, and download the `op36` artifact.

- [ ] **Step 1: Enable the feature and check the firmware compiles for the target**

Local build prerequisites, all verified on this machine on 2026-10-28 (the vendor only builds on Linux CI):
1. `scoop install gcc-arm-none-eabi` (done; 15.2.rel1) — bindgen for `nrf-sdc-sys`/`nrf-mpsl-sys` needs ARM newlib headers.
2. `cargo install flip-link` (done) — the `.cargo/config.toml` linker.
3. The cargo git checkout of `nrf-sdc` stores `nrf-sdc-sys/third_party` as a *symlink*, which Windows git materialises as a 27-byte text file. Replace it with a copy (done once for `~/.cargo/git/checkouts/nrf-sdc-60aaf678d91c8332/abe49d2`; redo if the checkout hash changes):
   ```bash
   d=~/.cargo/git/checkouts/nrf-sdc-*/abe49d2
   rm $d/nrf-sdc-sys/third_party && cp -r $d/nrf-mpsl-sys/third_party $d/nrf-sdc-sys/third_party
   ```
4. Environment for every build (put it in `build-op36.sh`):
   ```bash
   SYSROOT='C:\Users\boris_kudriashov\scoop\apps\gcc-arm-none-eabi\current\arm-none-eabi'
   export BINDGEN_EXTRA_CLANG_ARGS="--target=arm-none-eabi --sysroot=$SYSROOT -I$SYSROOT\include -IC:\Users\boris_kudriashov\scoop\apps\gcc-arm-none-eabi\current\lib\gcc\arm-none-eabi\15.2.1\include"
   ```

Run (from `rmk/keyboards/op36`): `cargo build --release --bin central`
Expected: `Finished release` (≈5 min warm, ≈12 min cold). The warning `[storage].start_addr = 0xcc000 has no effect with DFU` is normal. If the build fails for a new reason, switch to `--ci` and record the reason in `firmware/op36-layer-notify/BUILD_NOTES.md`.

- [ ] **Step 2: Write `build-op36.sh`**

Local path, mirroring `.github/workflows/build.yml` lines 157–180:
```bash
cargo build --release --bin central
llvm-objcopy -O binary target/thumbv7em-none-eabihf/release/central left.bin
python ../../../firmware/tools/uf2conv.py left.bin --base 0x26000 --family 0xADA52840 --output ../../../firmware/op36-layer-notify/op36_left.uf2
git rev-parse HEAD > ../../../firmware/op36-layer-notify/BUILD_COMMIT.txt
```
Download `uf2conv.py` and `uf2families.json` from `https://raw.githubusercontent.com/microsoft/uf2/master/utils/` once, into `firmware/tools/`, and commit them.

- [ ] **Step 3: Run the script and verify the output**

Run: `bash firmware/tools/build-op36.sh`
Expected: `firmware/op36-layer-notify/op36_left.uf2` exists, size within ±10 % of the stock `op36_left.uf2` (1 189 376 bytes; an unpatched local build of `main` gave 1 193 472), `BUILD_COMMIT.txt` holds the branch head. `uf2conv.py` SHA256 `ad36ba2d…c20f26`, `uf2families.json` SHA256 `9d8f561e…64c194` (already downloaded to `firmware/tools/` on 2026-10-28).

- [ ] **Step 4: Commit in both repos**

```bash
# rmk clone
git add keyboards/op36/Cargo.toml && git commit -m "feat(op36): enable layer_notify" && git push -u origin op36-layer-notify
# omega36
git add firmware/tools firmware/op36-layer-notify && git commit -m "firmware: build script and layer-notify UF2 for OP36 left half"
```

---

### Task 6: Flash the left half and prove the protocol end to end

**Files:**
- Create: `omega36/firmware/tools/layer_notify_probe.py` (Python 3.12, `hidapi` package in a venv at `omega36/.venv`, git-ignored)
- Create: `omega36/firmware/FLASHING.md` (the vendor procedure as done on 2026-10-28, plus rollback)

**Interfaces:**
- Produces: `layer_notify_probe.py` — opens the HID device with VID `0xE126`, PID `0x0036`, usage page `0xFF60`, usage `0x61`; sends `[0x00, 0x07, 0xE9, 0x01, 30] + zeros` (hidapi on Windows needs the leading report id byte); prints the reply; then loops for 15 s printing every packet whose first byte is `0xE9` as `active=<n> default=<n> mask=<bin> fw_layout=<n>`.

- [ ] **Step 1: Write the probe and a unit test for its parser**

```python
def parse_state(pkt: bytes) -> dict:
    assert pkt[0] == 0xE9 and pkt[1] == 0x01
    return {"active": pkt[2], "default": pkt[3], "mask": pkt[4] | (pkt[5] << 8), "fw_layout": pkt[6]}

def test_parse_state():
    assert parse_state(bytes([0xE9, 1, 3, 0, 0x0E, 0, 1] + [0] * 25)) == {"active": 3, "default": 0, "mask": 0b1110, "fw_layout": 1}
```

Run: `python -I -m pytest firmware/tools/layer_notify_probe.py -q` (pytest in the venv). Expected: `1 passed`.

- [ ] **Step 2: Flash the left half** (user action, same as 2026-10-28)

Ask Boris to plug the cable into the left half and double-tap Reset. When drive `D:` with label `NRF52BOOT` appears:
```bash
cp firmware/op36-layer-notify/op36_left.uf2 /d/
```
Expected: drive disappears within ~5 s; `Get-PnpDevice` shows `VID_E126&PID_0036` again.

- [ ] **Step 3: Run the probe while Boris holds the red thumb key, then both thumb keys**

Run: `.venv/Scripts/python -I firmware/tools/layer_notify_probe.py`
Expected output, in order:
```
reply: active=0 default=0 mask=0b0 fw_layout=0
notify: active=1 default=0 mask=0b10 fw_layout=0     # red held
notify: active=0 default=0 mask=0b0 fw_layout=0      # released
notify: active=3 default=0 mask=0b1110 fw_layout=0   # both held
```
(`mask` lists explicitly activated layers; the default layer is not in it.)

- [ ] **Step 4: Confirm silence after expiry**

Stop the probe, wait 35 s, run a second instance that only reads (no subscribe) for 10 s while pressing a thumb key. Expected: no `0xE9` packets. Then confirm Entropy, if installed, still connects and reads the layout.

- [ ] **Step 5: Write `FLASHING.md` and commit**

```bash
git add firmware/FLASHING.md firmware/tools/layer_notify_probe.py
git commit -m "firmware: flashing notes and layer-notify probe"
```

---

## Self-review notes

- Spec §5 packet formats, opt-in TTL, transport-of-origin, non-blocking send, feature gate, branch point, left-half-only flash, rollback: Tasks 2–6. Spec §9 split-latency item: untouched by design.
- The reviewer's "`layer_change.subs` must be raised" concern does not apply: this plan calls `layer_notify::notify` directly from the keymap instead of adding an event subscriber. If the implementer chooses a processor instead, they must bump `[event] layer_change.subs = 2` in `keyboards/op36/keyboard.toml`.
- Names used across tasks: `LayerState`, `encode_state`, `subscribe`, `active_subscriber`, `notify`, `now_ms`, `NAMESPACE`, `SUB_STATE`, `MAX_TTL_SECS`, `last_transport`, `layer_mask`, `host_data::layout` — consistent in Tasks 2–4.
