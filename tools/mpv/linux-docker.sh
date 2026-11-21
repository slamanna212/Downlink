#!/usr/bin/env bash
# Build + stage the pinned Linux libmpv in the pinned container. Used by CI, release and dev machines.
#   tools/mpv/linux-docker.sh [out_dir]     (default tools/mpv/out/linux/lib)
# Dev: export DOWNLINK_MPV_PATH="$PWD/tools/mpv/out/linux/lib/libmpv.so.2"
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
. "$HERE/pins.env"
OUT="${1:-$HERE/out/linux/lib}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
docker run --rm -v "$HERE:/mpv:ro" -v "$OUT:/out" "$MPV_LINUX_IMAGE" \
  bash -c "/mpv/build-linux.sh /out && chown -R $(id -u):$(id -g) /out"
