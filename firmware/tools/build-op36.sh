#!/usr/bin/env bash
# Build both OP36 halves (central = left, peripheral = right) from the local rmk clone and
# convert them to UF2. Run from the omega36 repo root:  bash firmware/tools/build-op36.sh [--ci]
#
# Local prerequisites (verified 2026-10-28, see docs/superpowers/plans/2026-10-28-firmware-layer-notify.md Task 5):
#   scoop install gcc-arm-none-eabi ; cargo install flip-link ; the nrf-sdc checkout's
#   nrf-sdc-sys/third_party symlink replaced by a copy of nrf-mpsl-sys/third_party.
# uf2conv.py  SHA256 ad36ba2d61fb2ea371832262392088281eee474c609df4142adbee4ea3c20f26
# uf2families.json SHA256 9d8f561e050507c5909063005cd6f5b157e6fe6db1e8d25b5d9027d57064c194
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/firmware/op36-layer-notify"
KBD="$ROOT/rmk/keyboards/op36"
mkdir -p "$OUT"

if [[ "${1:-}" == "--ci" ]]; then
  # Fallback: let the vendor workflow in the fork build it.
  git -C "$ROOT/rmk" push -u origin op36-layer-notify
  gh workflow run build.yml --repo yeroo/rmk --ref op36-layer-notify
  echo "Started. Download the 'op36' artifact with: gh run download --repo yeroo/rmk -n op36 -D \"$OUT\""
  exit 0
fi

: "${ARM_SYSROOT:=C:\Users\boris_kudriashov\scoop\apps\gcc-arm-none-eabi\current\arm-none-eabi}"
SYSROOT="$ARM_SYSROOT"
: "${ARM_GCC_INCLUDE:=C:\Users\boris_kudriashov\scoop\apps\gcc-arm-none-eabi\current\lib\gcc\arm-none-eabi\15.2.1\include}"
GCCINC="$ARM_GCC_INCLUDE"
export BINDGEN_EXTRA_CLANG_ARGS="--target=arm-none-eabi --sysroot=$SYSROOT -I$SYSROOT\include -I$GCCINC"
# Reuse the dependency build cache of the reference clone when present.
[[ -d "$ROOT/_ref/rmk/keyboards/op36/target" ]] && export CARGO_TARGET_DIR="$ROOT/_ref/rmk/keyboards/op36/target"
TARGET_DIR="${CARGO_TARGET_DIR:-$KBD/target}"

if ! git -C "$ROOT/rmk" diff --quiet HEAD -- rmk keyboards/op36; then
  echo "rmk tree has uncommitted firmware changes; commit them first so BUILD_COMMIT.txt is honest" >&2
  exit 1
fi
( cd "$KBD" && cargo build --release --bin central --bin peripheral )
llvm-objcopy -O binary "$TARGET_DIR/thumbv7em-none-eabihf/release/central" "$OUT/left.bin"
python -I "$ROOT/firmware/tools/uf2conv.py" "$OUT/left.bin" --base 0x26000 --family 0xADA52840 --output "$OUT/op36_left.uf2"
llvm-objcopy -O binary "$TARGET_DIR/thumbv7em-none-eabihf/release/peripheral" "$OUT/right.bin"
python -I "$ROOT/firmware/tools/uf2conv.py" "$OUT/right.bin" --base 0x26000 --family 0xADA52840 --output "$OUT/op36_right.uf2"
rm -f "$OUT/left.bin" "$OUT/right.bin"
git -C "$ROOT/rmk" describe --tags --always --dirty > "$OUT/BUILD_COMMIT.txt"
( cd "$OUT" && sha256sum op36_left.uf2 op36_right.uf2 > SHA256SUMS.txt )
ls -la "$OUT"
