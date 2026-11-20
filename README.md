# Downlink

A desktop IPTV client for **Dispatcharr**, M3U/XMLTV playlists and Xtream accounts. Windows, macOS and Linux.

Downlink is a player. It ships no channels, playlists, guide data or credentials: the first run is an empty
"add a source" screen, and you bring your own.

> Works with [Dispatcharr](https://github.com/Dispatcharr/Dispatcharr) 0.32.0 or newer, tested with 0.34.1.
> Downlink is an independent project and is not affiliated with or endorsed by Dispatcharr.

## Features

- Channel list with groups, search, favourites and recently watched.
- Programme guide with now/next, local search of programme titles and descriptions.
- Catch-up and "restart this programme" on sources that offer an archive (Dispatcharr).
- Fast channel changes: the playback engine is pre-warmed and nothing waits on the network before a zap.
- Playback by libmpv, bundled with the app: H.264, HEVC, MPEG-2, AC-3/E-AC-3, teletext, hardware decode where the
  machine allows it. You never install mpv yourself.
- Works offline from its local cache: the channel list and guide open without the server.
- System tray, media keys and OS media controls (MPRIS, Windows media overlay, macOS Now Playing), keeps the screen
  awake while playing, pauses and releases the stream when the machine sleeps.
- Single instance and `downlink://channel/<uuid>` deep links.
- Updates through signed releases, checked in the background and installed when you quit. Never mid-playback.
- Your credentials live in the operating system keychain and are never written to the database or the logs.

## Screenshots

_Screenshots will be added here. They will only show synthetic test data, never real channels or provider artwork._

## Install

Download the latest build from the [Releases](../../releases) page. Dev builds are marked "pre-release".

Current builds are **not code-signed**, so each system asks you to confirm the first launch.

### Windows

Run the `Downlink_*_x64-setup.exe` installer (per-user, no admin rights needed).
SmartScreen may say "Windows protected your PC": choose **More info**, then **Run anyway**.

### macOS

Open the `.dmg` and drag Downlink to Applications. Builds are ad-hoc signed, so macOS blocks the first launch.
Clear the quarantine flag once, then open it normally:

```bash
xattr -dr com.apple.quarantine /Applications/Downlink.app
```

### Linux

Pick one:

- **AppImage**: `chmod +x Downlink_*.AppImage && ./Downlink_*.AppImage`
- **Debian / Ubuntu**: `sudo apt install ./downlink_*_amd64.deb`
- **Fedora / openSUSE**: `sudo dnf install ./downlink-*.x86_64.rpm`

The packages carry their own libmpv; they do not depend on your distribution's mpv. On GNOME the tray icon needs an
AppIndicator extension. Each Linux release also lists `SHA256SUMS-linux-x86_64.txt` and `.sig` files.

## Getting started

1. Start Downlink and choose **Add a source**.
2. For Dispatcharr, enter the server address and an API key (Dispatcharr: user menu, API keys).
3. Wait for the first sync, then pick a channel.

More in [`docs/user/testing.md`](docs/user/testing.md) (what to check and how to report a problem).

## Building from source

```bash
rustup toolchain install stable
corepack enable && corepack prepare pnpm@10 --activate
pnpm install
pnpm -C apps/ui build
cargo run -p downlink-desktop            # dev: DOWNLINK_MPV_PATH=/path/to/libmpv.so.2 uses a system libmpv
```

Linux build dependencies: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
pkg-config build-essential`. See [`docs/dev-setup.md`](docs/dev-setup.md) for the local test environment and
[`docs/PLAN.md`](docs/PLAN.md) for the design.

## Legal

Downlink is a media player. It does not provide, host, sell or link to any content, and it has no knowledge of what
you play. You are responsible for making sure that you have the right to access and watch the streams you add, under
the laws that apply to you and the terms of your provider. The authors accept no liability for how the software is
used. This is not legal advice.

Downlink contains and bundles mpv/libmpv (GPL) and FFmpeg libraries; their licences apply to those parts.

## Licence

GPL-3.0-or-later. See [`LICENSE`](LICENSE).
