# Pinned libmpv builds (PLAN §4.10, ADR-0006)

libmpv ships inside every installer. Nothing here is a system dependency. Update a pin by editing
this file and the matching script in the same commit; record the reason.

| Platform | Source | Pin | Staged to | Status |
|---|---|---|---|---|
| Windows x86_64 | zhongfly GPL `mpv-dev-x86_64` build (`libmpv-2.dll`) | **TBD tag** (choose the newest release that passes M0, record the tag and SHA-256 here) | `resources/mpv/` | script untested |
| macOS universal | Homebrew bottles (arm64 + x86_64) merged with `lipo`, dylib tree fixed with `install_name_tool` | **TBD** formula version + bottle SHA-256 per arch | `Contents/Frameworks/` | script untested |
| Linux x86_64 | Collected in an Ubuntu 22.04 (glibc baseline) container, image digest pinned | **TBD** image digest + `libmpv2` package version | AppImage `usr/lib/`; deb/rpm `/usr/lib/downlink/mpv/` | script collects from the host libmpv for dev only |

## Dev machine (what M0 ran against)

- Debian 13, system `libmpv.so.2`, mpv 0.40.0 (client API 2.5), FFmpeg 7.1.5. Loaded through
  `DOWNLINK_MPV_PATH` (unsupported in releases).

## Rules

- Never fork mpv; never load a system libmpv in a release artefact.
- Staged output goes to `tools/mpv/out/<platform>/` (gitignored); the release workflow copies it
  into the bundle.
- A CI job runs each artefact on a clean VM with no mpv installed and plays a fixture to first
  frame (PLAN §4.10).
