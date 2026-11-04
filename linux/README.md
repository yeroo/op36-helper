# OP36 Helper on Linux

The same helper as on Windows: a click-through overlay of the Ergohaven OmegaPoint36 keymap with
the live layer, legends in the current keyboard layout (XKB, e.g. English and Russian), Shift /
Ctrl / Alt / Super variants, find-a-key, both batteries, the master half, firmware flashing and
self-update. Built and tested on Ubuntu 26.04 (GNOME on Wayland); it should run on any desktop
with X11 or XWayland, a compositor and a StatusNotifierItem tray.

## Install

From the [latest release](https://github.com/yeroo/op36-helper/releases/latest), download
`op36-helper-linux-x86_64-<version>.tar.gz`, then:

```sh
tar xf op36-helper-linux-x86_64-*.tar.gz
cd op36-helper-linux-x86_64-*/
./install.sh --udev            # binary to ~/.local/bin, menu entry, firmware; udev rule via sudo
```

`./install.sh --udev-keys` adds the optional keyboard rule (below). `./install.sh --uninstall`
removes everything except your settings.

Start **OP36 Helper** from the app menu. Options → *Start with the session* adds an autostart entry.

### Permissions

The helper needs two kinds of device access; the desktop session grants neither by default.

| What | Rule | Without it |
|---|---|---|
| The keyboard's Vial interface (`/dev/hidraw*`): keymap, layer, battery, firmware ids | `70-op36.rules` | the overlay shows the cached or factory keymap greyed out as *off* |
| Key events from every keyboard (`/dev/input/event*`) | `71-op36-keyboards.rules`, or `sudo usermod -aG input $USER` and log in again | no Shift/Ctrl/Alt/Super variants, no find-a-key, no global hotkeys |

The second one is a real privacy trade-off: any program you run can then read what you type.
Wayland only gives key events to the focused window, so the helper cannot see them any other
way. Without that rule, bind a desktop shortcut to `op36-helper --toggle` to toggle the overlay
(GNOME: Settings → Keyboard → Custom Shortcuts).

## Using it

- Tray icon (GNOME needs the AppIndicator extension, which Ubuntu ships enabled): left-click shows
  or hides the overlay, middle-click opens Options, right-click opens the menu.
- `Ctrl+Alt+K` toggles and `Ctrl+Alt+Arrows` move the overlay by 10 px. These need key access.
- Commands for the running helper: `op36-helper --toggle | --show | --hide | --options | --quit`.
- Settings `~/.config/op36-helper/settings.json`, keymap cache `keymap.bin`, log `log.txt`.

## Firmware

Same as on Windows: Options → **Flash left half…**, then double-tap that half's reset button. The
desktop mounts the bootloader drive (usually under `/media/$USER`), the helper copies the UF2,
syncs it, and the half restarts. The bundled firmware lives in `~/.local/share/op36-helper/firmware`
(or a `firmware/` folder next to the binary).

## Updates

The helper checks GitHub every 12 hours (Options can turn this off). **Update now** downloads the
Linux tarball, checks its SHA-256 against the digest GitHub publishes, replaces the binary and the
bundled firmware in place, and restarts. A copy under `/usr`, `/opt`, a snap or flatpak, or in a
directory you cannot write, is left to its package manager.

## How it works on Wayland

The overlay is an X11 window (a dock with an empty input shape, so clicks go through it) shown
through XWayland, which GNOME, KDE and most wlroots compositors start on demand. That is the only
portable way to get an always-on-top, click-through, positioned window. Native Wayland apps have
none of those on GNOME. On a fractionally scaled GNOME display, turn on
`xwayland-native-scaling` (GNOME 47+) for a crisp overlay:
`gsettings set org.gnome.mutter experimental-features "['scale-monitor-framebuffer', 'xwayland-native-scaling']"`.

The keyboard layout comes from GNOME's input sources (`org.gnome.desktop.input-sources`) and the
light/dark theme from `org.gnome.desktop.interface color-scheme`. Other desktops get US legends
and a light theme unless they keep those GNOME settings.

## Building from source

```sh
sudo apt install libxkbcommon-dev       # the only build dependency beyond Rust
git clone --recursive https://github.com/yeroo/op36-helper && cd op36-helper/helper
cargo build --release                   # target/release/op36-helper
../linux/install.sh --udev              # or run it in place
```

`linux/build-tarball.sh` builds the release tarball.
