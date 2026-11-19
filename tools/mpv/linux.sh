#!/usr/bin/env bash
# Stage a private libmpv set for Linux into tools/mpv/out/linux/lib.
# SKELETON. Release use: run inside the pinned Ubuntu 22.04 container (see PINNED.md).
# On a dev box it collects the host libmpv and its non-system dependencies.
set -euo pipefail
OUT="$(cd "$(dirname "$0")" && pwd)/out/linux/lib"
SRC="${LIBMPV_SRC:-/usr/lib/x86_64-linux-gnu/libmpv.so.2}"
mkdir -p "$OUT"
cp -L "$SRC" "$OUT/"
# Copy every dependency except the glibc/GPU/windowing stack that must come from the host.
EXCLUDE='^(linux-vdso|ld-linux|libc\.|libm\.|libdl\.|libpthread|librt\.|libstdc\+\+|libgcc_s|libGL|libEGL|libGLX|libGLdispatch|libX11|libxcb|libwayland|libdrm|libvulkan|libasound|libpulse|libpipewire)'
ldd "$SRC" | awk '/=> \//{print $1, $3}' | while read -r name path; do
  if ! echo "$name" | grep -Eq "$EXCLUDE"; then cp -L "$path" "$OUT/"; fi
done
command -v patchelf >/dev/null && for f in "$OUT"/*.so*; do patchelf --set-rpath '$ORIGIN' "$f"; done
echo "staged $(ls "$OUT" | wc -l) libraries in $OUT"
