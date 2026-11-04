# op36-helper (developer notes)

User documentation: the [repository README](../README.md). This file covers building and the CLI.

Tray application (Windows and Linux) that draws a click-through overlay of an Ergohaven OmegaPoint36 keymap in
the vendor's documentation style: live layer from the keyboard, legends in the current Windows
language, Shift / Ctrl / Alt / Win variants, and "find a key" for any key pressed on any keyboard.

## Build and run

```
cargo build --release          # helper/target/release/op36-helper.exe, single file, no installer
cargo run                      # debug build with console logging
cargo test
```

Dependencies: the `rmk-types` crate from the firmware clone at `../rmk` (path dependency), so the
`omega36/rmk` clone must exist (see `firmware/FLASHING.md`).

## Platforms

The model, keymap, legends, Vial protocol, geometry, firmware and update logic are shared. Each
platform supplies the window, renderer, tray, key input and language source:

| | Windows (`win_app.rs`) | Linux (`linux/`) |
|---|---|---|
| Overlay | layered window, `UpdateLayeredWindow` | X11/XWayland ARGB dock window, empty input shape (`x11.rs`) |
| Renderer | Direct2D + DirectWrite (`render_d2d.rs`) | tiny-skia + ab_glyph outlines (`render_skia.rs`) |
| Tray | `Shell_NotifyIcon` | StatusNotifierItem via ksni (`tray.rs`) |
| Keys | Raw Input | evdev (`keys.rs`); needs `/dev/input` access |
| Language, legends | foreground `HKL`, `ToUnicodeEx` | GNOME input sources, xkbcommon (`desktop.rs`, `xkb.rs`) |
| Options | Win32 dialog | egui window in a child process (`options_ui.rs`) |
| Reserve space | app bar | `_NET_WM_STRUT_PARTIAL` |
| Single instance, `--toggle` | named mutex, window message | Unix socket in `$XDG_RUNTIME_DIR` (`instance.rs`) |
| Update | Inno setup / exe swap via PowerShell | tarball: binary renamed over, firmware refreshed |

Worker threads (`hid_thread`, `firmware_thread`, `update_thread`) report through a `sink::Sink`
callback; on Windows it posts a boxed message to the handler window, on Linux it feeds the main
loop's channel.

Check the Windows build from Linux: `cargo check --target x86_64-pc-windows-gnu` (needs a mingw C
compiler on `PATH` for hidapi and ring).

## Using it

- Tray icon: left-click shows/hides the overlay, right-click opens the menu, double-click opens
  Options. `Ctrl+Alt+K` toggles; `Ctrl+Alt+Arrow` moves the overlay by 10 px.
- Options: monitor, position, offset, size, opacity, display mode (always / only while a layer or
  modifier is held), toggle hotkey, Latin hint, theme (follow Windows / light / dark), reserve
  screen space (maximized windows stop above the overlay), autostart (Startup-folder shortcut),
  find-a-key on/off and duration, Reload keymap.
- The keymap is read from the keyboard over Vial (USB or Bluetooth) and cached; the bundled factory
  keymap is the fallback. Layer changes arrive live from the patched firmware (feature
  `layer_notify`); on stock firmware the overlay stays on the base layer.
- Batteries: an iPhone-style pictogram on each half with the bare percentage; a white bolt while
  that half is on USB power (dual-role firmware, 2026-10-30). The USB / Bluetooth glyph sits beside
  the battery of the half that is the master. While the halves swap masters a yellow double arrow
  pulses between them and the tooltip says "switching master"; after 15 s without a reconnect the
  plain offline state returns.
- Options shows `Firmware: left v…, right v…`, each half's build id (`git describe`) as the
  keyboard reports it; `unknown` until a half has reported.
- A half that is not linked to the master is washed out with `offline` where its battery sits
  (reported by the firmware the moment the split link changes). Firmware without that bit draws
  both halves normally.

## Command-line tools (run without the tray)

| Command | Purpose |
|---|---|
| `--dump-keymap <file>` | Read the keymap and print it; also writes the postcard cache format |
| `--probe-layers [secs]` | Subscribe to layer notifications and print them |
| `--snapshot <png> [EN\|RU] [layer] [scale] [find-hid-hex] [charging=left\|right\|both] [master=right] [switching] [half-offline]` | Render a frame offline; the trailing tokens stage the battery bolt, the master side and the switching indicator |
| `--write-keymap-zmk` | Write Boris's ZMK layout, bracket combos and tap-hold rules to the keyboard (run `--dump-keymap` first to keep a copy; exit the tray helper before any HID command) |
| `--write-taphold` | Write only the tap-hold behaviour settings |

## Files

Settings `%APPDATA%\op36-helper\settings.json` (Linux: `~/.config/op36-helper/`), keymap cache `keymap.bin`, log `log.txt`
(rotated at 1 MB to `log.1.txt`), icons materialised next to them for the tray.

## Troubleshooting

- Status pill says `offline`: the Vial HID interface was not found. On USB the left half must be
  the one on the cable; on Bluetooth the keyboard must be paired and connected.
- `no layer data`: firmware without the `layer_notify` feature.
- Screen capture tools that use GDI do not show the overlay; it is a layered window. The overlay
  itself is fine.
