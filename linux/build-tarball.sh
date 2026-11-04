#!/bin/sh
# Build the Linux release tarball: linux/Output/op36-helper-linux-x86_64-<version>.tar.gz holding
# op36-helper, firmware/, the udev rules, the desktop entry, the icon and install.sh.
#
#   linux/build-tarball.sh [--firmware-dir <dir>] [--skip-cargo]
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
fw_dir=$root/firmware/op36-layer-notify
skip_cargo=0
while [ $# -gt 0 ]; do
    case $1 in
        --firmware-dir) fw_dir=$2; shift ;;
        --skip-cargo) skip_cargo=1 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

version=$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$root/helper/Cargo.toml" | head -n1)
[ -n "$version" ] || { echo "no version in helper/Cargo.toml" >&2; exit 1; }

if [ "$skip_cargo" = 0 ]; then
    (cd "$root/helper" && cargo build --release --locked)
fi
bin=$root/helper/target/release/op36-helper
[ -x "$bin" ] || { echo "missing $bin" >&2; exit 1; }
for f in op36_left.uf2 op36_right.uf2 BUILD_COMMIT.txt; do
    [ -f "$fw_dir/$f" ] || { echo "missing $fw_dir/$f" >&2; exit 1; }
done

name=op36-helper-linux-x86_64-$version
out=$root/linux/Output
stage=$out/$name
rm -rf "$stage"
mkdir -p "$stage/firmware"
cp "$bin" "$stage/op36-helper" && chmod 0755 "$stage/op36-helper"
cp "$fw_dir/op36_left.uf2" "$fw_dir/op36_right.uf2" "$fw_dir/BUILD_COMMIT.txt" "$stage/firmware/"
cp "$root/linux/install.sh" "$stage/install.sh" && chmod 0755 "$stage/install.sh"
cp "$root/linux/70-op36.rules" "$root/linux/71-op36-keyboards.rules" \
   "$root/linux/op36-helper.desktop" "$root/linux/README.md" "$stage/"
cp "$root/helper/assets/icon_on_preview.png" "$stage/op36-helper.png"
cp "$root/LICENSE" "$stage/"
tar -C "$out" -czf "$out/$name.tar.gz" --owner=0 --group=0 "$name"
rm -rf "$stage"
echo "$out/$name.tar.gz"
