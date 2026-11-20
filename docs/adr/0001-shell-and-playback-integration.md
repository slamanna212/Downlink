# ADR-0001: Shell and playback integration

Status: Accepted (plan decision D1, D7). Linux GNOME Wayland is a conditional go per `docs/M0-go-no-go.md`; Windows, macOS, KDE and X11 are unverified.

## Context

Downlink needs a desktop IPTV UI with controls, overlays and animation drawn over live video, on Windows, macOS and Linux
(Wayland and X11), with a Rust domain core that has no UI dependency. A webview cannot decode live MPEG-TS efficiently or
reliably, and each webview engine (WebView2, WKWebView, WebKitGTK) differs too much for a shared GPU-texture bridge.

## Decision

Rust workspace for the core, Tauri 2 shell, React 19.3 + TypeScript + Vite + Mantine 9 UI, in-process libmpv for all
playback. mpv renders into a native surface ordered beneath a transparent webview; the webview draws the whole interface.

| OS | Surface | mpv output |
|---|---|---|
| Windows | mpv `--wid` child HWND beneath the WebView2 child | `vo=gpu-next`, `gpu-api=d3d11` |
| macOS | layer-backed `NSView` below the WKWebView | render API (`mpv_render_context`) |
| Linux | `GtkOverlay` with a `GtkGLArea` under the transparent WebKitGTK webview | render API (OpenGL) |

Rust owns a `VideoSurface` trait (`attach`, `set_bounds`, `set_visible`, `detach`); the UI reports the video element's rect in
physical pixels. The window is `transparent: true`; only the app shell paints an opaque background, and the player route leaves
the video rect transparent. The `tauri-plugin-libmpv` crates are not used.

## Consequences

- Two surfaces to keep in sync, with three small per-platform shims instead of one webview texture bridge.
- Every UI layout must tolerate a transparent video region; no blur on Linux (`allowBlur` false).
- The core crates never depend on Tauri, wry, React or mpv (CLAUDE.md rule 1), so the shell can be replaced.

## What would change it

M0 shows the surface cannot hold 60 fps, ends up above the UI after fullscreen or Space switches, or steals focus from the
webview. Fallback order: a separate video window on the affected platform only, then a Flutter + media_kit shell (core crates
unchanged). An external mpv process stays only as the X11 emergency fallback.

## Implementation notes (2026-11)

- Linux path built and run (`apps/desktop/spike`): runtime-loaded libmpv, one pre-warmed instance, render API into a `GtkGLArea`
  inside a `GtkOverlay`. On GNOME Wayland (llvmpipe, no GPU) 96% of the framebuffer was non-black, 30 fps equal to the source, 0 drops, and
  the user confirmed video renders in the window on a real display. Still unverified: translucency and clicks over the video, 60 fps,
  fullscreen, KDE, X11. Windows `--wid` is written but untested; macOS is a skeleton.
- The GL area needs `internal_format=GL_RGBA8`, or mpv reports `INVALID_FRAMEBUFFER_OPERATION`.
- WebKitGTK fires no `requestAnimationFrame` while the compositor treats the window as hidden, so bounds reporting keeps a short
  timeout fallback beside the single-rAF debounce.
- mpv requires `LC_NUMERIC=C` and GTK resets it, so the engine forces it at construction.
- `vo=gpu-next` is not available through the render API in mpv 0.40.0 (it fails to initialise and aborts). Render-API platforms use
  `vo=libmpv`; only Windows can use `gpu-next` with `--wid`. See ADR-0002 for the HDR consequence.
