#!/usr/bin/env bash
# Create the unversioned "latest download" copies (PLAN §10.1) from versioned assets.
# Usage: unversioned-copies.sh ASSET_DIR
# Set SKIP_MACOS=1 when macOS builds are disabled (no dmg).
# Writes the copies next to the originals (same directory) and prints their names.
set -euo pipefail
DIR="${1:?usage: $0 ASSET_DIR}"
cd "$DIR"

copy() { # <glob> <unversioned name>
  local src
  src="$(find . -maxdepth 1 -type f -name "$1" ! -name '*.sig' | sort | head -n1)"
  if [ -z "$src" ]; then echo "no asset matches $1" >&2; exit 1; fi
  cp "$src" "$2"
  echo "$2"
}

copy '*_x64-setup.exe'   Downlink_x64-setup.exe
[ "${SKIP_MACOS:-}" = 1 ] || copy '*_universal.dmg'   Downlink_universal.dmg
copy '*_amd64.AppImage'  Downlink_amd64.AppImage
copy '*_amd64.deb'       Downlink_amd64.deb
copy '*.x86_64.rpm'      Downlink-x86_64.rpm
