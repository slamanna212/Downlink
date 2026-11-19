#!/usr/bin/env bash
# UNTESTED on macOS. Collect libmpv and its non-system dylib tree, and build a universal set.
# Usage:
#   tools/mpv/macos.sh <out_dir>            # collect for this runner's arch into <out_dir>/<arch>
#   tools/mpv/macos.sh <out_dir> merge      # lipo <out_dir>/arm64 + <out_dir>/x86_64 -> <out_dir>/universal
# Run `collect` on an arm64 runner and an x86_64 runner (copy both trees under <out_dir>), then `merge`.
set -euo pipefail
OUT="${1:?usage: $0 <out_dir> [merge]}"
MODE="${2:-collect}"
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
. "$HERE/pins.env"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

is_system() { case "$1" in /usr/lib/*|/System/*) return 0 ;; *) return 1 ;; esac; }

collect() {
  local arch dest
  arch="$(uname -m)"; [ "$arch" = arm64 ] || arch=x86_64
  dest="$OUT/$arch"; mkdir -p "$dest"
  brew install mpv
  local v; v="$(brew list --versions mpv | awk '{print $2}')"
  echo "mpv formula $v (pinned: $MPV_MAC_FORMULA_VERSION)"
  if [ "$MPV_MAC_FORMULA_VERSION" != TBD ] && [ "$v" != "$MPV_MAC_FORMULA_VERSION" ]; then
    echo "warning: mpv formula $v differs from the pin" >&2
  fi
  local seen="$dest/.seen"; : > "$seen"
  local queue=("$(brew --prefix mpv)/lib/libmpv.2.dylib")
  while [ "${#queue[@]}" -gt 0 ]; do
    local lib="${queue[0]}"; queue=("${queue[@]:1}")
    local real; real="$(python3 -c 'import os,sys;print(os.path.realpath(sys.argv[1]))' "$lib")"
    local name; name="$(basename "$lib")"
    grep -qx "$name" "$seen" && continue
    echo "$name" >> "$seen"
    cp -L "$real" "$dest/$name"; chmod u+w "$dest/$name"
    while read -r dep; do
      is_system "$dep" && continue
      case "$dep" in @rpath/*|@loader_path/*|@executable_path/*) continue ;; esac
      queue+=("$dep")
    done < <(otool -L "$real" | tail -n +2 | awk '{print $1}')
  done
  # Rewrite install names so everything resolves next to the loader (Contents/Frameworks).
  for f in "$dest"/*.dylib; do
    install_name_tool -id "@rpath/$(basename "$f")" "$f"
    while read -r dep; do
      is_system "$dep" && continue
      case "$dep" in @rpath/*|@loader_path/*|@executable_path/*) continue ;; esac
      install_name_tool -change "$dep" "@loader_path/$(basename "$dep")" "$f"
    done < <(otool -L "$f" | tail -n +2 | awk '{print $1}')
    codesign --force -s - "$f"
  done
  rm -f "$seen"
  echo "collected $(ls "$dest" | wc -l) dylibs for $arch in $dest"
}

merge() {
  local dest="$OUT/universal"; mkdir -p "$dest"
  for f in "$OUT"/arm64/*.dylib; do
    local n; n="$(basename "$f")"
    lipo -create "$OUT/arm64/$n" "$OUT/x86_64/$n" -output "$dest/$n"
    codesign --force -s - "$dest/$n"
  done
  echo "universal set in $dest"
}

case "$MODE" in collect) collect ;; merge) merge ;; *) echo "unknown mode $MODE" >&2; exit 2 ;; esac
