#!/usr/bin/env bash
# Build the pinned libmpv for Linux and stage it for bundling. Runs INSIDE the pinned Debian container
# (see linux-docker.sh, which is what CI, release and dev machines call).
#   tools/mpv/build-linux.sh <out_dir>
# Builds libplacebo + mpv from verified source against Debian's own FFmpeg/Wayland/Vulkan, then runs
# linux.sh to collect the dependency closure into <out_dir>.
set -euo pipefail

OUT="${1:?usage: $0 <out_dir>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
. "$HERE/pins.env"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
# Build tools, then mpv/libplacebo dev deps. Runtime-only host stacks (GL, Wayland, X11, VA-API, VDPAU,
# PipeWire/Pulse/ALSA) come from the user's system at run time; see linux.sh's exclude list.
apt-get install -y -qq --no-install-recommends \
  ca-certificates curl git xz-utils patchelf file build-essential pkgconf meson ninja-build python3 \
  python3-jinja2 glslang-dev libvulkan-dev liblcms2-dev libxxhash-dev libunwind-dev \
  libavcodec-dev libavformat-dev libavfilter-dev libavutil-dev libswresample-dev libswscale-dev \
  libass-dev libzimg-dev libuchardet-dev libjpeg-dev libluajit-5.1-dev \
  libva-dev libvdpau-dev libdrm-dev libdisplay-info-dev libgbm-dev libegl-dev libgl-dev \
  libwayland-dev wayland-protocols libxkbcommon-dev \
  libx11-dev libxext-dev libxrandr-dev libxpresent-dev libxss-dev libxv-dev libxinerama-dev libxfixes-dev \
  libpipewire-0.3-dev libpulse-dev libasound2-dev >/dev/null

PREFIX=/opt/mpv
SRC="$(mktemp -d)"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$PREFIX/lib/x86_64-linux-gnu/pkgconfig"

# --- libplacebo (git tag with submodules; verify the tagged commit) ---------------------------------
git -c advice.detachedHead=false clone -q --depth 1 --branch "v$LIBPLACEBO_VERSION" --recurse-submodules \
  --shallow-submodules https://code.videolan.org/videolan/libplacebo.git "$SRC/libplacebo"
got="$(git -C "$SRC/libplacebo" rev-parse HEAD)"
[ "$got" = "$LIBPLACEBO_COMMIT" ] || { echo "libplacebo commit $got != pin $LIBPLACEBO_COMMIT" >&2; exit 1; }
meson setup "$SRC/libplacebo/build" "$SRC/libplacebo" --prefix="$PREFIX" --libdir=lib --buildtype=release \
  -Dvulkan=enabled -Dopengl=enabled -Dshaderc=disabled -Dglslang=enabled -Dlcms=enabled \
  -Ddemos=false -Dtests=false >/dev/null
ninja -C "$SRC/libplacebo/build" install >/dev/null

# --- mpv (release tarball; verify SHA-256) ----------------------------------------------------------
curl -fsSL -o "$SRC/mpv.tar.gz" "https://github.com/mpv-player/mpv/archive/refs/tags/v$MPV_VERSION.tar.gz"
echo "$MPV_SRC_SHA256  $SRC/mpv.tar.gz" | sha256sum -c --quiet
mkdir "$SRC/mpv" && tar xzf "$SRC/mpv.tar.gz" -C "$SRC/mpv" --strip-components=1
meson setup "$SRC/mpv/build" "$SRC/mpv" --prefix="$PREFIX" --libdir=lib --buildtype=release \
  -Dlibmpv=true -Dcplayer=false -Dlua=luajit -Djavascript=disabled \
  -Dmanpage-build=disabled -Dhtml-build=disabled -Dpdf-build=disabled \
  -Dwayland=enabled -Dx11=enabled -Degl=enabled -Dgl=enabled -Dvulkan=enabled \
  -Dvaapi=enabled -Dvdpau=enabled -Ddrm=enabled -Dlcms2=enabled -Dzimg=enabled \
  -Dpipewire=enabled -Dpulse=enabled -Dalsa=enabled >/dev/null
ninja -C "$SRC/mpv/build" install >/dev/null

# --- stage: libmpv + its non-system closure, rpath $ORIGIN ------------------------------------------
LD_LIBRARY_PATH="$PREFIX/lib" LIBMPV_SRC="$PREFIX/lib/libmpv.so.2" "$HERE/linux.sh" "$OUT"
