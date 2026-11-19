#!/usr/bin/env bash
# UNTESTED on macOS. Build a universal libmpv + dylib tree in tools/mpv/out/macos/Frameworks.
# Run on an arm64 runner and on an x86_64 runner (ARCH_DIR=arm64|x86_64), then `./macos.sh merge`.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)/out/macos"
case "${1:-collect}" in
  collect)
    ARCH_DIR="${ARCH_DIR:?set ARCH_DIR=arm64 or x86_64}"
    DEST="$ROOT/$ARCH_DIR"; mkdir -p "$DEST"
    brew install mpv   # TODO: pin formula version / bottle SHA-256 per PINNED.md
    LIB="$(brew --prefix mpv)/lib/libmpv.2.dylib"
    cp -L "$LIB" "$DEST/"
    # TODO: walk `otool -L` recursively, copy every non-system dylib, rewrite install names:
    #   install_name_tool -id @rpath/<name> and -change <old> @loader_path/<name>
    ;;
  merge)
    mkdir -p "$ROOT/Frameworks"
    for f in "$ROOT"/arm64/*.dylib; do
      n="$(basename "$f")"
      lipo -create "$ROOT/arm64/$n" "$ROOT/x86_64/$n" -output "$ROOT/Frameworks/$n"
    done
    ;;
  *) echo "usage: ARCH_DIR=arm64|x86_64 $0 collect | $0 merge" >&2; exit 2 ;;
esac
