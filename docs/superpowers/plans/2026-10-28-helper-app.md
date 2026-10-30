# OP36 Layout Helper (Windows tray app) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust tray application that draws a click-through overlay of the OP36 keymap in the vendor's documentation style, following the live layer from the keyboard, the Windows language of the foreground window, held modifiers, and showing how to type any key pressed on any keyboard.

**Architecture:** One UI thread owns every window (hidden message window for tray/hotkey/raw input, the layered overlay, the Options dialog) and a pure `Model` that turns keymap + layer + modifiers + language + find-key state into a `Frame`. One HID thread talks Vial to the keyboard (keymap read, layer subscription with 20 s renew) and posts results to the UI thread with `PostMessage`. All decision logic (protocol codec, legend rules, find-key search, geometry, settings) is pure and unit-tested; Win32 and Direct2D code is thin and verified manually against the mockup.

**Tech Stack:** Rust stable 1.99, crates `windows` (Win32 UI, Raw Input, Direct2D, DirectWrite, Shell), `hidapi`, `rmk-types` + `postcard` (decode the keyboard's native key actions losslessly), `serde`/`serde_json` (settings), `log` + `simplelog` (file log). No installer; single exe.

**Spec:** `docs/superpowers/specs/2026-10-28-op36-layout-helper-design.md` (§6 helper, §6.2 visual, §6.3 legend rules, §6.6 find a key, §7 how it gets made). Firmware protocol: `docs/superpowers/plans/2026-10-28-firmware-layer-notify.md` Global Constraints.

## Global Constraints

- Crate at `C:\Users\boris_kudriashov\Source\omega36\helper`, package and binary name `op36-helper`, edition 2024, `#![windows_subsystem = "windows"]` in release.
- Depends on the firmware plan's Task 1 (the `rmk` clone at `omega36/rmk`) for the `rmk-types` path dependency, and on its Tasks 2–6 (flashed left half) for live layer data from Task 5 onward. Everything else in this plan works against stock v0.2.0.
- No registry writes, no admin, no network, no code signing. Autostart = `.lnk` in `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`.
- Settings file `%APPDATA%\op36-helper\settings.json`, cached keymap `%APPDATA%\op36-helper\keymap.bin` (postcard), log `%APPDATA%\op36-helper\log.txt`.
- Keyboard: HID VID `0xE126`, PID `0x0036`, usage page `0xFF60`, usage `0x61`; 32-byte packets; write with a leading `0x00` report-id byte on Windows (hidapi), reads return 32 data bytes (confirm against `_ref/entropy/src/hid.rs` when implementing).
- Layer protocol bytes exactly as the firmware plan: subscribe `07 E9 01 ttl`, get `08 E9 01`, notification `E9 01 active default lo hi fw_layout`. Renew every 20 s with TTL 60.
- Overlay window style: `WS_POPUP`, ex-style `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`; per-monitor DPI awareness v2 via manifest.
- Visual constants (spec §6.2): key 64, pitch 72, radius 9, column stagger `[40,10,0,10,20]` (pinky→inner), thumb rotation 15°, thumb centres (left half, unscaled) `(236,300) (308,312) (380,330)`, half canvas 470×400, panel `rgba(28,30,36,0.80)` radius 14, case `#ededed`, home-row `#e2e2e2`, layer colours `0 #ffffff`, `1 #d9414c`, `2 #1b8da3`, `3 #f3e39a`, pale `#f3aeb3 / #b9dfe8 / #faf2cf`, RU legend `#d63a3a`, find halo `#ffd34d`.
- Defaults: primary monitor, anchor bottom-centre, offset (0, 8 px above work area), scale 0.6, opacity 80 %, hotkey Ctrl+Alt+K, find-key on with 2.0 s, display mode "always".
- Physical layout, position index 0–35 in reading order (spec screenshots): rows 0–2 = left `(r,4) (r,3) (r,2) (r,1) (r,0)` then right `(r+3,0) … (r+3,4)`; thumbs 30–35 = `(0,5) (1,5) (2,5) (5,5) (4,5) (3,5)` (left outer→inner, then right inner→outer). Matrix 6 rows × 6 cols, 16 layers, flat index `layer*36 + row*6 + col`.

## Review Focus

- A keyboard layout other than EN/RU in the foreground (e.g. German): legends must still come from `ToUnicodeEx` for that layout, with the EN hint, never crash on unknown HKL (Task 6 test `legend_uses_foreground_layout_for_any_hkl`).
- The keyboard unplugged while the overlay is up: tray turns grey, overlay shows the cached keymap with the "not connected" marker, reconnect succeeds without restart (Task 11 manual check and test `model_marks_disconnected`).
- Selected monitor absent at startup: fall back to primary without overwriting the saved monitor name (Task 8 test `placement_falls_back_to_primary`).
- A key pressed that exists only with an extra Shift (e.g. `:` in EN on base layer): find-key must rank the Symbols-layer `:` first and the `⇧ + ;` recipe second (Task 9 test `find_ranks_exact_before_shifted`).
- Dead-key layouts: `ToUnicodeEx` must not consume or alter the user's pending dead key (Task 6: call with `wFlags = 1 << 2` on Windows 10+, test `char_source_passes_no_change_flag`).

---

### Task 1: Crate scaffold, settings, logging, single instance

**Files:**
- Create: `helper/Cargo.toml`, `helper/build.rs` (embed manifest via `embed-manifest` or `winres`), `helper/app.manifest` (DPI awareness PerMonitorV2, no UAC elevation)
- Create: `helper/src/main.rs`, `helper/src/settings.rs`, `helper/src/paths.rs`
- Test: `helper/src/settings.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `paths::app_dir() -> PathBuf` (`%APPDATA%\op36-helper`, created on demand), `paths::settings_file()`, `paths::keymap_cache_file()`, `paths::log_file()`, `paths::startup_shortcut()`.
  - `#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)] struct Settings { visible: bool, monitor: Option<String>, anchor: Anchor, offset: (i32, i32), scale: f32, opacity: f32, hotkey: Hotkey, autostart: bool, display_mode: DisplayMode, find_key: bool, find_key_secs: f32, show_en_hint: bool }` with `impl Default` holding the Global Constraints defaults.
  - `enum Anchor { TopLeft, Top, TopRight, Left, Center, Right, BottomLeft, Bottom, BottomRight }`, `enum DisplayMode { Always, WhileHeld }`, `struct Hotkey { modifiers: u32 /* MOD_CONTROL|MOD_ALT|… */, vk: u32 }`.
  - `Settings::load(path) -> Settings` (defaults on missing or invalid file, logs a warning), `Settings::save(&self, path) -> io::Result<()>` (write to `*.tmp` then rename).
  - `main`: named mutex `Local\op36-helper-single-instance`; if it already exists, `PostMessage` `WM_APP_TOGGLE` to the existing hidden window (found by class name `Op36HelperMsg`) and exit.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn defaults_match_spec() {
    let s = Settings::default();
    assert_eq!((s.scale, s.opacity, s.find_key_secs), (0.6, 0.8, 2.0));
    assert_eq!(s.anchor, Anchor::Bottom);
    assert_eq!(s.offset, (0, 8));
    assert_eq!(s.hotkey, Hotkey { modifiers: MOD_CONTROL.0 | MOD_ALT.0, vk: 0x4B });
    assert!(s.visible && s.find_key && s.show_en_hint && !s.autostart);
}

#[test]
fn save_then_load_round_trips() { /* tempdir; s.scale = 0.9; save; assert_eq!(Settings::load(&p), s) */ }

#[test]
fn corrupt_file_yields_defaults() { /* write "{not json"; assert_eq!(Settings::load(&p), Settings::default()) */ }
```

- [ ] **Step 2: Run tests to verify they fail** — `cd helper && cargo test settings` → compile error (no crate yet / no items).
- [ ] **Step 3: Create the crate and implement the Interfaces block** — `cargo init --name op36-helper`; dependencies: `windows` (features `Win32_Foundation, Win32_UI_WindowsAndMessaging, Win32_UI_Input_KeyboardAndMouse, Win32_System_Threading`), `serde`, `serde_json`, `log`, `simplelog`, `dirs`; dev-dep `tempfile`.
- [ ] **Step 4: Run tests to verify they pass** — `cargo test settings` → `3 passed`.
- [ ] **Step 5: Commit** — `git add helper && git commit -m "feat(helper): crate scaffold, settings, logging, single instance"`

---

### Task 2: Hidden message window, tray icon, hotkey, empty overlay window

**Files:**
- Create: `helper/src/win/mod.rs` (helpers: `to_wide`, `check(HRESULT)`), `helper/src/win/msgwindow.rs`, `helper/src/tray.rs`, `helper/src/overlay/window.rs`
- Create: `helper/assets/icon_on.ico`, `helper/assets/icon_off.ico` (coloured / grey keyboard glyph, 16/32/48 px; generate with a small Python script committed under `helper/assets/make_icons.py`)
- Modify: `helper/src/main.rs`

**Interfaces:**
- Produces:
  - `const WM_APP_TOGGLE: u32 = WM_APP + 1; WM_APP_TRAY = WM_APP + 2; WM_APP_HID = WM_APP + 3; WM_APP_RAWKEY = WM_APP + 4;` in `win/msgwindow.rs`.
  - `MsgWindow::create(handler: Box<dyn FnMut(u32, WPARAM, LPARAM) -> Option<LRESULT>>) -> MsgWindow` (class `Op36HelperMsg`), `fn hwnd(&self) -> HWND`.
  - `Tray::new(hwnd, connected: bool) -> Tray`, `fn set_connected(&mut self, bool)`, `fn set_tooltip(&mut self, &str)`, `fn show_menu(&self, hwnd)` with items `Show overlay`/`Hide overlay` (checkmark state), `Options…`, `Exit` returning `enum TrayCmd { Toggle, Options, Exit }`. Left click = `TrayCmd::Toggle`, double-click = `Options`.
  - `Hotkey::register(hwnd, &settings.hotkey) -> Result<(), String>` (id 1; error text includes the conflicting combination).
  - `OverlayWindow::create() -> OverlayWindow` with the ex-styles from Global Constraints; `fn show(&self, bool)`, `fn set_bounds(&self, x, y, w, h)`, `fn present(&self, frame: &BgraBitmap)` calling `UpdateLayeredWindow` with `ULW_ALPHA` and `BLENDFUNCTION { SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA }`; `fn reassert_topmost(&self)`.
  - `BgraBitmap { width, height, premultiplied: Vec<u32> }` in `overlay/mod.rs` (temporary CPU surface, Task 8 replaces with Direct2D).

- [ ] **Step 1: Write the test for the one pure piece** — `tray::menu_command_from_id(id) -> Option<TrayCmd>` round trip for ids 1001–1003, `None` for others.
- [ ] **Step 2: Run** `cargo test tray` → compile error.
- [ ] **Step 3: Implement** the Interfaces block. Temporary overlay content: a 600×300 panel filled with `rgba(28,30,36,0.80)` and rounded corners drawn with a simple software fill, positioned bottom-centre of the primary monitor's work area (`SystemParametersInfoW(SPI_GETWORKAREA)`).
- [ ] **Step 4: Run** `cargo test` → pass; then `cargo run` and check manually: tray icon appears; left click hides/shows the panel; Ctrl+Alt+K toggles; clicking through the panel reaches the window beneath; the panel does not appear in Alt-Tab; `Exit` removes the icon; a second `cargo run` toggles the first instance and exits.
- [ ] **Step 5: Commit** — `git commit -am "feat(helper): tray icon, hotkey, click-through layered overlay window"`

---

### Task 3: Keymap model and OP36 physical layout

**Files:**
- Create: `helper/src/keymap/mod.rs`, `helper/src/keymap/layout.rs`
- Modify: `helper/Cargo.toml` (add `rmk-types = { path = "../rmk/rmk-types" }`, `postcard = "1"`)
- Test: in-module

**Interfaces:**
- Produces:
  - `pub use rmk_types::action::{KeyAction, Action}; pub use rmk_types::keycode::{KeyCode, HidKeyCode}; pub use rmk_types::modifier::ModifierCombination;`
  - `struct Keymap { rows: u8, cols: u8, layers: u8, actions: Vec<KeyAction>, layer_names: Vec<String> }` with `fn flat(layer, row, col) -> usize`, `fn action(&self, layer: u8, pos: &PhysicalKey) -> KeyAction`, `fn resolve(&self, active_mask: u16, default_layer: u8, pos) -> (u8 /*layer it came from*/, KeyAction)` (Transparent falls through highest active → lower → default; `No` stays `No`), `fn to_bytes(&self) -> Vec<u8>` / `fn from_bytes(&[u8]) -> Option<Keymap>` (postcard; `Serialize`/`Deserialize` derive on `Keymap`).
  - `struct PhysicalKey { index: u8, row: u8, col: u8, hand: Hand, column: Option<u8> /*0 pinky..4 inner*/, thumb: Option<u8> /*0 outer..2 inner*/, home_row: bool }`, `enum Hand { Left, Right }`, `const OP36_KEYS: [PhysicalKey; 36]` per Global Constraints.
  - `fn key_at(row, col) -> Option<&'static PhysicalKey>`.
  - Note: the `rmk-types` build script reads `KEYBOARD_TOML_PATH` if set; the helper must not set it (defaults are fine).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn op36_has_36_unique_matrix_positions() { /* set of (row,col) has len 36; rows < 6; cols < 6 */ }
#[test] fn reading_order_matches_spec() {
    assert_eq!((OP36_KEYS[0].row, OP36_KEYS[0].col), (0, 4));   // Q
    assert_eq!((OP36_KEYS[9].row, OP36_KEYS[9].col), (3, 4));   // P
    assert_eq!((OP36_KEYS[30].row, OP36_KEYS[30].col), (0, 5)); // Bksp
    assert_eq!((OP36_KEYS[33].row, OP36_KEYS[33].col), (5, 5)); // Enter (right inner)
    assert_eq!((OP36_KEYS[35].row, OP36_KEYS[35].col), (3, 5)); // Tab
}
#[test] fn home_row_flags() { /* indices 10,11,12,13,16,17,18,19 home_row; others not */ }
#[test] fn transparent_falls_through_to_lower_active_layer() {
    let mut km = Keymap::empty(6, 6, 4);
    km.set(0, 0, 4, KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Q))));
    km.set(1, 0, 4, KeyAction::Transparent);
    let (from, a) = km.resolve(0b0011, 0, key_at(0, 4).unwrap());
    assert_eq!(from, 0);
    assert_eq!(a, KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Q))));
}
#[test] fn postcard_round_trip() { /* km.to_bytes() → from_bytes() == km */ }
```

- [ ] **Step 2: Run** `cargo test keymap` → compile error.
- [ ] **Step 3: Implement** the Interfaces block (`Keymap::empty(rows, cols, layers)` and `set(layer,row,col,action)` are test/plumbing helpers, keep them `pub(crate)`).
- [ ] **Step 4: Run** `cargo test keymap` → `5 passed`.
- [ ] **Step 5: Commit** — `git commit -am "feat(helper): keymap model, OP36 physical layout, postcard cache format"`

---

### Task 4: Vial protocol codec, HID transport, keymap reader, `--dump-keymap`

**Files:**
- Create: `helper/src/vial/mod.rs`, `helper/src/vial/protocol.rs`, `helper/src/vial/transport.rs`, `helper/src/vial/reader.rs`
- Modify: `helper/src/main.rs` (CLI flags `--dump-keymap <file>`, `--probe-layers`), `helper/Cargo.toml` (`hidapi = "2"`)
- Create: `helper/assets/factory_keymap.bin` (produced in Step 4 from the real keyboard, committed)

**Interfaces:**
- Produces (`protocol.rs`, pure):
  - `type Packet = [u8; 32];`
  - `fn req_protocol_version() -> Packet` (`01`), `fn req_layer_count() -> Packet` (`11`), `fn req_keymap_buffer(offset: u16, size: u8) -> Packet` (`12 off_hi off_lo size`, size ≤ 28), `fn parse_keymap_buffer(reply: &Packet, size: u8) -> Vec<u16>` (big-endian u16s from byte 4), `fn req_native_action(layer, row, col) -> Packet` (`08 E8 03 01 layer row col`), `fn parse_native_action(reply: &Packet) -> Result<KeyAction, NativeStatus>` (status at byte 4, len at byte 5, postcard payload from byte 6), `fn req_layer_subscribe(ttl: u8) -> Packet` (`07 E9 01 ttl`), `fn req_layer_state() -> Packet` (`08 E9 01`), `fn parse_layer_state(pkt: &Packet) -> Option<LayerState>` (`None` unless bytes 0..2 == `E9 01`), `struct LayerState { active: u8, default: u8, mask: u16, fw_layout: u8 }`, `fn via_keycode_to_action(kc: u16) -> Option<KeyAction>` for the basic ranges only: `0x0000` → `No`, `0x0001` → `Transparent`, `0x0004..=0x00FF` → `Single(Key(Hid))`, `0x0100..=0x1FFF` → `Single(KeyWithModifier(kc & 0xFF, mods from bits 8–12 per QMK: bit8 ctrl, 9 shift, 10 alt, 11 gui, 12 right))`, `0x2000..=0x3FFF` → `TapHold(Key, Modifier)` (QMK MT), `0x4000..=0x4FFF` → `TapHold(Key, LayerOn((kc>>8)&0xF))` (LT), `0x5220|n` → `LayerOn(n)` (RMK MO), `0x5260|n` → `LayerToggle(n)`, `0x5240|n` → `DefaultLayer(n)`, anything else → `None` (then the native read decides).
- Produces (`transport.rs`): `trait VialTransport { fn write(&mut self, &Packet) -> Result<()>; fn read(&mut self, timeout_ms: i32) -> Result<Option<Packet>>; }`, `struct HidTransport` implementing it over `hidapi::HidDevice` (opened with `open_path` of the device matching VID/PID/usage page/usage; `set_blocking_mode(false)` + `read_timeout`), `fn find_device(api: &HidApi) -> Option<DeviceInfo>`, `struct FakeTransport { script: VecDeque<Packet>, sent: Vec<Packet> }` under `cfg(test)`.
- Produces (`reader.rs`): `fn read_keymap(t: &mut dyn VialTransport, rows: u8, cols: u8) -> Result<Keymap>`: layer count via `11`; read the whole VIA buffer in 28-byte chunks (`layers*rows*cols*2` bytes); map with `via_keycode_to_action`; for every flat index where the mapping is `None` **or** the VIA keycode is `0` but the position is inside the matrix, issue `req_native_action` and use its result (`NativeStatus::InvalidPosition` → `No`). Request/response matching: after each write, read until a packet whose byte 0 equals the request's byte 0 (and bytes 1–2 for `08 E8`), discarding `E9` notifications, timeout 500 ms → `Err`. Read every layer the keyboard reports (16 × 36 × 2 bytes = 42 buffer packets, well under a second); do not assume layers ≥ 5 are empty.
- `main --dump-keymap <file>`: reads and writes `Keymap::to_bytes()`; prints a text table (layer × 36 positions, `Debug` of each action) to stdout. `--probe-layers`: subscribes and prints `LayerState` lines until Ctrl+C (same as the firmware plan's Python probe, used from here on).

- [ ] **Step 1: Write the failing tests** (`protocol.rs`, `reader.rs` with `FakeTransport`)

```rust
#[test] fn keymap_buffer_request_is_big_endian() { assert_eq!(&req_keymap_buffer(0x0102, 28)[..4], &[0x12, 0x01, 0x02, 28]); }
#[test] fn parse_layer_state_rejects_other_packets() { assert!(parse_layer_state(&[0x08; 32]).is_none()); }
#[test] fn parse_layer_state_reads_fields() {
    let mut p = [0u8; 32]; p[..7].copy_from_slice(&[0xE9, 1, 3, 0, 0x0E, 0, 1]);
    assert_eq!(parse_layer_state(&p), Some(LayerState { active: 3, default: 0, mask: 0x0E, fw_layout: 1 }));
}
#[test] fn via_mod_tap_decodes_to_tap_hold() {
    // QMK MT(MOD_LGUI, KC_A) = 0x2000 | (0x08 << 8) | 0x04
    let a = via_keycode_to_action(0x2804).unwrap();
    assert!(matches!(a, KeyAction::TapHold(Action::Key(KeyCode::Hid(HidKeyCode::A)), Action::Modifier(m), _) if m.left_gui()));
}
#[test] fn via_momentary_layer_decodes() { assert_eq!(via_keycode_to_action(0x5221), Some(KeyAction::Single(Action::LayerOn(1)))); }
#[test] fn parse_native_action_decodes_postcard_payload() {
    let action = KeyAction::Single(Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::new_from(false, false, false, true, false)));
    let mut reply = [0u8; 32]; reply[..4].copy_from_slice(&[0x08, 0xE8, 0x03, 0x01]); reply[4] = 0;
    let n = postcard::to_slice(&action, &mut reply[6..]).unwrap().len(); reply[5] = n as u8;
    assert_eq!(parse_native_action(&reply), Ok(action));
}
#[test] fn reader_uses_native_read_for_unmappable_keycodes() {
    // FakeTransport scripted: layer count 2; buffer replies where flat index 7 is 0x7C77 (unknown); then a native reply for (0,1,1)
    // assert the sent packets contain `08 E8 03 01 00 01 01` and the resulting action equals the native one.
}
#[test] fn reader_skips_layer_notifications_while_waiting() { /* script an E9 packet before the real reply; read_keymap still succeeds */ }
```

(`ModifierCombination::new_from(right, gui, alt, shift, ctrl)` — check the exact constructor name in `_ref/rmk/rmk-types/src/modifier.rs` and use that.)

- [ ] **Step 2: Run** `cargo test vial` → compile error.
- [ ] **Step 3: Implement** the Interfaces block.
- [ ] **Step 4: Run** `cargo test vial` → all pass. Then with the keyboard on USB: `cargo run -- --dump-keymap helper/assets/factory_keymap.bin`. Expected stdout: layer 0 position 0 = `Single(Key(Hid(Q)))`, position 10 = `TapHold(Key(Hid(A)), Modifier(LGui), …)`, position 31 = `Single(LayerOn(1))`; layer 2 position 11 = `TapHold(KeyWithModifier(Kc9, LShift), Modifier(LAlt), …)`; 5 non-empty layers. Commit the `.bin`.
- [ ] **Step 5: Commit** — `git add helper && git commit -m "feat(helper): Vial codec, HID transport, keymap reader, factory keymap dump"`

---

### Task 5: Layer client and the UI-side model skeleton

**Files:**
- Create: `helper/src/vial/layer_client.rs`, `helper/src/hid_thread.rs`, `helper/src/model.rs`
- Modify: `helper/src/main.rs`

**Interfaces:**
- Produces (`layer_client.rs`, pure state machine): `struct LayerClient { last_renew_ms: u64, supported: Option<bool> }`, `fn on_connect(&mut self) -> Vec<Packet>` (returns `[subscribe(60), get_state]`), `fn tick(&mut self, now_ms) -> Option<Packet>` (subscribe(60) when `now_ms - last_renew_ms >= 20_000`), `fn on_packet(&mut self, p: &Packet) -> Option<LayerState>` (sets `supported = Some(true)` on the first `E9` packet; `supported = Some(false)` if 1 s passes after `on_connect` with no `E9` reply — exposed as `fn on_timeout(&mut self, now_ms)`).
- Produces (`hid_thread.rs`): `fn spawn(ui_hwnd: HWND) -> Sender<HidCmd>`; `enum HidCmd { ReloadKeymap, Quit }`; thread loop: find device every 2 s when disconnected; on connect `read_keymap` → `HidEvent::Keymap(Keymap)`, then `LayerClient::on_connect`; read loop with 50 ms timeout feeding `on_packet`/`tick`; posts `WM_APP_HID` with a boxed `HidEvent` pointer in `LPARAM` (`enum HidEvent { Connected, Disconnected, Keymap(Keymap), Layer(LayerState), LayerNotifySupported(bool), Error(String) }`); on any HID error → `Disconnected` and back to the 2 s search.
- Produces (`model.rs`, pure): `struct Model { keymap: Keymap, keymap_source: KeymapSource /*Live|Cached|Bundled*/, connected: bool, layer_notify: Option<bool>, layer: LayerState, mods: Modifiers, hkl: Hkl, find: Option<FindResult>, settings: Settings }` with `fn apply(&mut self, ev: ModelEvent)`; `enum ModelEvent { Hid(HidEvent), Mods(Modifiers), Language(Hkl), Key(KeyPress), FindExpired, Settings(Settings) }`. `Modifiers`, `Hkl`, `KeyPress`, `FindResult` defined here as plain data (filled by Tasks 6, 7, 9). On startup `Model::new(settings)` loads `keymap.bin` from the cache, else `assets/factory_keymap.bin` via `include_bytes!`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn connect_sends_subscribe_then_get_state() { let mut c = LayerClient::new(); let v = c.on_connect(); assert_eq!(&v[0][..4], &[7,0xE9,1,60]); assert_eq!(&v[1][..3], &[8,0xE9,1]); }
#[test] fn renews_every_20_seconds() { let mut c = LayerClient::new(); c.on_connect_at(0); assert!(c.tick(19_999).is_none()); assert!(c.tick(20_000).is_some()); assert!(c.tick(20_001).is_none()); }
#[test] fn unsupported_after_one_second_silence() { let mut c = LayerClient::new(); c.on_connect_at(0); c.on_timeout(1_000); assert_eq!(c.supported, Some(false)); }
#[test] fn model_marks_disconnected() { /* apply Hid(Connected) then Hid(Disconnected): connected == false, layer.active == 0, keymap unchanged */ }
#[test] fn model_prefers_live_keymap_and_caches_it() { /* apply Hid(Keymap(k)) → keymap_source == Live, and `pending_cache_write == Some(k.to_bytes())` */ }
```

- [ ] **Step 2: Run** `cargo test layer_client model` → compile error.
- [ ] **Step 3: Implement** the Interfaces block; wire `hid_thread::spawn` in `main`, handle `WM_APP_HID` by `Model::apply` and set the tray icon state from `model.connected`.
- [ ] **Step 4: Run** tests → pass. Manual: `cargo run -- --probe-layers` prints the state line on start and a line per thumb-key press (requires the firmware plan's flashed left half; with stock firmware it prints `layer notify: not supported`).
- [ ] **Step 5: Commit** — `git commit -am "feat(helper): layer subscription client, HID thread, model skeleton"`

---

### Task 6: HID table, character source, legend rules

**Files:**
- Create: `helper/src/legend/hid_table.rs`, `helper/src/legend/chars.rs`, `helper/src/legend/mod.rs`, `helper/src/legend/labels.rs`

**Interfaces:**
- Produces:
  - `hid_table`: `fn hid_to_scancode(usage: u8) -> Option<ScanCode>` and `fn scancode_to_hid(sc: ScanCode) -> Option<u8>`, `struct ScanCode { code: u8, extended: bool }`, for HID usages `0x04..=0x65` and `0xE0..=0xE7` (modifiers) using the standard USB-HID-to-PS/2-set-1 table (same table QMK/ZMK docs reproduce; arrows, nav cluster and right modifiers are `extended`).
  - `chars`: `trait CharSource { fn char_for(&self, hkl: Hkl, sc: ScanCode, shift: bool) -> Option<String>; fn en_hkl(&self) -> Hkl; }`, `struct WinCharSource` (MapVirtualKeyExW `MAPVK_VSC_TO_VK_EX`, `ToUnicodeExW` with key state array and `wFlags = 1 << 2`, returns `None` for control chars/dead keys/no translation; `en_hkl` = first installed layout whose primary language id is `0x09`, else `LoadKeyboardLayoutW("00000409", KLF_NOTELLSHELL)`), `struct FakeCharSource { map: HashMap<(Hkl, u8, bool), &'static str> }` under `cfg(test)` pre-filled for EN-US and RU on the 36 keys.
  - `labels`: `fn label_for_hid(usage: HidKeyCode) -> Option<&'static str>` (`Bksp, Enter, Tab, Esc, Space, Del, Ins, Home, End, Page Up, Page Down, Prt Sc, Caps, ← ↑ ↓ →, F1…F24`), `fn label_for_action(a: &Action) -> Option<String>` (`Vol+ Vol− Mute ⏮ ⏯ ⏭`, `Flash`, `Out BLE`, `Out USB`, `Caps Word`, `BT n` for `User(0..=4)`, `BT CLR` for `User(7)`, `Sym n` for other `User(n)`, `Macro n`, `TD n` for `Morse(n)`).
  - `legend`: `struct Legend { primary: String, secondary: Option<String>, mod_icon: Option<&'static str>, layer_icon: Option<u8>, kind: KeyKind }`, `enum KeyKind { Normal, Home, Empty, HeldLayer, LayerKey(u8) }`, `struct LegendCtx { hkl: Hkl, mods: Modifiers, show_en_hint: bool, active_layer: u8 }`, `fn legend_for(action: &KeyAction, from_layer: u8, key: &PhysicalKey, ctx: &LegendCtx, chars: &dyn CharSource) -> Legend` implementing spec §6.3 rules 1–8, plus `fn mod_icon(m: ModifierCombination) -> &'static str` (`⊞`, `⇧`, `^`, `⎇`; several → joined without spaces).

- [ ] **Step 1: Write the failing tests** (all with `FakeCharSource`)

```rust
#[test] fn base_letter_in_ru_shows_ru_primary_and_en_hint() { /* Q key, hkl=RU → primary "й"→"Й" uppercase display, secondary Some("Q") */ }
#[test] fn base_letter_in_en_has_no_secondary() { /* Q, hkl=EN → ("Q", None) */ }
#[test] fn semicolon_in_en_shows_shift_variant() { /* ; → primary ";", secondary Some(":") */ }
#[test] fn shifted_symbol_layer_key_in_ru() { /* KeyWithModifier(Kc3, LShift), hkl=RU → primary "№", secondary Some("#") */ }
#[test] fn shift_held_swaps_primary() { /* ; with mods.shift → primary ":", secondary None */ }
#[test] fn ctrl_held_shows_en_letters() { /* Q, hkl=RU, mods.ctrl → primary "Q", secondary None */ }
#[test] fn mod_tap_shows_hold_icon() { /* TapHold(Key(A), Modifier(LGui)) → primary from A, mod_icon Some("⊞") */ }
#[test] fn layer_key_marks_kind() { /* Single(LayerOn(1)) → kind LayerKey(1); when ctx.active_layer == 1 → HeldLayer */ }
#[test] fn transparent_and_no_render_empty() { /* No → kind Empty, primary "" */ }
#[test] fn legend_uses_foreground_layout_for_any_hkl() { /* FakeCharSource with a DE entry for Y → primary "Z", secondary Some("Y") */ }
#[test] fn char_source_passes_no_change_flag() { /* WinCharSource unit: the flags argument it builds == 1 << 2 (expose `fn to_unicode_flags() -> u32`) */ }
#[test] fn hid_table_round_trips_letters_and_arrows() { /* 0x04 (A) ↔ sc 0x1E; 0x50 (Left) ↔ sc 0x4B extended */ }
```

- [ ] **Step 2: Run** `cargo test legend` → compile error.
- [ ] **Step 3: Implement** the Interfaces block. Display case for letters: show uppercase in both languages (as in the vendor screenshots); the "shifted differs by more than case" test is `to_uppercase(base) != shifted`.
- [ ] **Step 4: Run** `cargo test legend` → `12 passed`.
- [ ] **Step 5: Commit** — `git commit -am "feat(helper): HID table, Windows character source, legend rules"`

---

### Task 7: Raw Input (modifiers, key presses), foreground language

**Files:**
- Create: `helper/src/input/raw_input.rs`, `helper/src/input/modifiers.rs`, `helper/src/input/language.rs`
- Modify: `helper/src/win/msgwindow.rs` (handle `WM_INPUT`, `WM_APP_RAWKEY`), `helper/src/main.rs`

**Interfaces:**
- Produces:
  - `raw_input::register(hwnd) -> Result<()>` (`RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RIDEV_INPUTSINK, hwndTarget }`), `fn parse(lparam) -> Option<RawKey>` (`GetRawInputData`), `struct RawKey { scancode: ScanCode, vk: u16, down: bool, device: DeviceKind }`, `enum DeviceKind { Op36, Other }` (`GetRawInputDeviceInfoW(RIDI_DEVICENAME)` contains `VID_E126&PID_0036`, cached per `HANDLE`).
  - `modifiers`: `#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)] struct Modifiers { shift: bool, ctrl: bool, alt: bool, win: bool }`, `struct ModTracker` with `fn apply(&mut self, k: &RawKey) -> Modifiers` (left/right VKs folded) and `fn resync(&mut self) -> Modifiers` (from `GetAsyncKeyState`, called on foreground change).
  - `language`: `type Hkl = isize;` `fn foreground_hkl() -> Hkl` (`GetForegroundWindow` → `GetWindowThreadProcessId` → `GetKeyboardLayout`; for class `ConsoleWindowClass`/`ApplicationFrameWindow` try `GetGUIThreadInfo(tid).hwndFocus` first; `0` → keep previous), `fn primary_lang_tag(hkl) -> String` (`"EN"`, `"RU"`, else the 4-hex language id), `struct LanguageWatcher` installing `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_FOCUS)` plus a 250 ms `SetTimer`, posting `WM_APP_LANG` when the HKL changes.
- `ModelEvent::Key(KeyPress { hid: Option<u8>, scancode, mods_at_press: Modifiers, device: DeviceKind })` posted for every non-modifier key-down.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn tracker_folds_left_and_right_shift() { /* down LSHIFT → shift; down RSHIFT; up LSHIFT → still shift; up RSHIFT → none */ }
#[test] fn tracker_ignores_non_modifiers() { /* A down → Modifiers::default() */ }
#[test] fn primary_lang_tag_maps_known_ids() { assert_eq!(primary_lang_tag(0x04090409), "EN"); assert_eq!(primary_lang_tag(0x04190419), "RU"); assert_eq!(primary_lang_tag(0x04070407), "0407"); }
#[test] fn device_kind_from_name() { assert_eq!(DeviceKind::from_name(r"\\?\HID#VID_E126&PID_0036&MI_00#..."), DeviceKind::Op36); assert_eq!(DeviceKind::from_name(r"\\?\HID#VID_046D..."), DeviceKind::Other); }
```

- [ ] **Step 2: Run** `cargo test input` → compile error.
- [ ] **Step 3: Implement** the Interfaces block; wire into `main`: `WM_INPUT` → `parse` → `ModTracker::apply` → `ModelEvent::Mods` (only when changed) and `ModelEvent::Key` for key-downs; `WM_APP_LANG` → `ModelEvent::Language`.
- [ ] **Step 4: Run** tests → pass. Manual with the temporary overlay: log lines show `mods=…` changing when holding Shift on the laptop and when holding the OP36 home-row D; `lang=RU/EN` follows Win+Space; `device=Op36` for OP36 keys.
- [ ] **Step 5: Commit** — `git commit -am "feat(helper): raw input modifiers and key presses, foreground language watcher"`

---

### Task 8: Geometry, Direct2D rendering, placement on monitors

**Files:**
- Create: `helper/src/overlay/geometry.rs`, `helper/src/overlay/frame.rs`, `helper/src/overlay/render_d2d.rs`, `helper/src/overlay/placement.rs`
- Modify: `helper/src/overlay/window.rs` (present from a Direct2D GDI-compatible bitmap render target, or `ID2D1DeviceContext` on a `IDXGISurface` copied to a DIB section — builder's call, result is premultiplied BGRA for `UpdateLayeredWindow`), `helper/src/model.rs` (`fn frame(&self, chars: &dyn CharSource) -> Frame`)

**Interfaces:**
- Produces:
  - `frame.rs`: `struct Frame { status: String, status_accent: Option<String>, layer_color: u8, keys: Vec<KeyDraw>, recipes: Vec<Recipe>, connected: bool }`, `struct KeyDraw { index: u8, legend: Legend, color_layer: u8, halo: bool }`, `struct Recipe { chips: Vec<Chip> }`, `enum Chip { Layer(u8), Mod(&'static str), Key(String, u8 /*layer colour*/), Text(String) }`.
  - `geometry.rs` (unscaled logical px, left-half coordinates; right half mirrored about `x = 470`): `fn key_rect(key: &PhysicalKey) -> (cx, cy, rot_deg)` from the Global Constraints stagger/thumb values, `fn case_outline(hand) -> Vec<(f32, f32)>` (convex hull of all key corners, to be stroked with width 30 round-join in `#ededed`), `fn panel_size(scale, recipes: usize) -> (w, h)` (`(470*2 + 30 + 2*10) * scale` wide, `(400 + 24 + 10) * scale` high plus `34 * scale` per recipe row), `fn hit_rect_for_key(index, scale) -> Rect`.
  - `placement.rs`: `struct MonitorInfo { name: String, work_area: RECT, is_primary: bool, dpi: u32 }`, `fn monitors() -> Vec<MonitorInfo>` (`EnumDisplayMonitors` + `GetMonitorInfoW` + `GetDpiForMonitor`), `fn place(settings: &Settings, size: (i32, i32), monitors: &[MonitorInfo]) -> (i32, i32, &MonitorInfo)` (anchor + offset inside the chosen monitor's work area, fall back to primary when `settings.monitor` is not found, never mutates settings).
  - `render_d2d.rs`: `struct Renderer` (`ID2D1Factory1`, `IDWriteFactory`, text formats Segoe UI 600 at 18/13/11/10 logical px; falls back to Roboto if present), `fn render(&mut self, frame: &Frame, scale: f32, dpi: u32, opacity: f32) -> BgraBitmap` drawing per spec §6.2: panel, status, two cases, keys with fills by `KeyKind`/layer, legends (primary centre or top-left, secondary bottom-right in `#d63a3a` when RU-style hint is on, mod icon bottom-left, layer stack icon = three rhombi), halo when `halo`, recipe strip under the halves.

- [ ] **Step 1: Write the failing tests** (geometry/placement/frame are pure)

```rust
#[test] fn thumb_keys_are_rotated_15_degrees() { assert_eq!(key_rect(&OP36_KEYS[30]).2, 15.0); assert_eq!(key_rect(&OP36_KEYS[35]).2, -15.0); }
#[test] fn pinky_column_sits_40px_lower_than_middle() { let q = key_rect(&OP36_KEYS[0]); let e = key_rect(&OP36_KEYS[2]); assert_eq!(q.1 - e.1, 40.0); }
#[test] fn right_half_mirrors_left() { let q = key_rect(&OP36_KEYS[0]); let p = key_rect(&OP36_KEYS[9]); assert_eq!(q.0 + p.0, 470.0 + 470.0 + 30.0 /* two halves plus gap, in panel coordinates */); }
#[test] fn case_outline_is_convex_and_contains_all_keys() { /* every key corner inside hull (cross-product test) */ }
#[test] fn panel_grows_per_recipe_row() { assert_eq!(panel_size(1.0, 2).1 - panel_size(1.0, 0).1, 68.0); }
#[test] fn placement_bottom_center_default() { /* one 1920×1040 work-area primary monitor, size (600,300) → x = 660, y = 1040 - 300 - 8 */ }
#[test] fn placement_falls_back_to_primary() { /* settings.monitor = Some("\\\\.\\DISPLAY9") absent → primary used; settings unchanged */ }
#[test] fn frame_marks_held_layer_key_and_recolors() { /* model with layer.active = 1: all KeyDraw.color_layer == 1; key 31 legend.kind == HeldLayer; status == "OP36 · EN · Nav" when layer_names[1] == "Nav" */ }
```

- [ ] **Step 2: Run** `cargo test overlay` → compile error.
- [ ] **Step 3: Implement** the Interfaces block; replace the Task 2 placeholder with `Renderer::render(model.frame())` on every `ModelEvent` that changes the frame (compare frames with `PartialEq` to skip redraws); re-place on `WM_DISPLAYCHANGE`/`WM_DPICHANGED`; call `reassert_topmost` on a 2 s timer.
- [ ] **Step 4: Run** tests → pass. Manual: put the overlay next to `_scratch/design_shot3.png` state 1 (RU) and state 3 (red held): shapes, colours, legends and the held key match; drag a window underneath and click through the overlay; Win+Space flips RU/EN; holding D (home-row Shift) swaps legends.
- [ ] **Step 5: Commit** — `git commit -am "feat(helper): geometry, Direct2D renderer, monitor placement"`

---

### Task 9: Find a key

**Files:**
- Create: `helper/src/find_key.rs`
- Modify: `helper/src/model.rs` (`ModelEvent::Key` → `find`, `FindExpired` timer via `SetTimer` in `main`), `helper/src/overlay/frame.rs` (recipes + halos)

**Interfaces:**
- Produces: `struct FindQuery { hid: u8, shift: bool }` (from `KeyPress`: `hid` via `scancode_to_hid`, `shift = mods_at_press.shift`), `struct Candidate { layer: u8, key_index: u8, needs_shift: bool, implicit_shift: bool }`, `fn search(keymap: &Keymap, q: FindQuery, active_layer: u8) -> Vec<Candidate>` — a position matches when its action (tap part for `TapHold`) is `Key(Hid(h))` with `h == q.hid` and (`q.shift == false` or the action is `KeyWithModifier(h, shift)` → `implicit_shift`, else `needs_shift = true`); ranking: exact (no `needs_shift`) on `active_layer`, exact on other layers (ascending layer), then `needs_shift` ones; max 3. `fn recipe(c: &Candidate, keymap, ctx, chars) -> Recipe`: chips = layer path to reach `c.layer` (`[Layer(1)]`, `[Layer(2)]`, `[Layer(1), Layer(2)]` for layer 3, none for the active layer) + `Mod("⇧")` when `needs_shift` + `Key(label in current hkl, c.layer)`. `FindResult { candidates, started_ms }`; halos on `candidates[*].key_index` and on the thumb keys that carry `LayerOn(layer)` for each layer in the path (positions found by scanning layer 0 for `LayerOn(n)`; for layer 3 both).
- When `search` is empty: `Frame.recipes = [Recipe { chips: [Key(label), Text("not on OP36")] }]`.

- [ ] **Step 1: Write the failing tests** (keymap = `assets/factory_keymap.bin` loaded in tests)

```rust
#[test] fn find_left_bracket_points_to_symbols_layer() { let c = search(&km, FindQuery { hid: 0x2F, shift: false }, 0); assert_eq!((c[0].layer, c[0].key_index), (2, 6)); }
#[test] fn find_ranks_exact_before_shifted() { /* ':' = Semicolon+shift: first candidate layer 2 implicit_shift, second layer 0 key 19 needs_shift */ }
#[test] fn find_on_active_layer_first() { /* query Kc1 with active_layer 1 → first candidate layer 1 */ }
#[test] fn recipe_for_layer3_lists_both_thumbs() { /* Candidate{layer:3,…} → chips start [Layer(1), Layer(2)] */ }
#[test] fn find_result_expires() { /* model.apply(Key(..)); model.apply(FindExpired) → model.find == None */ }
#[test] fn modifier_presses_do_not_trigger_find() { /* KeyPress with hid 0xE1 (LShift) → model.find stays None */ }
```

- [ ] **Step 2: Run** `cargo test find_key` → compile error.
- [ ] **Step 3: Implement**; wire the timer (`settings.find_key_secs`) and the `find_key` toggle.
- [ ] **Step 4: Run** tests → pass. Manual: type `[` on the laptop with RU active → U key teal with halo, blue thumb haloed, strip shows `[L2] + [х]`; type `{` → two rows; type F1 → both thumbs haloed; strip disappears after 2 s and the keyboard drawing does not move.
- [ ] **Step 5: Commit** — `git commit -am "feat(helper): find a key with recipe strip"`

---

### Task 10: Options dialog and autostart

**Files:**
- Create: `helper/src/options/mod.rs`, `helper/src/options/dialog.rs`, `helper/src/options/autostart.rs`
- Modify: `helper/src/tray.rs` (`Options…`), `helper/src/main.rs`

**Interfaces:**
- Produces:
  - `autostart::set(enabled: bool, exe: &Path) -> io::Result<()>` creating/removing the `.lnk` via `IShellLinkW` + `IPersistFile`; `fn is_enabled() -> bool`.
  - `options::open(parent: HWND, settings: Settings, monitors: Vec<MonitorInfo>, on_change: Box<dyn FnMut(Settings)>)`: a normal top-level window (class `Op36HelperOptions`, `WS_OVERLAPPEDWINDOW` minus maximize, DPI-scaled layout) with standard controls: COMBOBOX monitor (`name — W×H`, "Primary" first), 3×3 radio grid for `Anchor`, two EDIT+UPDOWN for offset, TRACKBAR scale 30–120 (%), TRACKBAR opacity 0–100, COMBOBOX display mode, `msctls_hotkey32` for the hotkey, CHECKBOX autostart, CHECKBOX find-key + TRACKBAR 5–50 (×0.1 s), CHECKBOX show EN hint, a status GROUPBOX (device, layer notify, keymap source) with `Reload keymap`, buttons `OK`, `Cancel`, `Apply`. `Apply`/`OK` call `on_change` and save; sliders call `on_change` live (preview), `Cancel` restores the entry settings.
  - `options::apply_hotkey(hwnd, &Hotkey) -> Result<(), String>` re-registers; on failure the dialog shows the error in the status group and keeps the old hotkey.
  - `fn settings_from_controls(&self) -> Settings` and `fn controls_from_settings(&mut self, &Settings)` as the only two places that know control ids.

- [ ] **Step 1: Write the failing tests** (pure bits)

```rust
#[test] fn scale_slider_maps_percent_both_ways() { assert_eq!(slider_to_scale(60), 0.6); assert_eq!(scale_to_slider(1.2), 120); }
#[test] fn hotkey_control_word_round_trips() { /* msctls HOTKEY word: lo = vk, hi = HOTKEYF_CONTROL|HOTKEYF_ALT ↔ Hotkey { MOD_CONTROL|MOD_ALT, 0x4B } */ }
#[test] fn monitor_combo_index_zero_is_primary() { /* combo_to_monitor(0, &monitors) == None; combo_to_monitor(1, …) == Some(name) */ }
#[test] fn startup_shortcut_path_is_in_user_startup_folder() { assert!(paths::startup_shortcut().ends_with(r"Programs\Startup\OP36 Helper.lnk")); }
```

- [ ] **Step 2: Run** `cargo test options` → compile error.
- [ ] **Step 3: Implement** the Interfaces block.
- [ ] **Step 4: Run** tests → pass. Manual: open Options from the tray; move the overlay to the external monitor, bottom-right, scale 45 %, opacity 60 %; OK; restart the app → same placement; set hotkey Ctrl+Alt+O and toggle with it; enable autostart → `.lnk` exists in the Startup folder; disable → gone; unplug the external monitor → overlay appears on the laptop screen, Options still lists the saved monitor name as selected-but-missing.
- [ ] **Step 5: Commit** — `git commit -am "feat(helper): options dialog, autostart shortcut, live preview"`

---

### Task 11: Robustness, cache, status, release build, acceptance

**Files:**
- Modify: `helper/src/hid_thread.rs`, `helper/src/model.rs`, `helper/src/main.rs`, `helper/src/tray.rs`
- Create: `helper/README.md` (build, run, settings file, troubleshooting), `docs/superpowers/plans/2026-10-28-helper-acceptance.md` (the checklist below, ticked with results)

**Interfaces:**
- Produces: cache write of `keymap.bin` after every successful live read; `Model::status_line()` = `OP36 · {lang} · {layer name}` plus ` · offline` when disconnected and ` · no layer data` when `layer_notify == Some(false)`; tray tooltip = the same line; log rotation at 1 MB keeping one backup; `WhileHeld` display mode: visible while `layer.active != default` or any modifier held or `find.is_some()`, hidden 1 s after (fade = 5 steps of opacity over 250 ms); release profile `opt-level = "s"`, `lto = true`, `strip = true`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn status_line_offline() { /* disconnected → "OP36 · RU · Base · offline" */ }
#[test] fn status_line_without_layer_notify() { /* connected, layer_notify Some(false) → ends with "· no layer data" */ }
#[test] fn while_held_mode_visibility() { /* mods none, layer 0, no find → hidden; layer 1 → shown; FindResult → shown */ }
```

- [ ] **Step 2: Run** `cargo test status visibility` → fail.
- [ ] **Step 3: Implement**; `cargo build --release`; confirm the exe is a single file under 6 MB.
- [ ] **Step 4: Acceptance checklist** (manual, with the flashed left half on USB), record each result:
  1. Start from a clean `%APPDATA%\op36-helper`: overlay appears bottom-centre with the bundled keymap, then switches to the live keymap within 2 s (log shows `keymap: live`).
  2. Hold red / blue / both thumb keys: overlay recolours within one frame, status names the layer.
  3. Win+Space to RU and back: legends switch.
  4. Hold Shift (laptop) and home-row D (OP36): legends swap; release: back.
  5. Hold Ctrl: EN letters shown.
  6. Type `х` on the laptop in RU: find halo + recipe; expires after 2 s.
  7. Unplug the keyboard: tray grey, status `offline`, overlay still drawn; replug: live again, layer follows.
  8. Entropy opened while the helper runs: Entropy connects and shows the layout.
  9. Options: every control round-trips through restart.
  10. Second launch of the exe toggles the first instance.
  11. CPU in Task Manager at idle: 0 %; no redraws without events (log at debug level shows none).
- [ ] **Step 5: Commit** — `git add -A helper docs && git commit -m "feat(helper): robustness, cache, status line, release profile, acceptance record"`

---

## Self-review notes

- Spec §6.1 window/placement/size/opacity/display mode → Tasks 2, 8, 11. §6.2 visuals → Task 8. §6.3 legend rules 1–8 → Task 6 (+ layer names in Task 4/5 via `layer_names`, read with Vial QSID 200–215 only if cheap; otherwise `Layer n` — Task 4 may add `req_qmk_setting(qsid)` if Entropy's `hid.rs` shows the packet format in under an hour, else leave `Layer n` and note it). §6.4 data sources → Tasks 4, 5, 7. §6.5 tray/hotkey/options/settings/single instance/logging → Tasks 1, 2, 10, 11. §6.6 find a key → Task 9. §6.7 edge cases → Tasks 5, 8, 11. §7 stack and repo layout → Task 1 and the firmware plan's Task 1.
- Names reused across tasks: `Settings`, `Anchor`, `Hotkey`, `DisplayMode`, `Keymap`, `PhysicalKey`, `OP36_KEYS`, `key_at`, `Packet`, `LayerState`, `VialTransport`, `LayerClient`, `HidEvent`, `Model`, `ModelEvent`, `Modifiers`, `Hkl`, `KeyPress`, `DeviceKind`, `Legend`, `KeyKind`, `LegendCtx`, `CharSource`, `Frame`, `KeyDraw`, `Recipe`, `Chip`, `FindQuery`, `Candidate`, `FindResult`, `MonitorInfo`, `BgraBitmap`.
- Bluetooth transport, Universal Symbols decoding and `0xAC` layout sync stay out (spec §9).
