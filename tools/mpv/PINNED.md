# Pinned libmpv builds (PLAN §4.10, ADR-0006)

libmpv ships inside every installer. Nothing here is a system dependency. Update a pin by editing
this file and the matching script in the same commit; record the reason.

Pins live in `tools/mpv/pins.env` (read by the scripts); this file explains them.

| Platform | Source | Pin | Staged by | Staged to | Status |
|---|---|---|---|---|---|
| Windows x86_64 | zhongfly GPL `mpv-dev-x86_64` build (`libmpv-2.dll`) | tag `2026-11-18-f92afa75cd`, asset `mpv-dev-x86_64-20261118-git-f92afa75cd.7z`, SHA-256 `f8e087dae56f8bfc5079142ef51ac6a0b8b328a8dea590f6a53cbfb13d1e070e` (download and hash verified from Linux on 2026-11-19) | `windows.ps1 -OutDir <dir>` | `resources/mpv/` | script untested on Windows |
| macOS universal | Homebrew `mpv` bottle per arch, dylib tree re-rooted with `install_name_tool`, merged with `lipo` | formula version **TBD** (recorded by the first CI run into `pins.env`) | `macos.sh <dir>` on arm64 and x86_64 runners, then `macos.sh <dir> merge` | `Contents/Frameworks/` | script untested on macOS |
| Linux x86_64 | `libmpv2` from the pinned `ubuntu:22.04` container plus its non-glibc dependency closure | image digest **TBD**, package version **TBD** (`pins.env`; the script warns on mismatch) | `linux.sh <dir>` | AppImage `usr/lib/`; deb/rpm `/usr/lib/downlink/mpv/` | script run here on Debian 13 (173 libraries, 208 MB uncompressed, loads and plays under `--self-test`); trimming the closure is a release task |

`linux.sh` excludes glibc and the host-bound stacks (GL/EGL, X11/xcb, Wayland, DRM/gbm, Vulkan, VA-API/VDPAU, ALSA/Pulse/PipeWire/JACK,
systemd/dbus/udev, fontconfig/freetype) so GPU and audio come from the user's machine; everything else is bundled with `rpath=$ORIGIN`
(needs `patchelf`). It writes `MANIFEST.txt` with SHA-256 sums.

## Dev machine (what M0 ran against)

- Debian 13, system `libmpv.so.2`, mpv 0.40.0 (client API 2.5), FFmpeg 7.1.5. Loaded through
  `DOWNLINK_MPV_PATH` (unsupported in releases).

## CI smoke test

`downlink-desktop --self-test` (with `DOWNLINK_REQUIRE_BUNDLED=1`, and `DOWNLINK_MPV_DIR` if the engine is not next to the
executable) loads the bundled engine, plays a synthetic lavfi source with `vo=null` for 2 s and prints `OK`/`FAIL`. Run it on a clean VM
with no mpv installed.

## Rules

- Never fork mpv; never load a system libmpv in a release artefact.
- Staged output goes to `tools/mpv/out/<platform>/` (gitignored); the release workflow copies it
  into the bundle.
- A CI job runs each artefact on a clean VM with no mpv installed and plays a fixture to first
  frame (PLAN §4.10).
