# M0 go / no-go record

Date: 2026-11-19. Scope: PLAN §13.4. Build: `apps/desktop` spike (`apps/desktop/spike/run.sh`), libmpv 0.40.0
(client API 2.5, FFmpeg 7.1.5) loaded at runtime through `DOWNLINK_MPV_PATH` (dev only).

**Test machine (Linux row): Debian 13, GNOME Shell 48.7 on Wayland, software GL (llvmpipe, "Suspected software
renderer"), no GPU, no audio device, and no physical monitor attached** (GDK logs `gdk_monitor_get_workarea` assertion
failures; the session was idle, and `document.visibilityState` in the webview was `hidden`, so the compositor withheld
frame callbacks and `requestAnimationFrame` never fired). That limits what could be proven; see "Not verified".

## Update: run on the user's real GNOME Wayland display

The user ran the spike on a real display (a VM: llvmpipe, no audio device): **video renders in the window, first time**. That
settles "video renders under a native GtkGLArea in a real GNOME Wayland session". Still pending the user's check: translucent
bar visible over the video, controls clickable, fullscreen, 60 fps. The repeated hwdec/CUDA/VAAPI/Vulkan/ALSA errors in that log
were the autorun zap loop on a machine with no GPU and no audio. Follow-up fixes: the first audio-open failure now switches mpv to
`ao=null` instead of retrying on every `loadfile`, and expected hwdec probe failures are logged at debug. Library-level stderr lines
(`Failed to open VDPAU backend ...`, `ALSA lib ...`) come from the libraries themselves and cannot be filtered from the player.

## Linux verdict (GNOME Wayland, this machine): CONDITIONAL GO

The playback path is proven end to end: runtime-loaded libmpv, one pre-warmed instance, render API into a `GtkGLArea` in a
`GtkOverlay` under the Tauri webview, per-file headers, zap, seek, HEVC and HDR10 decode. Zap time on the client side is
well inside budget. **Not proven** (no display on this machine): that the video is visible through the transparent webview
on screen, 60 fps, tearing, fullscreen and input routing. Those need one run on a real desktop session before M1 relies on
it; the code path is GTK-level and expected to work. If it does not, PLAN §2.2's fallback (separate video window on Linux) applies.

## §13.4 table

| Check | Result | Status |
|---|---|---|
| Video under UI, Linux GNOME Wayland | mpv renders into the GLArea: readback of the GL framebuffer shows 841,424 of 880,640 pixels non-black (96%, the rest is letterbox) at 1280x688, alpha 255, no GL errors after setting `internal_format=GL_RGBA8`. Render rate 30.0 fps (equals the 30 fps source; `GtkGLArea` render count over 4 s, p50 of 20 zaps run), 0 decoder drops, 0 `vo` drops. Bounds reported by the page in physical pixels reach the surface (`rect: 0,32 1280x688 @1x`). Transparency of the webview over the GLArea, visual result, tearing and clickability: **not verified** (no monitor; `gtk_widget_draw` and X11 `GetImage` captures are not valid for GL/transparent windows; GNOME screenshot D-Bus and portal were denied or unanswered). 60 fps untested (no 60 fps source on this box). | partial / needs a desktop run |
| Video under UI, Windows | `--wid` code written, unverified | pending tester |
| Video under UI, macOS | skeleton only (`attach` returns an error) | pending tester |
| Video under UI, KDE Wayland | not run | pending tester |
| Video under UI, X11 session | not run; X11 `GetImage` on our ARGB window is rejected (BadMatch) so it cannot be screenshotted by the usual tools | pending tester |
| Fullscreen / Space switch / taskbar | Fullscreen toggled 4x from Rust: `is_fullscreen` flips correctly, state stays `Playing`, but with no monitor the compositor stopped frame callbacks (render fps 21.5 then 0) so this is **inconclusive**, not a failure | pending desktop run |
| Clean machine, no mpv | Not run. The loader order bundled, `DOWNLINK_MPV_PATH`, system (logged loudly as unsupported) is implemented and unit-tested; `tools/mpv/*` are skeletons | pending (needs staged libmpv + clean VM) |
| Zap, local synthetic HTTP MPEG-TS (H.264 1080p30 + AAC) | 20 zaps alternating two streams, `loadfile ... replace` on the pre-warmed instance: **p50 251 ms, p95 389 ms**, max 513 ms (first, cold), all 20 reached `Playing` | pass (not the reference server) |
| Zap, local test Dispatcharr 0.34.1, `/proxy/ts/stream/<uuid>` with `X-API-Key` per file | 20 zaps alternating Channel One / Test News: **p50 5.49 s, p95 6.37 s**. Server-side cause: a cold channel takes ~5.1 s to first byte (`curl` TTFB 5.17 s, then 0.05 s when the channel is already running), i.e. ~0.35 s is client time. The synthetic stack (ffmpeg `-re` start-up plus proxy buffering) is not the reference server. | pending reference server; do not use this number against the 2 s gate |
| Redirect channel: key never reaches a non-server host | Resolved the 302 in Rust-style (no redirect following, key sent only to Dispatcharr) and gave mpv the `Location` with no headers. With a header-logging stand-in provider the request carried no `X-API-Key` (0 matches). Negative control: letting mpv follow the redirect itself with a per-file `http-header-fields` **does forward the key** to the second host, which is why PLAN §3.5 must resolve redirects in Rust. Real local "Redirect One": `Location: http://172.17.0.1:8088/ch/redirect-one.ts`, plays, first frame 667 to 861 ms. The synthetic stream server does not log request headers, so absence of the key there is by construction, not from its log. | pass (stand-in), real-server check pending |
| Catch-up session plays and seeks | Seek mechanics verified on a seekable local stream (`seekable` profile, +30 s: 3.0 s to 37.0 s, state `Playing`). Real catch-up **blocked server-side**: `POST /api/catchup/sessions/` returned 201 for "Catchup One" but `GET playback_url` returns `400 Cannot build timeshift URL` (the M3U-sourced channel has no timeshift URL template; likely needs an XC-style account or catch-up template in the test setup). Position POST therefore returned 404 as documented. Session was deleted (204). | blocked, pending server setup |
| HEVC / HDR sample | HEVC + AC-3 decodes (zap 225 to 391 ms). HDR10 (Main 10, PQ/BT.2020) decodes and the pipeline reports `yuv420p10`, `pq`, `bt.2020`, 0 drops. **No hardware decode on this machine**: `hwdec-current=no` (VAAPI/CUDA/Vulkan unavailable, software GL). Correct colour and tone-mapping not visually checked. | partial; hwdec and colour pending GPU machine |
| `gpu-next` through the render API | **Not available**: `vo=gpu-next` with the render API fails (`Error opening/initializing the selected video_out`) in mpv 0.40.0, and the process aborts. Render-API platforms use `vo=libmpv`. HDR tone mapping on macOS/Linux therefore uses the default libmpv renderer; verify separately (PLAN §4.3). | recorded |
| Reference server real channel | not run | pending server |

## Findings that changed the code

1. mpv refuses to run unless `LC_NUMERIC` is `C` and GTK resets it; `Engine::new` forces it.
2. mpv's `ytdl_hook` re-requests the URL through yt-dlp after a failed open (CLAUDE.md rule 4: no stream request except an
   actual play, and a privacy leak). Base options now set `ytdl=no` and `load-scripts=no`.
3. The GL framebuffer from `GtkGLArea` needs `internal_format=GL_RGBA8` or mpv reports `INVALID_FRAMEBUFFER_OPERATION`.
4. The webview does not fire `requestAnimationFrame` while the compositor considers the window hidden; the spike page keeps a
   120 ms timeout fallback beside the single-rAF debounce. The real UI should not depend on rAF for correctness of bounds
   while hidden.
5. Dispatcharr redirect play costs a server-side HEAD (already noted in `docs/dispatcharr-endpoints.md`).

## Not verified, and what is needed

- A run on a real desktop session (GNOME Wayland with a monitor, KDE Wayland, one X11 session): translucent bar over video,
  60 fps, clicks on controls, fullscreen, tearing. Use `apps/desktop/spike/run.sh`; the page shows state, fps and rect.
- Windows (`--wid` beneath WebView2) and macOS (render API, NSView below WKWebView) implementations.
- Clean-machine bundling with staged libmpv from `tools/mpv/`.
- Reference server zap numbers; catch-up play and seek; HDR colour and `hwdec-current` on a GPU.

Fallbacks stay as in PLAN §13.4. Nothing observed so far argues for the Flutter fallback.
