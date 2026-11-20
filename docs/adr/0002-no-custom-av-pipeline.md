# ADR-0002: No custom A/V pipeline

Status: Accepted (plan decision D2).

## Context

Live IPTV playback needs demux, decode, A/V sync, hardware decode with fallbacks, subtitles (including teletext), HDR handling and
tolerance for broken live streams. Building that on FFmpeg directly would cost months and give users nothing visible.

## Decision

All playback goes through libmpv. We pin an mpv build, track upstream, never fork, and keep every mpv option string inside
`downlink-player` (CLAUDE.md rule 3). No mpv option appears in JS or in `downlink-core`.

## Consequences

- Playback quality tracks mpv; bugs are worked around with options or an mpv bump, not patches.
- The player crate is the single place that knows mpv. The UI sees a state machine and typed events.
- Features mpv lacks (for example HDR tone mapping parity across renderers) are accepted limits rather than reasons to own a renderer.

## What would change it

mpv cannot play a class of streams we must support, or its licence or embedding model blocks a platform. Even then the answer is
first a patched, pinned mpv build, not a bespoke pipeline.

## Implementation notes (2026-11)

- Built: `downlink-player` with runtime libmpv loading, options, a state machine and a retry policy.
- M0 found options that must be set to honour the project's network rules: `ytdl=no` and `load-scripts=no`. mpv's `ytdl_hook`
  re-requests a URL through yt-dlp after a failed open, which breaks the "no stream request except a real play" rule and leaks the URL.
- Per-file `http-header-fields` are forwarded by mpv across redirects, so redirect-profile channels are resolved in Rust and mpv is
  given the `Location` URL without headers (M0 negative control confirmed the leak when mpv follows the redirect itself).
- `gpu-next` through the render API is unavailable in mpv 0.40.0, so macOS and Linux render with `vo=libmpv`; HDR10 decodes and the
  pipeline reports `pq`/`bt.2020`, but tone mapping is the default renderer's and was not visually checked. Hardware decode was not
  testable (`hwdec-current=no` on the test machine).
