#!/usr/bin/env bash
# Build the pinned libmpv for macOS (Apple Silicon) and stage it for bundling (PLAN §4.10).
#   tools/mpv/macos.sh <out_dir>
# Runs natively on an arm64 Mac (CI: the macos-15 runner). Needs the Xcode command line tools plus meson, ninja
# and pkgconf (Homebrew is fine: build tools only, never libraries).
#
# Every dependency (FFmpeg, libass with FreeType/FriBidi/HarfBuzz, libplacebo) is built from verified source as a
# static library and linked into ONE self-contained libmpv.2.dylib that references nothing but the OS
# (/usr/lib, /System/Library). The script fails if anything else leaks in, or if the minimum macOS is wrong.
#
# Output:
#   <out_dir>/Frameworks/libmpv.2.dylib   unsigned; the release job signs it (Contents/Frameworks may hold code only)
#   <out_dir>/licenses/                   MANIFEST.txt + licences (Contents/Resources/mpv)
set -euo pipefail

OUT="${1:?usage: $0 <out_dir>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
. "$HERE/pins.env"

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo "macos.sh builds the Apple Silicon libmpv and must run on an arm64 Mac" >&2
  exit 1
fi
for tool in meson ninja pkg-config curl shasum git; do
  command -v "$tool" >/dev/null || { echo "missing build tool: $tool" >&2; exit 1; }
done

export MACOSX_DEPLOYMENT_TARGET="$MPV_MACOS_MIN"
export CFLAGS="-mmacosx-version-min=$MPV_MACOS_MIN -O2"
export CXXFLAGS="$CFLAGS"
export OBJCFLAGS="$CFLAGS"
export LDFLAGS="-mmacosx-version-min=$MPV_MACOS_MIN"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
SRC="$WORK/src"
PREFIX="$WORK/prefix"
mkdir -p "$SRC" "$PREFIX/lib/pkgconfig"

# Only our own prefix is visible to pkg-config, so nothing from Homebrew can be picked up. --static makes every
# .pc file hand over its private dependencies, which a static link into libmpv needs.
unset PKG_CONFIG_PATH
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
cat > "$WORK/native.ini" <<EOF
[binaries]
pkg-config = ['pkg-config', '--static']
EOF

fetch() { # fetch <url> <sha256> <dest_dir>: download, verify, unpack
  local url="$1" sum="$2" dest="$3" file
  file="$SRC/$(basename "$url")"
  curl -fsSL --retry 3 -o "$file" "$url"
  echo "$sum  $file" | shasum -a 256 -c --quiet
  mkdir -p "$dest"
  tar xf "$file" -C "$dest" --strip-components=1
}

meson_build() { # meson_build <src_dir> <static|shared> [meson -D options...]
  # meson and ninja report errors on stdout, so output goes to a log that is dumped when a step fails.
  local dir="$1" kind="$2" log
  shift 2
  log="$WORK/$(basename "$dir").log"
  echo "building $(basename "$dir")"
  if ! meson setup "$dir/build" "$dir" --prefix="$PREFIX" --libdir=lib --buildtype=release \
    --default-library="$kind" --native-file "$WORK/native.ini" "$@" >"$log" 2>&1; then
    echo "meson setup failed for $(basename "$dir"):" >&2
    tail -n 80 "$log" >&2
    [ -f "$dir/build/meson-logs/meson-log.txt" ] && tail -n 80 "$dir/build/meson-logs/meson-log.txt" >&2
    exit 1
  fi
  if ! ninja -C "$dir/build" install >"$log" 2>&1; then
    echo "ninja failed for $(basename "$dir"):" >&2
    grep -E -B2 -A12 'FAILED:|error:' "$log" | head -n 200 >&2 || tail -n 120 "$log" >&2
    exit 1
  fi
}

# --- FreeType, FriBidi, HarfBuzz, libass (subtitles; CoreText font provider) --------------------------------
fetch "https://download.savannah.gnu.org/releases/freetype/freetype-$FREETYPE_VERSION.tar.xz" \
  "$FREETYPE_SHA256" "$SRC/freetype"
meson_build "$SRC/freetype" static \
  -Dzlib=internal -Dbzip2=disabled -Dpng=disabled -Dbrotli=disabled -Dharfbuzz=disabled -Dtests=disabled

fetch "https://github.com/fribidi/fribidi/releases/download/v$FRIBIDI_VERSION/fribidi-$FRIBIDI_VERSION.tar.xz" \
  "$FRIBIDI_SHA256" "$SRC/fribidi"
meson_build "$SRC/fribidi" static -Ddocs=false -Dbin=false -Dtests=false

fetch "https://github.com/harfbuzz/harfbuzz/releases/download/$HARFBUZZ_VERSION/harfbuzz-$HARFBUZZ_VERSION.tar.xz" \
  "$HARFBUZZ_SHA256" "$SRC/harfbuzz"
meson_build "$SRC/harfbuzz" static \
  -Dfreetype=enabled -Dglib=disabled -Dgobject=disabled -Dcairo=disabled -Dchafa=disabled -Dpng=disabled \
  -Dzlib=disabled -Dicu=disabled -Dcoretext=disabled -Draster=disabled -Dvector=disabled -Dgpu=disabled \
  -Dgpu_demo=disabled -Dsubset=disabled -Dtests=disabled -Dintrospection=disabled -Ddocs=disabled \
  -Dutilities=disabled -Dbenchmark=disabled

fetch "https://github.com/libass/libass/releases/download/$LIBASS_VERSION/libass-$LIBASS_VERSION.tar.xz" \
  "$LIBASS_SHA256" "$SRC/libass"
meson_build "$SRC/libass" static \
  -Dfontconfig=disabled -Dcoretext=enabled -Ddirectwrite=disabled -Dlibunibreak=disabled -Dasm=disabled \
  -Dtest=disabled -Dcompare=disabled -Dprofile=disabled -Dfuzz=disabled -Dcheckasm=disabled

# --- FFmpeg (static; VideoToolbox decode, SecureTransport TLS, nothing autodetected) -------------------------
# --disable-autodetect keeps the runner's Homebrew libraries out; everything we want is enabled by name.
# pthreads is in FFmpeg's autodetect list, so it must be named too. avdevice stays on: mpv's av://lavfi (the
# app's --self-test source) needs the lavfi input device.
fetch "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz" "$FFMPEG_SHA256" "$SRC/ffmpeg"
(
  cd "$SRC/ffmpeg"
  ./configure --prefix="$PREFIX" --cc=clang --enable-static --disable-shared --enable-pic --enable-gpl \
    --disable-programs --disable-doc --disable-debug --disable-autodetect \
    --enable-pthreads --enable-zlib --enable-bzlib --enable-iconv \
    --enable-videotoolbox --enable-audiotoolbox --enable-securetransport \
    --pkg-config-flags=--static \
    --extra-cflags="$CFLAGS" --extra-ldflags="$LDFLAGS" >/dev/null
  make -j"$(sysctl -n hw.ncpu)" >/dev/null
  make install >/dev/null
)

# --- libplacebo (OpenGL only; git tag with submodules, tagged commit verified) -------------------------------
git -c advice.detachedHead=false clone -q --depth 1 --branch "v$LIBPLACEBO_VERSION" --recurse-submodules \
  --shallow-submodules https://code.videolan.org/videolan/libplacebo.git "$SRC/libplacebo"
got="$(git -C "$SRC/libplacebo" rev-parse HEAD)"
[ "$got" = "$LIBPLACEBO_COMMIT" ] || { echo "libplacebo commit $got != pin $LIBPLACEBO_COMMIT" >&2; exit 1; }
meson_build "$SRC/libplacebo" static \
  -Dvulkan=disabled -Dopengl=enabled -Dd3d11=disabled -Dglslang=disabled -Dshaderc=disabled -Dlcms=disabled \
  -Dlibdovi=disabled -Dunwind=disabled -Dxxhash=disabled -Ddemos=false -Dtests=false -Dbench=false -Dfuzz=false

# --- mpv (release tarball, SHA-256 verified; libmpv only, no Swift, no Lua) ----------------------------------
# Lua is off: base_options' `osc`/`ytdl` are then unknown and logged as rejected, which is harmless (the
# engine never fails on a rejected option) and means no scripting runtime ships at all.
# -lc++: HarfBuzz and libplacebo contain C++ and are linked statically into a C library.
fetch "https://github.com/mpv-player/mpv/archive/refs/tags/v$MPV_VERSION.tar.gz" "$MPV_SRC_SHA256" "$SRC/mpv"
# Without Swift, mpv 0.42 still compiles its Swift-only clipboard backend; see the patch header.
patch -p1 -d "$SRC/mpv" --forward --quiet < "$HERE/patches/mpv-macos-clipboard-without-swift.patch"
meson_build "$SRC/mpv" shared \
  -Dlibmpv=true -Dcplayer=false -Dgpl=true -Dtests=false -Dbuild-date=false -Dc_link_args="$LDFLAGS -lc++" \
  -Dcocoa=enabled -Dcoreaudio=enabled -Dgl=enabled -Dgl-cocoa=enabled -Dplain-gl=enabled \
  -Dvideotoolbox-gl=enabled -Dlibavdevice=enabled -Diconv=enabled -Dzlib=enabled \
  -Dswift-build=disabled -Dmacos-cocoa-cb=disabled -Dmacos-media-player=disabled -Dmacos-touchbar=disabled \
  -Dvulkan=disabled -Dvideotoolbox-pl=disabled -Davfoundation=disabled -Daudiounit=disabled \
  -Dlua=disabled -Djavascript=disabled -Dcplugins=disabled -Dlcms2=disabled -Dzimg=disabled \
  -Djpeg=disabled -Duchardet=disabled -Drubberband=disabled -Dvapoursynth=disabled -Dsubrandr=disabled \
  -Dlibarchive=disabled -Dlibbluray=disabled -Ddvdnav=disabled -Dcdda=disabled -Ddvbin=disabled \
  -Ddvda=disabled -Dlibcurl=disabled -Dcaca=disabled -Dsixel=disabled -Dshaderc=disabled \
  -Dspirv-cross=disabled -Dsdl2-audio=disabled -Dsdl2-video=disabled -Dsdl2-gamepad=disabled \
  -Dopenal=disabled -Djack=disabled -Dpulse=disabled -Dpipewire=disabled -Dsndio=disabled \
  -Dmanpage-build=disabled -Dhtml-build=disabled -Dpdf-build=disabled

# --- stage + verify --------------------------------------------------------------------------------------------
rm -rf "$OUT"
mkdir -p "$OUT/Frameworks" "$OUT/licenses"
OUT="$(cd "$OUT" && pwd)"
LIB="$OUT/Frameworks/libmpv.2.dylib"
cp -L "$PREFIX/lib/libmpv.2.dylib" "$LIB"
chmod 755 "$LIB"
install_name_tool -id "@rpath/libmpv.2.dylib" "$LIB"
strip -x "$LIB"

fail=0
archs="$(lipo -archs "$LIB")"
[ "$archs" = arm64 ] || { echo "libmpv architectures: '$archs' (want arm64)" >&2; fail=1; }
minos="$(otool -l "$LIB" | awk '/LC_BUILD_VERSION/{f=1} f&&$1=="minos"{print $2; exit}')"
[ "$minos" = "$MPV_MACOS_MIN" ] || { echo "libmpv minos $minos != $MPV_MACOS_MIN" >&2; fail=1; }
# Dependencies: the first otool line is the dylib itself (its id), the rest must be OS libraries.
leaks="$(otool -L "$LIB" | tail -n +3 | awk '{print $1}' | grep -Ev '^(/usr/lib/|/System/Library/)' || true)"
if [ -n "$leaks" ]; then
  echo "libmpv links non-system libraries:" >&2
  echo "$leaks" >&2
  fail=1
fi

cp "$SRC/mpv/LICENSE.GPL" "$OUT/licenses/LICENSE.GPL"
cp "$SRC/mpv/Copyright" "$OUT/licenses/mpv-Copyright"
{
  echo "# libmpv bundle, generated by tools/mpv/macos.sh on $(date -u +%FT%TZ)"
  echo "arch: $archs, minimum macOS: $minos, $(xcrun clang --version | head -1)"
  echo "mpv $MPV_VERSION, FFmpeg $FFMPEG_VERSION, libplacebo $LIBPLACEBO_VERSION ($LIBPLACEBO_COMMIT)"
  echo "libass $LIBASS_VERSION, FreeType $FREETYPE_VERSION, FriBidi $FRIBIDI_VERSION, HarfBuzz $HARFBUZZ_VERSION"
  echo "links:"
  otool -L "$LIB" | tail -n +3 | awk '{print "  " $1}'
  (cd "$OUT/Frameworks" && shasum -a 256 libmpv.2.dylib)
} > "$OUT/licenses/MANIFEST.txt"
cat "$OUT/licenses/MANIFEST.txt"

[ "$fail" = 0 ] || exit 1
echo "staged $LIB ($(du -h "$LIB" | cut -f1))"
