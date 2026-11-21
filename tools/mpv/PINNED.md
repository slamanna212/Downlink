# Pinned libmpv builds

libmpv ships inside every installer; nothing here is a system dependency. Every platform ships the **same
mpv release**. Pins live in `tools/mpv/pins.env` (read by the scripts); this file explains them. Change a pin
by editing `pins.env` (and this file if the source changes) in one commit, with the reason.

| Platform | mpv | Source | Staged by | Bundled to |
|---|---|---|---|---|
| Windows x86_64 | 0.42.0 | mpv's official `libmpv-v0.42.0-x86_64-w64-mingw32-gpl.zip` (GitHub release asset), SHA-256 in `pins.env`. Only `libmpv-2.dll` and `LICENSE.GPL` are kept. | `windows.ps1 -OutDir <dir>` | `resources/mpv/` |
| Linux x86_64 | 0.42.0 | Built from source in the pinned `debian:13@sha256:…` image: libplacebo 7.387.0 (git tag, commit verified) + mpv 0.42.0 (release tarball, SHA-256 verified) against Debian's FFmpeg 7.1 (Vulkan, VA-API, VDPAU, NVDEC). LuaJIT is on, like the official Windows build, so both accept the same options (`osc`, `ytdl`; scripts stay off via `load-scripts=no`). | `build-linux.sh <dir>` inside the image; `linux-docker.sh [dir]` runs it there from any Docker host | AppImage `usr/lib/`; deb `/usr/lib/downlink/mpv/` |
| macOS | — | Not built yet. Will build the same pinned mpv from source (no Homebrew libmpv) when a macOS job exists. | — | `Contents/Frameworks/` |

## Linux

- **Floor:** glibc 2.41 and Wayland 1.23 (Debian 13, Ubuntu 25.04, Fedora 42 and newer). Ubuntu 24.04 is not
  supported: mpv 0.42 needs `wl_display_create_queue_with_name` (Wayland 1.23) and the bundle is linked
  `BIND_NOW`, so it would not load there.
- **One build everywhere.** CI's Rust job and the Linux release job run *in* the pinned image and restore the
  same cached build (`.github/actions/libmpv-linux`, key = hash of the pins and scripts). Dev machines run
  `tools/mpv/linux-docker.sh` and set `DOWNLINK_MPV_PATH=$PWD/tools/mpv/out/linux/lib/libmpv.so.2`.
- `linux.sh` collects libmpv's non-system closure with `rpath=$ORIGIN` (needs `patchelf`) and writes
  `MANIFEST.txt` (versions + SHA-256). It excludes glibc and the host-bound stacks (GL/EGL, X11/xcb, Wayland,
  DRM/gbm, Vulkan, VA-API/VDPAU, ALSA/Pulse/PipeWire/JACK, systemd/dbus/udev, fontconfig/freetype), so GPU and
  audio come from the user's machine.
- CI runs the libmpv tests with `DOWNLINK_REQUIRE_MPV=1`: they fail, not skip, if the bundle cannot load.

## Rules

- Never load a system libmpv in a release artefact (`DOWNLINK_MPV_PATH` is a dev escape hatch).
- Staged output goes to `tools/mpv/out/<platform>/` (gitignored); the release config copies it into the bundle.

## Host libraries

The excluded stacks must exist on the user's machine. The `.deb` declares them
(`apps/desktop/tauri.release.linux.conf.json`); CI's container installs the two a bare image lacks
(`libpipewire-0.3-0t64`, `libva-wayland2`). Regenerate the list after a pin change by mapping every `NEEDED`
entry in the bundle that is not itself bundled to its package (`dpkg -S`).
