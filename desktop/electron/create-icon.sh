#!/bin/bash
# Resize the complete dove artwork into the sizes of a macOS icon set, and make electron/icon.icns of it.
# This script only writes electron/icon.icns inside this checkout, and runs on macOS, with sips and iconutil.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
artwork="$here/../public/columbina-icon.png"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
set="$work/columbina.iconset"
mkdir "$set"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$artwork" --out "$set/icon_${size}x${size}.png" > /dev/null
  sips -z "$((size * 2))" "$((size * 2))" "$artwork" --out "$set/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns "$set" -o "$here/icon.icns"
echo "$here/icon.icns"
