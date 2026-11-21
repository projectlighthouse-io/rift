#!/usr/bin/env bash
# Renders crates/app/assets/icon/rift.svg to PNGs and, on macOS, to rift.icns.
set -euo pipefail
cd "$(dirname "$0")/.."
icon=crates/app/assets/icon
cargo test -p rift --quiet -- --ignored render_icon
if command -v iconutil >/dev/null; then
  set=$(mktemp -d)/rift.iconset
  mkdir -p "$set"
  for s in 16 32 128 256 512; do
    cp "$icon/rift-$s.png" "$set/icon_${s}x${s}.png"
    cp "$icon/rift-$((s * 2)).png" "$set/icon_${s}x${s}@2x.png" 2>/dev/null || true
  done
  cp "$icon/rift-64.png" "$set/icon_32x32@2x.png"
  iconutil -c icns "$set" -o "$icon/rift.icns"
fi
