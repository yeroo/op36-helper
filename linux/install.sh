#!/bin/sh
# OP36 Helper for Linux: per-user install from the release tarball (or a source checkout after
# `cargo build --release`). No root needed except for the optional udev rules.
#
#   ./install.sh               binary, firmware, icon and menu entry under ~/.local
#   ./install.sh --udev        also install the keyboard access rule (asks for sudo)
#   ./install.sh --udev-keys   also let the helper read all keyboards (modifiers, find-a-key, hotkeys)
#   ./install.sh --uninstall   remove what the install put under ~/.local (settings are kept)
set -eu

here=$(cd "$(dirname "$0")" && pwd)
bin_dir=${XDG_BIN_HOME:-$HOME/.local/bin}
data_dir=${XDG_DATA_HOME:-$HOME/.local/share}
app_data=$data_dir/op36-helper
udev=0
udev_keys=0
uninstall=0

for arg in "$@"; do
    case $arg in
        --udev) udev=1 ;;
        --udev-keys) udev=1; udev_keys=1 ;;
        --uninstall) uninstall=1 ;;
        -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

if [ "$uninstall" = 1 ]; then
    "$bin_dir/op36-helper" --quit 2>/dev/null || true
    rm -f "$bin_dir/op36-helper" \
          "$data_dir/applications/op36-helper.desktop" \
          "$data_dir/icons/hicolor/256x256/apps/op36-helper.png" \
          "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/op36-helper.desktop"
    rm -rf "$app_data/firmware" "$app_data/updates"
    echo "Removed. Settings stay in ${XDG_CONFIG_HOME:-$HOME/.config}/op36-helper;"
    echo "udev rules (if installed): sudo rm /etc/udev/rules.d/70-op36.rules /etc/udev/rules.d/71-op36-keyboards.rules"
    exit 0
fi

# The binary: tarball layout first, then a source checkout's release build.
if [ -f "$here/op36-helper" ]; then
    src_bin=$here/op36-helper
elif [ -f "$here/../helper/target/release/op36-helper" ]; then
    src_bin=$here/../helper/target/release/op36-helper
else
    echo "op36-helper binary not found (run 'cargo build --release' in helper/ first)" >&2
    exit 1
fi

"$bin_dir/op36-helper" --quit 2>/dev/null && sleep 1 || true
mkdir -p "$bin_dir" "$app_data" "$data_dir/applications" "$data_dir/icons/hicolor/256x256/apps"
cp "$src_bin" "$bin_dir/op36-helper.new"
chmod 0755 "$bin_dir/op36-helper.new"
mv -f "$bin_dir/op36-helper.new" "$bin_dir/op36-helper"

for fw in "$here/firmware" "$here/../firmware/op36-layer-notify"; do
    if [ -f "$fw/op36_left.uf2" ] && [ -f "$fw/op36_right.uf2" ] && [ -f "$fw/BUILD_COMMIT.txt" ]; then
        mkdir -p "$app_data/firmware"
        cp -f "$fw/op36_left.uf2" "$fw/op36_right.uf2" "$fw/BUILD_COMMIT.txt" "$app_data/firmware/"
        echo "firmware $(cat "$fw/BUILD_COMMIT.txt") -> $app_data/firmware"
        break
    fi
done

for icon in "$here/op36-helper.png" "$here/../helper/assets/icon_on_preview.png"; do
    if [ -f "$icon" ]; then
        cp -f "$icon" "$data_dir/icons/hicolor/256x256/apps/op36-helper.png"
        break
    fi
done

for desktop in "$here/op36-helper.desktop" "$here/../linux/op36-helper.desktop"; do
    if [ -f "$desktop" ]; then
        sed "s|^Exec=op36-helper|Exec=$bin_dir/op36-helper|" "$desktop" \
            > "$data_dir/applications/op36-helper.desktop"
        break
    fi
done
command -v update-desktop-database >/dev/null && update-desktop-database "$data_dir/applications" 2>/dev/null || true
# An autostart entry written by an older copy keeps pointing at the right binary.
autostart=${XDG_CONFIG_HOME:-$HOME/.config}/autostart/op36-helper.desktop
[ -f "$autostart" ] && sed -i "s|^Exec=.*|Exec=\"$bin_dir/op36-helper\"|" "$autostart"

if [ "$udev" = 1 ]; then
    rules_dir=$here
    [ -f "$rules_dir/70-op36.rules" ] || rules_dir=$here/../linux
    sudo cp "$rules_dir/70-op36.rules" /etc/udev/rules.d/70-op36.rules
    if [ "$udev_keys" = 1 ]; then
        sudo cp "$rules_dir/71-op36-keyboards.rules" /etc/udev/rules.d/71-op36-keyboards.rules
    fi
    sudo chmod 0644 /etc/udev/rules.d/70-op36.rules /etc/udev/rules.d/71-op36-keyboards.rules 2>/dev/null || true
    sudo udevadm control --reload
    sudo udevadm trigger
    echo "udev rules installed"
fi

echo "Installed $bin_dir/op36-helper. Start it from the app menu (OP36 Helper) or run: op36-helper &"
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "note: $bin_dir is not on PATH in this shell" ;;
esac
