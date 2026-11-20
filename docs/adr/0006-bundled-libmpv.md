# ADR-0006: libmpv bundled in every installer

Status: Accepted (plan decision D11). Loader built; `tools/mpv/linux.sh` is implemented; macOS and Windows scripts are untested.

## Context

Playback correctness depends on exact mpv behaviour (options, render API, FFmpeg version). A system libmpv varies by distro and
version, may lack features, and would mean users debug a different player than the one tested.

## Decision

libmpv ships inside every installer; the user never installs mpv. Windows NSIS carries `libmpv-2.dll` in `resources/mpv/`; the macOS
universal app carries fat `libmpv.dylib` plus dependents in `Contents/Frameworks/` (fixed with `install_name_tool`); the Linux
AppImage carries a `.so` set in `usr/lib/` with `rpath=$ORIGIN/../lib`; `.deb` and `.rpm` carry a private copy in
`/usr/lib/downlink/mpv/` with no `Depends: libmpv2`. Linux libs are collected in a pinned build container (Ubuntu 22.04 glibc
baseline). Load order: bundled, then `DOWNLINK_MPV_PATH` (developer escape hatch), then the system library, logged loudly as
unsupported. Release artefacts never load a system libmpv (CLAUDE.md rule 14). CI runs each artefact on a clean machine with no mpv
and plays a fixture stream to first frame.

## Consequences

- About 30 to 40 MB per platform added. GPL-3.0 project because of the GPL mpv build (decision D4, to be confirmed).
- macOS universal needs every dependency as a fat binary (`lipo`), built on arm64 and x86_64 runners; an M1 task.
- We own libmpv security updates and must scrub `APPDIR`/`LD_LIBRARY_PATH` from child environments on Linux.

## What would change it

A licence decision that forbids GPL mpv (a permissive licence needs an LGPL build and loses mpv's X11 output), or installer size
becoming unacceptable.

## Implementation notes (2026-11)

- The runtime loader with the bundled, `DOWNLINK_MPV_PATH`, system order is implemented and unit-tested in `downlink-player`.
- M0 ran against libmpv 0.40.0 (client API 2.5, FFmpeg 7.1.5) through `DOWNLINK_MPV_PATH`. `tools/mpv/linux.sh` is implemented and was run (it collected 173 libraries plus a MANIFEST and set rpath through `patchelf` when present; pins are in `tools/mpv/pins.env`). `macos.sh` and `windows.ps1` are implemented but untested. The clean-machine CI smoke test has not run (CI is paused), so the "no mpv installed" claim is untested.
- Because the bundle pins mpv, the `vo=gpu-next` render-API limitation seen in 0.40.0 (ADR-0001) is a pinned, known quantity; revisit it when bumping the pinned build.
