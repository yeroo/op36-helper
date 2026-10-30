# OmegaPoint36 Layout Helper — Design

Date: 2026-10-28
Status: reviewed by builder check, awaiting Boris's approval
Owner: Boris Kudriashov (user). Author of this document: Claude, from the brainstorming session.

## 1. Intent

Boris types on an Ergohaven OmegaPoint36 (OP36), a 36-key split keyboard with four
layers and home-row modifiers, switching between English and Russian Windows
layouts. He wants a small always-available on-screen reminder of what every key
does *right now*: in the current Windows language, on the layer that is currently
held, and with the Shift / Ctrl / Alt / Win variants visible. The reference look is
the vendor's documentation (see `../../../1.png` … `4.png` in the repo root):
shaped half-cases, staggered columns, angled thumb keys, EN letter top-left, RU
letter red bottom-right, layers colored red / teal / yellow.

Two deliverables make this possible:

1. **A firmware patch** to the vendor's RMK firmware fork so the keyboard pushes
   its active layer to the host. Without it the host cannot know a layer key is
   held: layer keys never leave the keyboard.
2. **A Windows tray application in Rust** ("the helper") that draws the overlay.

## 2. Goals, non-goals, anti-goals

Goals

- Overlay is on top of all normal windows, fully click-through, never takes
  focus, never appears in Alt-Tab or the taskbar.
- Shows the real keymap currently on the keyboard, read from the keyboard.
- Legends follow the Windows keyboard language of the foreground window (EN/RU
  today; any installed layout in principle).
- Reflects the active layer live, within ~50 ms of the layer key going down/up.
- Reflects Shift / Ctrl / Alt / Win being held (physical or home-row).
- Tray icon: left-click toggles visibility; menu: Show/Hide, Options…, Exit.
- Global hotkey toggles visibility.
- Options dialog: monitor, placement, size, opacity, hotkey, autostart.
- "Find a key": pressing a key on any keyboard shows how to type it on the OP36.
- Firmware patch is small, opt-in from the host, and does not disturb the
  vendor's Entropy app when the helper is not running.

Non-goals (v1)

- Bluetooth transport for layer state and keymap reading. Boris types over USB
  today and will move to Bluetooth later; see §9 Phase 2.
- Editing the keymap. Entropy does that.
- Combos, tap-dance and macro contents (pressed-key highlight comes for free
  from "Find a key", see §6.6).
- Support for other Ergohaven boards or the Qube dongle (keep the code generic
  where it is free, but do not test or promise it).

Anti-goals (would make it a failure even if it "works")

- Injecting fake keystrokes (e.g. F13) to signal layers. Rejected by Boris.
- Writing to the Windows registry, requiring admin rights, code signing,
  or calling external services. Same IT constraints as the MeetNow project:
  user-level, offline, no registry.
- An overlay that steals clicks or focus, even briefly.
- A firmware change that changes typing behaviour or breaks Entropy.

## 3. Context and facts established

Hardware / firmware

- OP36 standalone split, nRF52840 in each half, UF2 bootloader ("NRF52BOOT"
  drive, double-tap Reset on the underside). Left half is the central: only it
  talks USB/Bluetooth to the host; the right half talks to the left over BLE.
- On 2026-10-28 the keyboard was migrated from ZMK to the vendor's official
  **RMK v0.2.0** (release 2026-10-15, commit `347cf14f…`). Files, checksums and
  ZMK backups are in `firmware/` in this repo:
  `firmware/op36-v0.2.0/op36_{left,right}.uf2`, `firmware/backup-zmk-{left,right}/CURRENT.UF2`,
  `firmware/zmk-rollback/`, `firmware/settings-reset/settings_reset.uf2`.
- USB identity after migration: VID `0xE126`, PID `0x0036`, product "OP36",
  manufacturer "Ergohaven". Interfaces: keyboard (MI_00), mouse/consumer/system
  (MI_01), **Vial raw HID vendor collection (MI_02, usage page `0xFF60`,
  usage `0x61`)**, and a CDC serial port (MI_03, COM6 today, defmt logs).
- Vial raw HID carries fixed 32-byte packets both ways. The firmware's Via
  service (`rmk/src/host/via/mod.rs`) receives packets from
  `HOST_REQUEST_CHANNEL` and answers with `try_send_host_reply(transport, [u8;32])`
  (`rmk/src/channel.rs`). That reply path can carry unsolicited packets.
- Firmware already publishes `LayerChangeEvent(u8 active_layer)`
  (`rmk/src/event/state.rs`, published from `rmk/src/keymap.rs`) whenever the
  resolved active layer changes. Processors subscribe with the
  `#[processor(subscribe = [...])]` macro (see `rmk/src/display/mod.rs`).
- Via command bytes already in use that the helper must not collide with:
  standard VIA `0x01…0x12`, Vial prefix `0xFE`, host-data `0xAA…0xAE`
  (Entropy → keyboard: time, volume, **layout EN/RU = 0xAC**, media), and the
  Ergohaven custom namespace `0xE8` under `CMD_VIA_CUSTOM_GET_VALUE (0x08)` /
  `SET_VALUE (0x07)` with sub-ids `0x01…0x07` (battery halves, native key
  action get/set, native dynamic actions, combo layer).
- OP36 RMK keyboard definition: `keyboards/op36/keyboard.toml` in the fork.
  Matrix 6 rows × 6 cols, 16 layers (5–15 compiled as `No`), `matrix_map`
  gives physical key → (row, col). Vial unlock keys `[[1,2],[1,1]]`.
  Factory layers: Base, Nav, Symbols, Adjust (see that file for the exact keys;
  they differ slightly from the ZMK-era screenshots in Nav and Symbols).
- Vendor code available locally for reference (read-only clones, not part of
  this repo): `_ref/rmk` (firmware fork, shallow), `_ref/entropy` (vendor host
  app, Rust, egui), `_ref/ergohaven-zmk-config` (the old ZMK keymap matching the
  screenshots), `_ref/zmk-studio-messages`.
- Entropy's own "Layout Indicator" polls the Vial switch-matrix state and
  *simulates* the layer. It needs Vial unlock. This design does not use that.

Host machine

- Windows 11 Enterprise, two monitors at the office (laptop + external).
- Rust stable 1.99 with target `thumbv7em-none-eabihf`, `llvm-tools`,
  `llvm-objcopy` (scoop llvm, includes `clang.exe` and `libclang.dll`), Python
  3.12, `gh` CLI logged in as GitHub user `yeroo`. No `arm-none-eabi-gcc` yet.
- The firmware CI (`.github/workflows/build.yml`) builds with
  `cargo build --release --bin central|peripheral` in `keyboards/op36`, converts
  with `objcopy -O binary` + `uf2conv.py --base 0x26000 --family 0xADA52840`,
  and sets `BINDGEN_EXTRA_CLANG_ARGS` to an ARM newlib sysroot because the
  `nrf-sdc`/`nrf-mpsl` dependencies run bindgen.

## 4. System overview

```
 ┌───────────── keyboard (left half, RMK) ─────────────┐
 │ keymap ──LayerChangeEvent──► layer_notify processor │
 │                                   │ (if subscribed)  │
 │                      Vial raw HID 32-byte packets    │
 └───────────────────────────┬──────────────────────────┘
                             │ USB (v1) / BLE (phase 2)
 ┌───────────── helper (Rust, Windows) ─────────────────┐
 │ hid thread: subscribe/keepalive, read keymap,        │
 │             receive layer notifications              │
 │ input: WH_KEYBOARD_LL hook → modifier state          │
 │ language: foreground window HKL, poll + WinEvent     │
 │ model: keymap + layer + mods + HKL → legends         │
 │ overlay: layered click-through window, DPI aware     │
 │ tray icon, global hotkey, Options dialog, settings   │
 └──────────────────────────────────────────────────────┘
```

## 5. Part 1 — Firmware patch ("layer notify")

Behaviour

- The keyboard sends a **layer notification** packet to the host whenever the
  active layer changes, but only while a host is **subscribed**.
- Subscription is a Vial packet from the host and expires after a TTL unless
  renewed. The helper renews every 20 s with a 60 s TTL. When nobody is
  subscribed the keyboard sends nothing extra, so Entropy and any other Vial
  client behave exactly as before.
- On subscribe (and on explicit request) the keyboard replies with the current
  state immediately, so the helper never starts in an unknown layer.
- Sent on the transport the subscription came from (USB today; the type
  already distinguishes `HostTransport::Usb` / `Ble(endpoint)`, so BLE later is
  a matter of the host side).

Packet formats (32 bytes, unused bytes zero)

| Direction | Bytes | Meaning |
|---|---|---|
| host → kb | `0x07 0xE9 0x01 ttl_s` | Subscribe / renew. `CMD_VIA_CUSTOM_SET_VALUE`, helper namespace `0xE9`, sub-id 1. `ttl_s = 0` unsubscribes. |
| host → kb | `0x08 0xE9 0x01` | Get current layer state once. |
| kb → host | `0xE9 0x01 active default lo hi fw_layout` | Notification (and reply to both requests above). `active` = resolved active layer, `default` = default layer, `lo/hi` = 16-bit bitmask of layers currently on, `fw_layout` = the firmware's own host-layout byte as last set by `0xAC` (0 = EN/unknown, 1 = RU), carried now so the packet never needs a second version when Phase 2 adds Universal Symbols support. |

Design choices behind this: the namespace `0xE9` is adjacent to the vendor's
`0xE8` but distinct, so a future vendor sub-id cannot collide. The
notification's first byte is `0xE9`, which is not a VIA command id, so a Vial
client that receives one unexpectedly (Entropy running at the same time as the
helper) sees an unknown command byte rather than a plausible reply to its own
pending request. Replies to the two requests use the same `0xE9 0x01 …` shape
so the helper has one parser.

Implementation requirements (how is left to the builder, within these)

- Lives in the RMK fork as a small, self-contained addition (new module under
  `rmk/src/host/`, a few lines in the Via dispatcher, one task/processor
  subscribing to `LayerChangeEvent`), behind a Cargo feature enabled for
  `keyboards/op36`. Rebase-friendly: minimal edits to existing files.
- Must not block the keyboard path: use the existing non-blocking
  `try_send_host_reply`; drop the notification if the queue is full.
- `[event] layer_change.subs` in `keyboards/op36/keyboard.toml` must be raised
  (the split driver already holds the single default subscriber; a second
  subscriber without this panics at boot — found by the design review).
- Unit-tested in the host-side `cargo test` of the `rmk` crate like the
  existing Via tests (subscribe → notification emitted; TTL expiry → silence;
  unknown sub-id → unhandled).
- Source control: a full clone of `ergohaven/rmk` in `rmk/` inside this repo's
  folder (git-ignored by the parent, it is its own repository), branch
  `op36-layer-notify`, pushed to a fork under `yeroo` so GitHub Actions can
  build it with the vendor's existing workflow if the local build fights with
  bindgen. Builder's call which build path to use; both produce the same
  `op36_left.uf2` / `op36_right.uf2` at `--base 0x26000`. Branch from
  `347cf14f`, the exact commit the shipped v0.2.0 was built from, so the only
  behavioural delta is the patch (the fork's `main` is a few commits ahead).
- Only the **left** half needs the new firmware (layers resolve on the central).
  Flash the right half too only if the fork's split protocol requires matching
  versions; check `rmk/src/split` before deciding, and say which in the plan.
- Flash procedure and rollback are the vendor's: double-tap Reset, copy the
  UF2 to the `NRF52BOOT` drive. Rollback = flash stock v0.2.0 from `firmware/`.

## 6. Part 2 — Windows helper

### 6.1 Overlay window

- One top-level window: `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST |
  WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, `WS_POPUP`, no taskbar/Alt-Tab entry.
  Rendered with per-pixel alpha (`UpdateLayeredWindow`), so the rounded
  cases and the dark panel blend over anything underneath.
- Re-asserts `HWND_TOPMOST` when it loses the top position (some apps steal
  it). It is fine not to cover exclusive-fullscreen games.
- Per-monitor DPI aware (v2). Geometry is defined in logical units and scaled
  by DPI × user scale.
- Redraws only when the model changes (layer, modifiers, language, keymap,
  settings); no busy loop.
- Placement = chosen monitor + one of nine anchor positions (corners, edge
  centres, centre) + x/y offset in pixels. Default: primary monitor, bottom
  centre, 8 px above the taskbar work-area edge. The window is click-through,
  so it cannot be dragged; position is set in Options.
- Size = scale factor, default 0.6 of the documentation size (two halves
  ≈ 600 px wide at 100 % DPI), range 0.3–1.2, live preview while dragging the
  slider in Options.
- Opacity of the dark panel 0–100 %, default 80 %. Keys stay opaque.
- Display mode: "always on while enabled" (chosen) with an additional option
  "only while a layer key or Shift/Ctrl/Alt/Win is held, fade out 1 s after
  release" for later use. Fade is a simple opacity ramp.

### 6.2 Visual design

Match the documentation screenshots and the approved mockup
(`.superpowers/brainstorm/*/content/design-v3.html`, generator
`_scratch/gen_mock2.py`, rendered check `_scratch/design_shot2.png`):

- Dark rounded panel (`rgba(28,30,36,0.80)`) with a one-line status in the
  corner: `OP36 · RU · Nav` plus a dim "not connected" marker when applicable.
- Two half-cases, light grey (`#ededed`) rounded silhouettes hugging the keys
  (convex hull of the key rectangles with ~15 px margin and round joins is an
  acceptable approximation of the real case).
- 3 × 5 keys per half, pitch 72, key 64, corner radius 9, column vertical
  stagger `[+40, +10, 0, +10, +20]` from pinky to inner column; three thumb
  keys per half rotated 15° (left half clockwise, right half mirrored),
  starting under the middle finger column and descending toward the centre.
  Thumb order outer→inner: left `Bksp, L1, Space`; right `Tab, L2, Enter`.
  (Positions are factory defaults for illustration; actual legends come from
  the keymap.)
- Base layer: white keys, home-row keys (A S D F, J K L ;) light grey.
  Key colours per layer index: 0 white; 1 red `#d9414c` (text white);
  2 teal `#1b8da3` (text white); 3 yellow `#f3e39a` (text dark);
  4+ a muted palette the builder picks. Keys with no action on the active
  layer: pale tint of the layer colour (`#f3aeb3`, `#b9dfe8`, `#faf2cf`).
- The held layer key is drawn white with the layer-stack icon in its layer's
  colour. A key that *leads* to another layer from the current one (e.g. MO(3)
  on Nav) shows the stack icon in the target layer's colour.
- Legend positions: primary large at centre (or top-left when a secondary is
  present), secondary bottom-right, home-row modifier icon bottom-left small.
  Long labels (`Page Down`, `Prt Sc`, `Insert`) shrink to fit, two lines allowed.
- Modifier icons: `⊞` Win, `⇧` Shift, `^` Ctrl, `⎇` Alt (Windows idiom, not
  the mac `⌘ ⌥` from the screenshots).
- Fonts: Segoe UI (Roboto if installed), weight 600 for legends. Cyrillic
  must render crisply; use a text engine with proper shaping/anti-aliasing
  (DirectWrite recommended).

### 6.3 Legend rules

Inputs: keymap action for (layer, position); active layer; held modifiers
(from Windows); foreground keyboard layout `HKL`.

1. Resolve the action on the active layer; `Transparent` falls through to the
   next lower active layer, then the default layer. `No` = empty pale key.
2. Decompose the action: basic key, key with implicit modifiers
   (`WM`/`SHIFTED`), mod-tap (tap key + hold modifier), layer-tap, momentary
   layer, toggle/to layer, consumer/media, system (bootloader, output USB/BLE,
   Bluetooth profile "User" keys), macro, tap-dance/combo output.
3. For a basic HID keycode: map HID usage → Windows scan code (standard
   table) → virtual key via `MapVirtualKeyExW(…, MAPVK_VSC_TO_VK_EX, hkl)` →
   character via `ToUnicodeExW` with the needed modifier state for `hkl`,
   without disturbing the real dead-key state (flag bit 2 on Windows 10+).
   If no printable character results, use a short label table: `Bksp, Enter,
   Tab, Esc, Space, Del, Ins, Home, End, Page Up, Page Down, Prt Sc, Caps,
   ← ↑ ↓ →, F1…F24, Vol+, Vol−, Mute, ⏮ ⏯ ⏭`, and for firmware actions
   `BT 0…4, BT CLR, Out BLE, Out USB, Flash`. The firmware's "Universal
   Symbols" / "Russian letters" / layout-switch `User(n)` actions (see
   `_ref/rmk/rmk/src/universal_symbols/mod.rs`) are **not** in the factory
   keymap today; v1 shows them with a generic `Sym n` label. Full support
   (decoding them and sending the host layout with `0xAC`) is Phase 2.
4. Language presentation. Let `L` be the foreground layout and `EN` the
   installed Latin layout (first installed `0x0409`-family layout, else load
   `00000409`).
   - If `L` is not `EN` and the key yields a letter/symbol in both: primary =
     char in `L` (red), secondary hint = char in `EN` (dark, top-left), as in
     the screenshots.
   - If `L` is `EN`: primary = char; secondary = shifted char when it differs
     by more than case (`;`→`:`, `,`→`<`), nothing for letters.
   - On layers whose keys carry an implicit Shift (`SHIFTED(Kc3)`): primary
     = the shifted char in `L` (e.g. `№`), secondary = the unshifted/EN form
     (`#`), giving `# №`, `^ :`, `& ?` as in screenshot 3.
5. Shift held (physical or home-row): primary becomes the shifted character
   for `L`; secondary hidden. Caps-lock is ignored in v1.
6. Ctrl, Alt or Win held: all keys show the `EN` uppercase letter / symbol
   (what shortcuts use), secondary hidden, panel status shows the held
   modifiers. Shift combined with these behaves like 6, not 5.
7. Mod-tap keys show the tap legend per rules 3–6 plus the hold modifier icon.
   Layer-tap shows the tap legend plus the stack icon.
8. Layer names come from the keyboard when readable (Ergohaven layer names,
   QSID 200–215), else `Layer n`.

### 6.4 Data sources

- **Keymap**: read over Vial at connect and whenever a (re)connect happens, and
  re-read on demand from Options ("Reload keymap"). Use the Ergohaven *native
  key action* protocol (`0x08 0xE8 0x03/0x04`, see `_ref/entropy/src/hid.rs`,
  `hid_protocol.rs`, `keycode.rs` and `_ref/rmk/rmk/src/host/via/mod.rs`) so
  rich actions like `MT(WM(Kc9,LShift),LAlt)` decode losslessly; fall back to
  standard VIA `dynamic_keymap_get_keycode` for anything the native protocol
  does not return. Only layers 0–4 need reading in v1 (5–15 are `No`), but do
  not hardcode 5. Cache the last good keymap on disk and use it, plus a bundled
  copy of the factory keymap from `keyboards/op36/keyboard.toml`, when the
  keyboard is not connected (overlay shows a "not connected" marker).
- **Layer**: notifications per §5. On connect: subscribe, request state, renew
  every 20 s. On loss of the device: show base layer, grey tray icon, retry
  connect every 2 s. Device = HID device with VID `0xE126`, PID `0x0036`,
  usage page `0xFF60`, usage `0x61` (`hidapi` crate exposes these).
- **Modifiers and key presses**: **Raw Input**, not a low-level hook.
  `RegisterRawInputDevices` for the keyboard usage with `RIDEV_INPUTSINK` on
  the helper's hidden message window delivers every key press system-wide
  with scan code, VK, make/break flag and the **source device handle**. Chosen
  over `WH_KEYBOARD_LL` because an unsigned user-level exe installing a global
  hook is what endpoint security flags, no existing project of Boris's uses
  one, and the device handle lets "Find a key" know whether the press came
  from the OP36 (VID `0xE126`) or another keyboard. Track left/right Shift,
  Ctrl, Alt, Win from these events and reconcile with `GetAsyncKeyState` on
  focus changes so modifiers never stick.
- **Language**: `GetKeyboardLayout(thread of GetForegroundWindow())`, refreshed
  on `EVENT_SYSTEM_FOREGROUND` and `EVENT_OBJECT_FOCUS` via `SetWinEventHook`,
  plus a 250 ms poll because console hosts and UWP frame windows report
  stale layouts; for `ConsoleWindowClass` / `ApplicationFrameWindow` fall back
  to the layout of the focused child via `GetGUIThreadInfo` where possible,
  else the last known layout.

### 6.5 Tray, hotkey, options, settings

- Tray icon (Shell_NotifyIcon): coloured when the keyboard is connected, grey
  otherwise; tooltip = status line. Left-click toggles the overlay. Right-click
  menu: `Show/Hide overlay`, `Options…`, `Exit`. Nothing else in the menu.
- Global hotkey via `RegisterHotKey`, default **Ctrl+Alt+K**, changeable in
  Options with a capture field; conflicts are reported, not silently ignored.
- Options dialog (a normal focusable window, opened from the tray or by
  double-click on the tray icon):
  - Display: monitor dropdown (friendly name + resolution, remembers by device
    name and falls back to primary when unplugged), 3×3 anchor picker, x/y
    offset, scale slider, panel opacity slider, display mode
    (always / while held).
  - Behaviour: hotkey, "Start with Windows" (shortcut in the user's Startup
    folder, no registry), "Show EN hint in non-Latin layouts" toggle,
    "Find a key" on/off and its display duration.
  - Keyboard: status (device found, firmware supports layer notify yes/no,
    keymap read OK/cached/bundled), `Reload keymap` button, log excerpt.
  - About: version, link to this repo folder.
- Settings in `%APPDATA%\op36-helper\settings.json`, written atomically.
  Missing/corrupt file → defaults. Cached keymap next to it.
- Single instance (named mutex); a second launch toggles the existing one.
- Logging to `%APPDATA%\op36-helper\log.txt` with rotation, info level.

### 6.6 "Find a key" (requested by Boris)

Problem: he sometimes cannot find where a character or key lives on the OP36.
Behaviour: whenever a key is pressed on **any** keyboard (laptop, OP36, any
other), the overlay shows how to produce that same key on the OP36.

- Input comes from the same Raw Input stream as the modifiers: scan code (with
  the extended flag) + the modifier state at the moment of the press, plus the
  source device (OP36 or other keyboard). Convert
  the scan code to the HID usage (reverse of the table in §6.3 rule 3), so the
  lookup is independent of language: `х` typed in RU and `[` typed in EN are
  both HID `LeftBracket`.
- Search every readable layer (0–4 today, do not hardcode) for actions whose
  output is that usage with the same implicit modifiers; e.g. pressing `{`
  (Shift+LeftBracket) matches `WM(LeftBracket, LShift)` on Symbols *and*
  base-layer `[` + Shift. Rank: exact match on the active layer first, then
  exact matches on other layers, then matches that need an extra Shift.
- Presentation, for 2 s (configurable 0.5–5 s, or off) after the press:
  - every matching key position is drawn in the colour of the layer it is on
    (teal for Symbols) with a bright yellow halo, legend = what it produces;
  - the thumb key(s) that must be held to reach that layer get the same halo
    (both thumb keys for the Adjust layer);
  - **a recipe strip appears below the two halves, inside the panel**, one
    row per way to type the key, as a sequence of mini keycaps joined by `+`:
    `[▣ L2] + [х]`, `[▣ L2] + [⇧] + [х]`, `[⇧] + [х]`. Mini keycaps use the
    colour of the layer they belong to; the layer keycap carries the stack
    icon; modifier keycaps show `⇧ ^ ⎇ ⊞`; the final keycap shows the key's
    legend **in the current Windows language only** (`х` when RU is active,
    `[` when EN is active). Rows are ordered by rank above, at most three; the
    strip grows the panel downward and collapses when the timer ends, so the
    keyboard drawing never moves.
  - If no match exists (e.g. a key the OP36 keymap does not contain), the
    strip shows one row: `[key]  not on OP36`.
  - The lookup itself stays physical (HID usage), so results do not depend on
    language; only the labels do.
- Keys typed on the OP36 itself go through the same path, which doubles as a
  pressed-key highlight on the current layer. Modifier-only presses (Shift,
  Ctrl, Alt, Win) do not trigger find mode; they are handled by rules 5–6.
- Find mode never changes the displayed layer; it only adds halos and the
  status recipe on top of the current rendering, so holding a layer key while
  the halo is visible still works as before.
- Mockup state 6 in `design-v3.html` shows the intended look.

Layer 3 (Adjust, yellow) is reached by holding both thumb layer keys; the
firmware resolves it as a tri-layer (`layer_state[3] = layer1 && layer2`). The
overlay simply renders whatever active layer the notification reports; mockup
state 5 shows it. Both held thumb keys are drawn white with the yellow icon.

### 6.7 Error handling and edge cases

- Firmware without the patch (stock v0.2.0): keymap still reads, layer stays
  at base, Options shows "layer notify: not supported by firmware".
- Entropy running at the same time: both can open the Vial collection on
  Windows; the helper's packets are designed to be ignorable by Entropy
  (§5). Document in Options that Entropy's Matrix Tester may be confused
  while the helper is subscribed.
- Keyboard sleeps/wakes or is unplugged: reconnect loop, no crash, no
  stuck modifiers.
- Monitor removed: fall back to primary without losing the saved choice.

## 7. How it gets made (decided)

- Language: **Rust** (Boris's choice) for both the helper and the firmware
  patch. Windows API through the `windows` crate; HID through `hidapi`
  (hidapi-rs, Windows backend) or `windows` HID calls — builder's call.
- Rendering: Direct2D + DirectWrite into a DXGI/GDI-compatible surface pushed
  via `UpdateLayeredWindow` is the recommendation (crisp Cyrillic text, paths
  for the case silhouette). A CPU rasterizer (`tiny-skia` + a glyph rasterizer)
  is acceptable if text quality is equivalent.
- Options dialog: builder's call between native Win32 controls and `egui`
  (`eframe`). If `egui`, it runs only while the dialog is open.
- Repository: this folder `C:\Users\boris_kudriashov\Source\omega36` becomes a
  git repo. Layout: `helper/` (Rust crate `op36-helper`), `firmware/`
  (release zips, backups, flash notes; large blobs kept, they are < 2 MB each),
  `docs/` (this design, plans), `rmk/` (clone of the fork, git-ignored),
  `_ref/` and `_scratch/` and `.superpowers/` git-ignored.
- Tests: pure-function unit tests for keycode decoding, legend rules (with
  injectable `to_unicode`/`map_vk` so tests do not depend on installed layouts),
  protocol encode/decode, settings round-trip. Firmware: host-side unit tests in
  the `rmk` crate. Manual acceptance with the real keyboard, scripted as a
  checklist in the plan.
- Build outputs: `helper/target/release/op36-helper.exe`, single file, no
  installer. Autostart = Startup-folder shortcut created by the app.

## 8. Left to the builder

- Module structure, threading model (one HID thread, one UI thread, channels),
  crate choices within §7.
- Exact HID-usage→scan-code table source and the label table contents.
- Case-silhouette geometry details beyond §6.2 as long as the result matches
  the mockup closely.
- Firmware module placement and feature naming; whether the right half must
  be reflashed (§5).
- Local vs CI firmware build path (§5).
- Palette for layers 4+.

## 9. Phase 2 (explicitly out of scope now, keep doors open)

- Bluetooth: Windows exposes the BLE HID vendor report as a HID collection but
  with quirks (Entropy's `hid.rs` has `HidTransport::Bluetooth` handling and an
  "unsafe Bluetooth report map" guard). Keep the transport behind a trait so the
  BLE path is an addition, not a rewrite. Firmware side already carries the
  transport in the subscription.
- Pressed-key highlighting, combos and tap-dance display, telling the
  keyboard the host layout (`0xAC`) so firmware "Russian letters" features work.
- Display mode "only while held" polish, animations.
- Split-link latency. With RMK v0.2.0 Boris sees right-hand keys arriving
  after later left-hand keys in the middle of sentences. The active split link
  runs at 15 ms (`active_central_conn_param`, `rmk/src/split/ble/central.rs`)
  and drops to a ~210 ms idle cadence after 120 s; ZMK used 7.5 ms. A tweak to
  the cadence is possible in the same custom firmware but is explicitly
  deferred by Boris (2026-10-28); the layer-notify patch must not make it
  worse (no extra work on the key path).

## 10. References

- Vendor firmware fork: https://github.com/ergohaven/rmk (local `_ref/rmk`)
- Releases: https://github.com/ergohaven/rmk/releases (v0.2.0 used)
- Vendor host app: https://github.com/ergohaven/entropy (local `_ref/entropy`)
- Old ZMK keymap matching the screenshots: `_ref/ergohaven-zmk-config/config/op36.keymap`
- Flashing docs: https://docs.eh.works/firmware
- Mockups: `.superpowers/brainstorm/34429-1793187793/content/design-v2.html`
