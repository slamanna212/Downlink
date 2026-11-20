# Downlink — plan

> Desktop IPTV client for Dispatcharr, M3U/XMLTV and Xtream. Windows · macOS · Linux.
> Revised 2026-11-19. This file is the source of truth; ADRs live in `docs/adr/`.
> Reference server: Sam's Dispatcharr at `10.1.20.202:9191` (0.32.0 at time of writing; upstream 0.34.1).
> Implementation: M0–M5 built (2026-11); M5b (multiview, track/stream pickers, mini player) in progress;
> Linux verified, Windows/macOS pending testers. Observed server
> behaviour lives in `docs/dispatcharr-endpoints.md`; performance numbers in `docs/perf.md`; M0 record in
> `docs/M0-go-no-go.md`.

## Contents

0. Decisions at a glance
1. Product
2. Architecture
3. Dispatcharr integration
4. Playback
5. Catch-up
6. Sources: M3U, XMLTV, Xtream
7. Data model, caching, sync
8. UX and UI (Mantine)
9. Roadmap and acceptance criteria
10. Releases, packaging, auto-update
11. Engineering handbook
12. Risks, decisions, open questions
13. Kickoff

---

## 0. Decisions at a glance

| Area | Decision |
|---|---|
| Name | **Downlink**. Crates `downlink-*`, bundle id `com.slamanna212.downlink`, deep links `downlink://`, app-data key `com.downlink.state.v1` |
| Shell | Tauri 2 (stable 2.x) + Rust workspace |
| UI | React **19.3** + TypeScript + Vite + **Mantine 9** (latest 9.x) |
| Playback | libmpv, in-process, rendered into a native surface **under** a transparent webview |
| Engine shipping | libmpv is bundled in every installer; never a system dependency |
| Scope | A **viewing client**. No admin console — Dispatcharr's web UI is the admin console |
| Must-have | Live TV, guide, **catch-up**, VOD, DVR scheduling/playback, M3U/XMLTV/Xtream sources, **multiview** |
| Releases | Same artefact mix as Apogee (§10), signed updater artefacts, `latest.json`, auto-update with stable + dev channels |
| Platforms | Windows x64, macOS universal (two Mac testers), Linux x86_64 (Wayland and X11) |
| Licence | GPL-3.0 with a GPL mpv build (default; confirm in §12) |

---

## 1. Product

### 1.1 Problem

Dispatcharr users run a capable server — several providers, EPG matching, output profiles, DVR,
catch-up — and then watch it through a browser tab, a Plex/Jellyfin tuner abstraction, or an IPTV
player that knows nothing about Dispatcharr. On the desktop (Windows HTPC, Mac mini under the TV,
Linux living-room box) nothing is a real client.

### 1.2 Who it is for

- **Operator** (Sam): runs Dispatcharr, a few providers, 1–3 users, knows what `tvg-id` is. Wants a
  fast client with a guide that never lags, catch-up, DVR and real playback quality.
- **Household member**: given an account by the operator. Needs a 10-foot UI, keyboard/remote
  navigation, favourites, resume and zero settings. Any complexity leaking into this persona is a bug.
- **Tinkerer**: wants a raw M3U/XMLTV or Xtream login, with or without Dispatcharr. Served by the
  sources in §6.

### 1.3 Jobs to be done

| Job | With Downlink |
|---|---|
| Put something on in under 10 seconds | Opens on the last channel or Continue Watching; one keypress plays |
| What's on tonight? | Virtualised guide grid, instant scroll |
| Watch channel 4 | Type `4`, Enter |
| I missed the start | **Restart this programme** from live, or pick any past programme in the guide (catch-up) |
| Record this | Guide → record, one-off or series |
| Why is it buffering? | Player info panel: source, bitrate, buffer, dropped frames, decoder, failover events |
| Finish that film on the other PC | Resume synced through Dispatcharr app-data |

### 1.4 Positioning

Downlink is the desktop client Dispatcharr is missing: mpv-class playback, a guide that keeps up,
working catch-up, and the server's viewing features (DVR, profiles, failover) without a browser.

Differentiators, in order:

1. **Dispatcharr-native** — API key, channels, profiles, DVR, catch-up sessions.
2. **Playback quality** — libmpv: HEVC, AC-3/E-AC-3 passthrough, HDR tone mapping, deinterlacing,
   teletext, hardware decode on all three OSes. Browser-engine players can't decode AC-3/E-AC-3.
3. **Guide performance** — virtualised two-axis grid, SQLite-backed.
4. **Looks** — only counts if it ships with the performance above.

### 1.5 Scope

**v1.0**

- Sources: Dispatcharr (REST, API key or username/password → JWT), Xtream (`player_api.php`),
  M3U/M3U8 + XMLTV. Multiple sources at once.
- Live TV: channel list, guide grid, groups, search, favourites, channel numbers, logos, now/next,
  catch-up badges, adult/radio flags, hidden channels respected.
- Playback: hardware decode, fast zap, audio/subtitle track picker (teletext and DVB subtitles come
  through it), passthrough, deinterlace, compact always-on-top mini player (§8.2), sleep inhibition.
- **Stream picker** (Dispatcharr only, and only when the account may switch streams): list the
  channel's provider streams in failover order and switch the live channel to one (§3.4).
- **Multiview**: up to 4 live channels at once (9 when hardware and the server's stream limit allow),
  one audible, layouts 2-up / 2×2 / 1+3 / 3×3 (§4.11, DESIGN.md §9).
- **Catch-up**: restart current programme, play past programmes from the guide, seek within the
  archive window, jump back to live (§5).
- VOD: movies and series browse, detail, resume, next episode.
- DVR: schedule/cancel from the guide (one-off and series), recordings list, watch completed and
  in-progress recordings.
- State: favourites, recents, resume, layout — local, synced to Dispatcharr app-data.
- OS integration: tray, media keys, OS media controls (SMTC / Now Playing / MPRIS), single instance,
  deep links (`downlink://play?channel=…`).
- Auto-update with stable and dev channels (§10).

**Non-goals**

- **Any admin surface.** No M3U account, EPG source, user, profile, output-profile, channel-editing,
  refresh-trigger, plugin, backup, log or notification management. Dispatcharr's web UI does all of it.
- A custom audio/video pipeline (ADR-0002).
- Mobile/TV builds. The core is portable, but the shipping targets are the three desktops.
- A server, sync service, cloud account or telemetry.
- Bundled playlists, provider recommendations, DRM (Widevine/PlayReady), transcoding, Stalker portals,
  casting, aspect/zoom controls. Each is a post-1.0 candidate at most.

### 1.6 What "done" looks like

Open the app → it is on the channel you left → arrow keys work → the guide opens instantly → a past
programme plays from the guide → recording sets in two keypresses → no spinner lasts more than a
second without saying what it is waiting on. Every network wait over 300 ms gets a state, every
failure gets a sentence and a retry, and no error message ever shows a credential-bearing URL.

### 1.7 Legal posture (not legal advice)

- Ships no content, playlists or credentials. First run is an empty "add a source" state.
- README carries the standard player-not-provider disclaimer.
- No provider names, real-provider fixtures or copyrighted channel screenshots in the repo
  (synthetic fixtures only).
- UI copy says "works with Dispatcharr", never implies it is official.

---

## 2. Architecture

### 2.1 Constraints

1. Windows 10/11 x64, macOS 13+ (universal: Apple Silicon + Intel), Linux x86_64 (Wayland and X11).
2. Codec coverage of real IPTV: H.264/HEVC in MPEG-TS, AC-3/E-AC-3/MP2/AAC, MPEG-2, teletext,
   HDR10/HLG, 1080i.
3. Zap p50 < 1.2 s, p95 < 2.5 s (keypress → first frame, 1080p H.264, reference server).
4. Guide and list stay at 60 fps.
5. One install, no system dependencies (no "install mpv first").
6. The playback engine is a dependency, not a deliverable.

### 2.2 ADR-0001 — Shell and playback integration

**Decision.** Rust workspace for the domain core, Tauri 2 shell, React 19.3 + TypeScript + Vite +
Mantine 9 UI, in-process libmpv for all playback. mpv renders into a **native surface ordered under a
transparent webview**; the webview draws the entire interface on top.

Per-platform surface (all proven in shipping or merged open-source Tauri apps; M0 confirms on our
build):

| OS | Surface | How mpv renders | Evidence |
|---|---|---|---|
| Windows | mpv's own `--wid` child HWND of the Tauri window, z-ordered **beneath** the WebView2 child | `vo=gpu-next`, `gpu-api=d3d11` | Frame Player (ships on Windows); Grid (`lpbborges/grid` #13: "libmpv's own wid child window, ordered beneath WebView2") |
| macOS | Layer-backed `NSView` inserted below the WKWebView (`addSubview:positioned:NSWindowBelow`) | **render API** (`mpv_render_context`) into a Metal/OpenGL layer we own — mpv's macOS backend has no window-embedding path, so do not use `--wid` | Frame Player ships macOS (it patched libmpv; we use the render API instead, as IINA does) |
| Linux | A `GtkOverlay` replaces Tauri's default window child; a `GtkGLArea` sits **under** the transparent WebKitGTK webview | **render API** (OpenGL) into the `GtkGLArea` | Grid #13 (native Wayland, "where `--wid` cannot"); `get-air/tauri-video-plugin` uses the same GtkOverlay pattern |

The Linux path is GTK-level, not a raw `wl_subsurface`, so it is expected to work on **both Wayland
and X11**. M0 verifies GNOME (Mutter), KDE (KWin) and one X11 session. If X11 fails, the fallback is a
separate video window on X11 only.

**Why not the render API into the webview?** A webview can't consume a GPU texture from mpv without a
per-engine shared-texture bridge (WebView2, WKWebView and WebKitGTK all differ). Native surface
under a transparent webview is one design with three small shims.

**Why not an external mpv process?** No UI over video, no proper PiP, no animated transitions, and a
second window that steals focus. Kept only as the X11 emergency fallback.

**Consequences.**

- Two surfaces to keep in sync. Rust owns a `VideoSurface` trait: `attach(parent)`,
  `set_bounds(x, y, w, h, scale)`, `set_visible(bool)`, `detach()`. The UI reports the video
  element's rect on resize/route change (ResizeObserver, one rAF debounce), in physical pixels.
- Window `transparent: true`; html/body/`#root` transparent; the opaque app background is painted by
  CSS on the app shell, and only the player route leaves the video rect transparent (§8.6).
- The `tauri-plugin-libmpv` crates are **not** used (Linux embedding unsupported, relies on system
  libmpv on macOS/Linux). We implement the surfaces ourselves.

**What would change this decision.** M0 shows the video surface can't hold 60 fps, appears above the
UI after fullscreen/Space switches, or steals input focus from the webview. Fallback order: separate
video window on the affected platform only → Flutter + media_kit shell (the core crates survive
unchanged because they have no UI dependency).

### 2.3 ADR-0002 — No custom A/V pipeline

libmpv already provides FFmpeg demux/decode, A/V sync, `gpu-next` rendering (HDR, tone mapping),
hardware-decode negotiation with safe fallbacks, subtitles including teletext, and a decade of
handling broken live streams. Owning a pipeline would cost months and buy nothing a user sees. We pin
an mpv build, track upstream, never fork, and keep every mpv-specific detail inside `downlink-player`.

### 2.4 Module layout

```
downlink/
├─ Cargo.toml                   # workspace
├─ crates/
│  ├─ downlink-core/            # no UI, no mpv. Pure domain.
│  │  ├─ source/                # Source trait: dispatcharr/ | xtream/ | m3u/ (lexer, attrs) | xmltv/
│  │  ├─ normalise/             # SourceItem → Channel/Stream/Programme, stable ids, diff
│  │  ├─ catalog/               # channels, groups, profiles, filtering (adult/radio/hidden) in ONE place
│  │  ├─ epg/                   # windowing, now/next, timezone handling
│  │  ├─ catchup/               # session API + URL template engine (§5)
│  │  ├─ vod/                   # movies, series, episodes
│  │  ├─ dvr/                   # recordings, series/recurring rules (read + create/cancel)
│  │  ├─ images/                # authenticated fetch, resize, content-addressed disk cache
│  │  ├─ store/                 # SQLite (sqlx), migrations, FTS5
│  │  ├─ sync/                  # Dispatcharr app-data read/write + merge
│  │  ├─ auth/                  # API key / JWT, keyring
│  │  └─ redact/                # credential redaction for logs, errors, diagnostics
│  ├─ downlink-player/          # libmpv FFI, sessions, state machine, property observation, VideoSurface impls
│  ├─ downlink-platform/        # tray, media keys, SMTC/Now Playing/MPRIS, power, deep links, single instance
│  └─ downlink-cli/             # headless dev tool: probe a source, dump EPG, play a channel
├─ apps/
│  ├─ desktop/                  # Tauri app: commands, state, window mgmt, updater
│  └─ ui/                       # React + TS + Vite + Mantine; no business logic
├─ tools/
│  ├─ mpv/                      # scripts that fetch/build and stage the pinned libmpv per platform
│  └─ fixtures/                 # synthetic M3U/XMLTV/JSON + ffmpeg stream recipes
└─ docs/                        # this plan + ADRs
```

Rules:

- `downlink-core` never depends on Tauri, wry, React or mpv. Its tests run headless in seconds.
- The UI never talks HTTP. Every call is a Tauri command returning a typed DTO.
- `downlink-player` owns every mpv option string. No mpv options in JS or in core.
- Every content origin is a `Source` returning the same normalised model plus a `Caps` bitflag. No UI
  or player code branches on source kind.
- Nothing except an actual play request touches a **stream endpoint**: `/proxy/ts/stream/`, `/proxy/hls/`,
  `/proxy/vod/`, catch-up session URLs, and Xtream `/live/`, `/movie/`, `/series/`, `/timeshift/`
  (see §3.5, §4.6). Status endpoints such as `/proxy/ts/status/` are fine.

### 2.5 Runtime design

**Process model.** One process, one Tokio runtime, one SQLite pool (`sqlx`, WAL, `busy_timeout=5s`),
one libmpv instance per playback session (pre-warmed at startup, reused across zaps).

**Commands (Tauri IPC)** are verbs: `add_source`, `test_source`, `list_channels`, `search`,
`get_guide`, `play_channel`, `play_catchup`, `restart_programme`, `player_command`, `set_favourite`,
`schedule_recording`, … Each returns `Result<T, AppError>`; `AppError` has a machine `code`, a human
`message` and an optional `retry_after`. The UI renders `message` and never invents failure copy.

**Events (Rust → UI)**: `player://state` (≤10 Hz), `catalog://updated` (with counts),
`session://stats` (1 Hz, only while the info panel is open), `task://progress`, `update://status`.
Coalesced per animation frame in JS.

**Refresh.** On connect and on schedule (§7.3): channels + groups, EPG window, background EPG fill.
VOD fetched on demand. All refreshes cancellable and resumable; a semaphore of 2 concurrent requests
per source so a Pi-class server is never hammered.

**Offline.** SQLite is the read model. Server unreachable → app opens from cache, shows a quiet
"Offline — last updated hh:mm" banner, playback fails with a specific sentence.

**Error taxonomy**: `auth`, `network`, `server` (5xx / version), `stream` (provider failure, no
sources, connection limit), `media` (codec/format), `local` (disk/DB). Only `auth` and
`server`(version) block; everything else degrades.

### 2.6 Security and secrets

- API keys, passwords and JWT refresh tokens live in the OS keychain (`keyring`: Credential Manager,
  Keychain, Secret Service). SQLite stores only a keychain reference. A test asserts no secret is
  written to the DB.
- TLS: self-signed allowed per source behind an explicit toggle with a warning; re-prompt if the
  certificate changes. Verification is never silently disabled.
- **Redaction** (hard rule, unit-tested): `/live/<u>/<p>/`, `/movie/<u>/<p>/`, `/series/<u>/<p>/`,
  `/timeshift/<u>/<p>/`, `player_api.php` / `get.php` / `xmltv.php` / `timeshift.php` query
  credentials, `X-API-Key`, `Authorization`, and provider URLs from redirects. Applied at the logging
  boundary, in `AppError` messages, in the debug panel and in the diagnostics bundle.
- No telemetry, no crash upload, no phone-home except the updater manifest fetch.
- CSP: app origin only; images come through the custom protocol (§3.6), never from remote origins.

---

## 3. Dispatcharr integration

Paths are from `GET /api/schema/?format=json` on the reference server (0.32.0: 237 paths / 347
operations). Downlink uses roughly 25 of them; the client is **hand-written** (serde + reqwest), not
generated.

### 3.1 Connecting

| Mechanism | Use |
|---|---|
| **API key** — `X-API-Key: <key>` (per-user, from Users → API & XC in Dispatcharr) | Primary |
| JWT — `POST /api/accounts/token/` → access (30 min) + refresh; `POST /api/accounts/token/refresh/` | Fallback when the user has no key; refresh in background |
| Session cookie login | Not used |

Connect sequence: `GET /api/core/version/` → `GET /api/accounts/users/me/` (user + level) →
`GET /api/core/timezones/` / settings for the server timezone.

User levels: **Admin**, **Standard**, **Streamer** (XC only). A Streamer key can browse and play;
anything it can't do is hidden, not shown-and-erroring.

### 3.2 API drift policy

Dispatcharr's API is unversioned. Policy:

1. Deserialize tolerantly: `#[serde(default)]` on optional fields, unknown fields ignored, enums as
   `String` with a `known()` helper.
2. Probe at connect: version string → capability set, cached per source. Never gate a feature on a
   single 404; fall back and remember.
3. Minimum supported server **0.32.0**; tested up to the latest upstream at each release. Shown in
   README and About.
4. A small CI script fetches the upstream OpenAPI schema weekly and opens an issue if any endpoint in
   `docs/dispatcharr-endpoints.md` changed shape. That is the only codegen-adjacent tooling.

### 3.3 Data shapes used

- (Observed shapes on 0.34.1, incl. deviations from this section, are in `docs/dispatcharr-endpoints.md`.)
- `Channel`: `id`, `uuid`, `channel_number` (float), `name`, `channel_group_id`, `tvg_id`,
  `epg_data_id`, `streams[]`, `stream_profile_id`, `logo_id`, `user_level`, `is_adult`,
  `is_catchup`, `catchup_days`, `is_radio`, `hidden_from_output`.
- `EPGGridProgram`: `id` (string — dummy EPG ids are synthetic), `start_time`, `end_time`, `title`,
  `sub_title`, `description`, `tvg_id`, `season`, `episode`, `is_new`, `is_live`, `is_premiere`,
  `is_finale`. Envelope `{"data":[…]}`.
- `Movie` / `Series` / `Episode`, `Recording`, `Logo` as per schema.
- `CatchupSessionCreate`: `{channel_uuid, start, duration?}` (duration 1–480 min).
- Lists are DRF-paginated `{count, next, previous, results[]}`; never assume `page_size` is honoured.

### 3.4 Feature → endpoint map

| Feature | Endpoints |
|---|---|
| Identity | `GET /api/accounts/users/me/`, `GET /api/core/version/`, token + refresh |
| Channels | `GET /api/channels/channels/?page=&page_size=&search=&channel_group=` |
| Channel streams (failover order) | `GET /api/channels/channels/{id}/streams/` |
| Stream picker (switch the live channel) | `POST /proxy/ts/change_stream/{channel_uuid}` `{stream_id}` — a control endpoint, not a stream endpoint. It moves the **server's** channel session, so everyone watching that channel switches too. Offered only to admin accounts (`Caps::STREAM_SWITCH`); a 403 hides it for the session |
| Groups | `GET /api/channels/groups/` |
| Channel profiles | `GET /api/channels/profiles/` |
| Stream profiles (redirect detection) | stream profile list + server default from core settings (exact paths verified in M1) |
| Guide | `GET /api/epg/grid/?channel_profile_id=&start=&end=&prev_days=&days=` |
| Now/next | `POST /api/epg/current-programs/` `{channel_uuids:[…]}` |
| Programme search | `GET /api/epg/programs/search/` (params verified in M1) |
| Logos | `GET /api/channels/logos/{id}/cache` |
| Live play | `GET /proxy/ts/stream/{channel_uuid}` (opt-in HLS: `…?output_format=hls` → `/proxy/hls/<token>/index.m3u8`, stop with `DELETE /api/proxy/hls/sessions/<token>/`) |
| Catch-up | `POST /api/catchup/sessions/`, `POST /api/catchup/sessions/{id}/position/`, `DELETE /api/catchup/sessions/{id}/` |
| VOD | `GET /api/vod/movies/`, `/series/`, `/series/{id}/episodes/`, `/categories/`; images `…/{id}/image`; play `GET /proxy/vod/{content_type}/{content_id}`. Details/episodes are lazy until `…/{id}/provider-info/` is called; episode payloads embed the provider account incl. credentials — parse typed fields only (see endpoints doc) |
| DVR | `GET/POST /api/channels/recordings/` (bare array), `…/{id}/stop`, `…/{id}/extend` (`{"extra_minutes": n}`), `…/{id}/file`, in-progress `…/{id}/hls/index.m3u8`; `recurring-rules/` (list, delete), `series-rules/` (`{"rules": […]}`; create then `series-rules/evaluate/`; delete by `?tvg_id=&title=`) |
| My session | `GET /proxy/ts/status/{channel_id}` (shape verified in M1) |
| App data | `GET/PUT /api/accounts/users/me/app-data/{key}/` |

### 3.5 Playing a stream

Primary: **`/proxy/ts/stream/{channel_uuid}`** with `X-API-Key` passed as a **per-file** mpv option
(`loadfile <url> replace -1 http-header-fields=…`), never as a global option. HLS is an **opt-in per-source
"Use HLS transport" setting** (decided 2026-11-20): Dispatcharr's HLS reads the same upstream buffer as TS,
so it only helps when something between client and server breaks long TS responses. There is no automatic
TS→HLS fallback. When on, Rust mints the session at play time
(`GET /proxy/ts/stream/{uuid}?output_format=hls`, redirects disabled → `/proxy/hls/<token>/index.m3u8`),
mpv gets the playlist with no auth headers (the token is the credential), the token is tracked in
`open_session` and deleted on every exit path, and a 410 re-mints at most once. XC `/live/` URLs are used only for Xtream
sources.

**Redirect-profile channels (key-leak protection).** If a channel's effective stream profile is
`Redirect`, Dispatcharr answers with a 302 to the provider URL, and FFmpeg would forward our
`X-API-Key` to the provider. Handling:

1. At sync, record each channel's **effective** stream profile (`stream_profile_id`, or the server
   default when null) and whether it is redirect-type. This is metadata only — no request to a stream endpoint.
2. At play time, for redirect-type channels only, Rust issues the play request itself with redirects
   disabled. A redirect profile answers with a 302 and no body; our client does not open a provider
   connection itself. (Observed on 0.34.1: Dispatcharr sends its own server-side HEAD — falling back to
   a streamed GET if the HEAD isn't 2xx — to the provider before replying, so this request is still
   only made when the user actually plays.) mpv gets the `Location` URL with **no** auth headers.
3. Proxy-type channels go straight to mpv with the header.
4. M0 verifies with a real redirect channel that the provider never sees the key.

**Never probe a stream endpoint.** A GET (or HEAD) on `/proxy/ts/stream/…` registers a client and makes the
server acquire a provider connection. No preflight, no prefetch, no "cheap GET". With
`max_connections` of 1–2 per provider, a probe steals the slot the next zap needs.

**Connection hygiene.** Stop the TS client (close the HTTP connection via `stop`) on navigate-away,
sleep and quit; `DELETE` HLS and catch-up sessions explicitly; on startup, delete any HLS/catch-up
session ids persisted from a crashed run. Backing out of the player into the guide is not navigate-away:
the guide's preview box becomes a mini player and holds the session; leaving the guide for any screen
other than the player stops it.

### 3.6 Images

Logos and VOD art come from authenticated endpoints. Never put the server URL in an `<img>`.
`downlink-core::images` fetches with auth, resizes to two sizes (160 px grid, 512 px detail),
stores in a content-addressed cache (`sha256(url)`), and serves through a Tauri custom protocol
(`dlimg://<hash>/<size>`). 512 MB LRU cap.

### 3.7 Pagination and load

- Paginate everything. Channel sync ≈ 2 pages at 500 for the reference lineup (544 channels).
- Guide: never fetch the whole range at once (§7.3). The grid endpoint takes `start`/`end`.
- No push feed exists; stats are polled at 1 Hz only while the info panel is open.

### 3.8 Things that will bite

1. `channel_number` is a float. Number entry: exact match wins, else the lowest channel ≥ the typed
   number.
2. `streams[]` order is the failover order; never reorder.
3. Store UTC, render local; never do date math in JS without an explicit zone (§8.6).
4. Programme ids are strings.
5. Adult/radio/hidden filtering lives only in `catalog/`; components never filter ad hoc.
6. `is_catchup` + `catchup_days` say a channel *may* have archive; the session endpoint is the
   authority for a specific programme. A 4xx becomes a plain sentence.
7. `hidden_from_output` channels are never shown in Downlink.

---

## 4. Playback

### 4.1 What we play

MPEG-TS over HTTP (default), HLS with TS segments, fMP4 per output profile, progressive MP4/MKV for
VOD. H.264, HEVC (4K), MPEG-2, occasional AV1. AAC, MP2, AC-3, E-AC-3, DTS/TrueHD in VOD. DVB
subtitles, teletext, SRT/ASS. HDR10/HLG; Dolby Vision in some VOD. 1080i is common in sports, so
deinterlacing is table stakes.

### 4.2 Why libmpv

| Option | Verdict |
|---|---|
| **libmpv** | Chosen: full FFmpeg codec coverage, hardware decode on all three OSes, `gpu-next`, teletext, render API for embedding |
| GStreamer | Rejected: per-OS plugin bundling (AC-3/E-AC-3 need bad/ugly) is a distribution project |
| libVLC | Viable fallback; weaker embedding and heavier runtime |
| FFmpeg + own renderer | Rejected (ADR-0002) |
| Web players (`<video>`, hls.js, mpegts.js) | Rejected: no AC-3/E-AC-3; HEVC depends on OS |

### 4.3 Options (the single source of truth — lives in `downlink-player`)

Base (all sessions):

```
vo=gpu-next              (Windows --wid path)  |  vo=libmpv (render API: macOS, Linux)
hwdec=auto-safe          gpu-api=auto (d3d11 on Windows)
video-sync=audio         keep-open=no          idle=yes
osc=no                   input-default-bindings=no   terminal=no
stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_delay_max=2
deinterlace=auto         alang=<pref>          slang=<pref>
user-agent=Downlink/<version> (<platform>; <OS>)   (same string as every core HTTP client)
```

Per-file headers go in `http-header-fields` as a string list: items joined with `,`, a literal comma
escaped as `\,` (mpv's `%n%` quoting does not apply inside the list). A plan `User-Agent` header goes
to the per-file `user-agent` option instead, so it replaces the default rather than being sent twice.

Profile `live` (live TV):

```
cache=yes   cache-secs=<Live buffer setting, default 2, range 1–10>
demuxer-readahead-secs=2   demuxer-max-bytes=32MiB   demuxer-max-back-bytes=16MiB
```

Profile `seekable` (catch-up, VOD, recordings):

```
cache=yes   demuxer-readahead-secs=20   demuxer-max-bytes=128MiB   demuxer-max-back-bytes=64MiB
```

M0 checks whether `gpu-next` is available through the render API in the pinned build; if not, the
render-API platforms use the default libmpv renderer and HDR tone mapping is verified separately.

**M0 result (Linux, mpv 0.40.0):** `vo=gpu-next` is not available through the render API, so render-API
platforms use `vo=libmpv`; HDR tone mapping is verified separately per platform. mpv also needs `ytdl=no`,
`load-scripts=no` and `LC_NUMERIC=C` (see ADR-0001/0002).

### 4.4 Session state machine

`Idle → Resolving → Connecting → Buffering → Playing ⇄ Paused → Stopped | Failed`.
Every transition is timestamped and visible in the debug panel. Buffering for more than 3 s says what
it is waiting on (connection, first data, first keyframe).

### 4.5 Failover

Dispatcharr already fails over between provider streams server-side. Client side: on `end-file` with
an error, or 8 s without a decoded frame → reconnect the same URL once → (Dispatcharr sources) ask for
the channel's stream list and surface the failure with the last error. Three attempts max, with
backoff. Never loop forever. Sources without server-side failover (`Caps::FAILOVER` absent) get the
same two-attempt retry and then a sentence.

### 4.6 Zap time

| Phase | Target |
|---|---|
| Keypress → command | < 30 ms (UI awaits nothing before `play_channel`) |
| Resolve play plan | < 50 ms (SQLite, no network) |
| `loadfile` → HTTP 200 | 150–600 ms (server-side provider acquire dominates) |
| First keyframe → first frame | 100–400 ms (GOP length) |
| **Total** | **p50 < 1.2 s, p95 < 2.5 s** |

Levers:

- Pre-warm one idle mpv instance at startup; zap with `loadfile … replace`, never recreate mpv.
- Show channel number/name/logo instantly; video fades in when it arrives.
- **Dwell prefetch is local only**: when a guide/list row is focused for > 300 ms, warm its now/next,
  logo and play plan from SQLite. It never touches the network for the stream.
- Prefer TS over HLS.

### 4.7 Hardware decode

`hwdec=auto-safe` by default, per-user override (`d3d11va`, `videotoolbox`, `vaapi`, `nvdec`,
`vulkan`, `no`). The info panel shows `hwdec-current`, so "green screen" reports are answerable in
one click.

### 4.8 Audio, subtitles, HDR

- Device list from mpv `audio-device-list`; chosen device remembered per machine; HDMI hotplug
  re-selects the default and continues.
- Output: Stereo / Surround / Passthrough (AC-3/E-AC-3/DTS only when the device reports it).
- `video-sync=audio`; no client DSP. Loudness is an output-profile concern in Dispatcharr.
- Subtitles: embedded, sidecar, teletext mode with page entry. Styling limited to size, colour,
  background. Track preference persists per channel and per series.
- HDR: `target-colorspace-hint=yes` on Windows/macOS; tone-map to SDR on Linux unless the compositor
  supports HDR. Dolby Vision badge only on actual detection.

### 4.9 Failure behaviour

| Failure | User sees |
|---|---|
| Provider stream dead | Freeze ≤ 2 s, "Reconnecting…", retry, then "Source unavailable" + "Try again" |
| Connection limit | "Your server is at its stream limit" |
| Unsupported codec | "This channel uses a format Downlink can't decode: <codec>" + Copy diagnostics |
| Server unreachable | Cache keeps working; banner "Offline — showing saved guide" |
| mpv instance dies | Supervisor restarts the session once, then reports with a crash id |
| Display sleep / lid close | Pause, release the stream (protects provider slots) |
| Catch-up not available for this programme | "This programme isn't in the archive" (from the session endpoint's 4xx) |

### 4.10 Bundled engine (ADR-0006)

**libmpv ships inside every installer.** The user never installs mpv and never runs a different mpv
than the one we tested.

| Artefact | What ships | Load |
|---|---|---|
| Windows NSIS | `libmpv-2.dll` + dependencies in `resources/mpv/` (zhongfly GPL x86_64 build, pinned by tag) | `LoadLibraryW` on the app-relative path |
| macOS universal `.app` / `.dmg` | **Universal** `libmpv.dylib` + every dependent dylib in `Contents/Frameworks/` | `@rpath` / `@loader_path`, fixed with `install_name_tool`; `dlopen` from the Frameworks dir |
| Linux AppImage | `.so` set in `usr/lib/` | `rpath=$ORIGIN/../lib`; scrub `APPDIR`/`LD_LIBRARY_PATH` from child environments |
| Linux `.deb` / `.rpm` | Private copy in `/usr/lib/downlink/mpv/` (via Tauri `bundle.linux.{deb,rpm}.files`) | `rpath` to the private dir; **no** `Depends: libmpv2` |

macOS universal is the hard one: libmpv and every dependency must be fat binaries. `tools/mpv/macos.sh`
builds (or collects from Homebrew bottles) on an arm64 runner and an x86_64 runner, collects each
dylib tree, and `lipo -create`s them. This is an M1 task, not M6.

Linux libs are collected in a pinned build container (Ubuntu 22.04 glibc baseline, image digest
pinned), so the bundled set is reproducible and never the end user's distro libmpv.

Load order: bundled → `DOWNLINK_MPV_PATH` (developer escape hatch) → system library, logged loudly as
unsupported. No code path ever asks the user to install anything.

Size: expect ~30–40 MB compressed per platform added by the engine; `.deb`/`.rpm` grow from Apogee's
~8 MB accordingly.

A CI job runs each artefact on a clean VM/container with **no mpv or libmpv installed**, asserts
`which mpv` is empty, and plays a fixture stream to first frame.

### 4.11 Multiview

- **Engines.** One mpv instance and one native surface per tile. Tile 1 is the main player (it keeps
  its pre-warmed instance and the full failover policy); tiles 2–9 get their own instances, created
  on entering multiview and destroyed on exit. Live channels only (no catch-up, VOD, recordings).
- **Surfaces.** Linux: one `GtkGLArea` per tile in the same `GtkOverlay`, each with its own render
  context. Windows: one `--wid` child HWND per tile beneath WebView2. macOS: one NSView per tile;
  until the macOS surface is reworked for several instances, macOS reports a cap of 1 tile and the
  entry points are hidden.
- **Options.** Tiles use profile `multiview` (owned by `downlink-player`): the `live` profile plus
  `hwdec=auto-safe`, `vd-lavc-skiploopfilter=nonkey` above 4 tiles, and `mute=yes` on every tile but
  the audible one. Only one tile is ever unmuted.
- **Budget.** Each tile is one stream on the server. The budget is `min(tile cap, account stream
  limit)` when the source reports a limit (Dispatcharr `stream_limit`, Xtream `max_connections`),
  else the tile cap. A tile that fails with `FailureKind::Limit` shows the stream-limit tile
  (Replace channel / Remove tile). 3×3 is offered only when hwdec is active and the budget is ≥ 9.
- **Sessions.** Rule 6 applies per tile: removing or replacing a tile, exiting multiview, sleep and
  quit stop that tile's stream and release any session it holds.
- **Network discipline.** Adding a tile is a play; the picker browses SQLite only (rule 4).

---

## 5. Catch-up

Catch-up is a v1.0 must-have and lands in M2 for Dispatcharr, M3 for M3U/Xtream.

### 5.1 User-facing behaviour

- **Guide**: past programmes on catch-up channels within `catchup_days` show an archive badge.
  Enter plays from the programme start. Programmes outside the window are visibly not playable.
- **Restart this programme**: from live playback, one key (and a control-bar button) restarts the
  current programme from its start through catch-up.
- **Seek** within the programme with ±10 s / ±60 s and a scrub bar; the bar shows the archive edge.
- **Go live**: one key returns to the live stream.
- **Continue watching**: catch-up positions resume like VOD (until the programme leaves the archive).
- Channel-level badge in the list and info panel: "Catch-up · 7 days".

### 5.2 Dispatcharr sources (preferred path)

1. `POST /api/catchup/sessions/` with `{channel_uuid, start, duration}` where `start` and `duration`
   come from the **EPG programme row** (one clock source for guide and archive).
2. Play the returned session URL with the `seekable` profile.
3. Report position with `POST …/{id}/position/` (throttled, every 10 s and on pause/seek).
4. `DELETE …/{id}/` on stop, zap, navigate-away, sleep and quit; ids persisted so a crashed run's
   sessions are deleted on next start.
5. A 4xx is the authority that this programme isn't playable → sentence from §4.9.

### 5.3 M3U and Xtream sources (template path)

The client builds the archive URL from a template. Template time semantics are carried as data per
channel (`utc_seconds` | `local_wallclock`), never inferred from the URL shape. That is what prevents
the classic "catch-up plays the wrong hour" bug.

| Mode | Build | Placeholders / form | Time semantics |
|---|---|---|---|
| `default` | `catchup-source` is the full URL | `{utc}`, `{utcend}`, `{start}`, `{end}`, `{duration}`, `{duration:N}`, `{channel-id}`, `{catchup-id}` | from `catchup-timezone` (Dispatcharr declares `UTC`) |
| `append` | append `catchup-source` to the live URL | same | same |
| `shift` | append `?utc={utc}&lutc={lutc}` | `{utc}`, `{lutc}` | UTC seconds |
| `xc` | `/timeshift/U/P/<minutes>/<YYYY-MM-DD:HH-MM>/<stream_id>.ts` | — | `server_info.timezone` wall clock |

`catchup-correction` (hours) applies before expansion; `catchup-days` bounds what the guide offers.
Flussonic mode is out of scope until someone asks.

Dispatcharr emits catch-up tags only on the XC-authenticated `/live/` playlist (the unauthenticated
`/output/m3u` has none), with:

```
#EXTM3U … catchup-timezone="UTC"
catchup="default" catchup-days="7"
catchup-source="http://host/streaming/timeshift.php?username=U&password=P&stream=<id>&utc={utc}&duration={duration:60}"
```

`{utc}` is programme start in Unix seconds; `{duration:N}` is length in seconds divided by N (Kodi convention), so `{duration:60}` is minutes; bare `{duration}` is seconds. Template-path
sessions have no server session to delete.

### 5.4 Acceptance

- Picking yesterday's programme in the guide plays it via `/api/catchup/sessions/`, seeks, resumes,
  and deletes its session on stop.
- The same channel added as an Xtream source plays the same programme via the `timeshift.php`
  template, with `{utc}`/`{duration}` filled from the same programme row.
- An `xc`-mode provider in a non-UTC timezone plays the correct hour (fixture test with a fake clock).
- "Restart this programme" from live starts at the programme's start time ± one GOP.

---

## 6. Sources: M3U, XMLTV, Xtream

### 6.1 The `Source` trait

```rust
#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;                     // stable, stored with every row
    fn kind(&self) -> SourceKind;                 // Dispatcharr | Xtream | M3u
    fn caps(&self) -> Caps;                       // EPG, VOD, DVR, CATCHUP, FAILOVER, LIMITS, PROFILES

    async fn connect(&self) -> Result<AccountInfo>;               // expiry, limits, timezone, user level
    async fn channels(&self, page: Page) -> Result<ChannelPage>;
    async fn guide(&self, window: TimeWindow) -> Result<Vec<Programme>>;
    async fn vod(&self, q: VodQuery) -> Result<VodPage>;
    async fn play(&self, ch: ChannelKey) -> Result<PlayPlan>;
    async fn catchup(&self, ch: ChannelKey, p: &Programme) -> Result<PlayPlan>;
    async fn stop_session(&self, s: SessionId) -> Result<()>;     // no-op for stateless sources
}
```

`PlayPlan = { url, headers, surface: Ts|Hls|Direct, profile: Live|Seekable, session: Option<SessionId>,
redact: RedactionSet }`. The player never learns where a URL came from.

Capability flags drive the UI: no `DVR` → no record buttons; no `CATCHUP` → no archive badges or
restart; no `LIMITS` → no connection counter.

Ingestion is separate from the source:
`fetch → normalise → diff against SQLite → batched upsert → catalog://updated`.

### 6.2 What "M3U" means here

| Thing | Handling |
|---|---|
| Extended M3U playlist (`#EXTM3U`/`#EXTINF`) | Parsed by `m3u/` |
| HLS manifest (`#EXT-X-*`) | Never parsed by us; mpv consumes it |
| Dispatcharr's `/output/m3u` | Just another M3U URL; has no catch-up tags |
| Xtream Codes (`player_api.php`, `get.php`, `/live/`) | `xtream/` adapter; covers Dispatcharr's XC surface and third-party panels |

### 6.3 M3U parser

- Header: `url-tvg`/`x-tvg-url` (may be a comma list), `url-logo`, `tvg-shift`, `catchup*`,
  `max-conn`.
- Entry: `#EXTINF:<dur> <attrs>,<name>` — split once on the first comma; quote-aware attribute scan;
  unknown attributes preserved.
- Attributes: `tvg-id`, `tvg-name`, `tvg-chno` (float), `tvg-logo`, `group-title`, `radio`,
  `tvg-shift`, `catchup*`, `type="playlist"` (include: fetch and merge, depth 2, cycle detection).
- Aux lines: `#EXTVLCOPT:http-user-agent=…` / `http-referrer=…`, `#EXTHTTP:{json}`, `#EXTGRP`
  (per-channel), `#KODIPROP` (DRM tags → channel labelled "DRM — not supported", excluded from play).
- Pipe headers on URLs (`url|user-agent=…`) → mpv `http-header-fields`; URL never rewritten.
- Hostile input: streaming (never `read_to_string`), lossy UTF-8 with Windows-1252 fallback, CRLF/CR/
  BOM, 50 MB single-line files, truncated entries. Per-entry error isolation; dropped entries counted.
- Hand-written (~400 lines). The `m3u` crate models audio playlists; `m3u8-rs` is for HLS.

### 6.4 XMLTV

- `quick-xml`, streaming, two passes (channels then programmes), 5,000-row batched transactions.
- Sniff gzip/xz magic bytes; don't trust the extension.
- Timestamps `YYYYMMDDHHMMSS ±HHMM`; missing offset → local time with a warning.
- Join order: `tvg-id` == `<channel id>` → `tvg-name` == `<display-name>` (also spaces→underscores) →
  display name; case-insensitive, `HD|FHD|UHD|4K` suffix stripped. Every non-id match is recorded so
  the UI can show "guide matched by name", and unmatched channels are one click from a manual remap.
- De-duplicate by `(channel, start, title)`; never delete future programmes on a failed refresh;
  show "guide last updated 4 h ago".

### 6.5 Xtream

`player_api.php?username=&password=` actions: (none) → `user_info` + `server_info`;
`get_live_categories`, `get_live_streams`, `get_short_epg`, `get_simple_data_table`,
`get_vod_categories`, `get_vod_streams`, `get_vod_info`, `get_series_categories`, `get_series`,
`get_series_info`. Guide from `xmltv.php`. Play URLs `/live/U/P/<id>.ts|.m3u8`, `/movie/…`,
`/series/…`; timeshift per §5.3.

Quirks absorbed in the adapter: numbers as strings, `auth: 0` with HTTP 200, `allowed_output_formats`
decides `.ts` vs `.m3u8`, `server_info.timezone` drives `xc` catch-up, load-balanced panels return
different hosts per request (resolve at play time, don't cache URLs). Dispatcharr's XC surface only
answers XC-enabled users; a 401 there is shown as a permissions problem, not a wrong password.

### 6.6 Normalisation

Strip quality suffixes into a `quality` field, map `group-title` / Xtream `category_name` to one group
table, assign a **stable synthetic id** (`tvg-id` if present, else `hash(name|group|url-host)`), so a
playlist refresh updates rows instead of duplicating them and favourites survive a provider URL change.

### 6.7 Measured reference (Sam's server, Dispatcharr 0.32.0)

- `/output/m3u`: 544 entries, 11 groups, attributes `tvg-id` (defaults to channel number as string),
  `tvg-name`, `tvg-logo` (server logo cache), `tvg-chno` (fractional, e.g. `5.5`), `group-title`; no
  catch-up tags.
- `/output/epg`: 4.6 MB, 15,098 programmes over 14 days. `days=1&prev_days=1` → 3.3 MB / 10,119
  programmes.

### 6.8 Acceptance

- A 150,000-entry M3U imports in < 3 s with < 300 MB peak RSS; one malformed entry never aborts it.
- Xtream against Dispatcharr and against one third-party panel fixture yield identical internal models.
- Re-importing an unchanged playlist changes zero row ids; a provider URL change keeps ids and
  favourites.
- A Widevine `#KODIPROP` entry shows "DRM — not supported".
- A credential-bearing playlist leaves no secret in logs (test).

---

## 7. Data model, caching, sync

ADR-0003: **SQLite is the read model; each source is the source of truth.** Every screen renders from
SQLite; network calls only refresh it.

### 7.1 Schema (v1 sketch)

```sql
CREATE TABLE source (
  id            INTEGER PRIMARY KEY,
  uuid          TEXT NOT NULL UNIQUE,      -- generated locally; identity for sync
  kind          TEXT NOT NULL,             -- dispatcharr | xtream | m3u
  name          TEXT NOT NULL,
  base_url      TEXT,
  secret_ref    TEXT,                      -- keychain entry name; never the secret
  username      TEXT,
  user_level    TEXT,                      -- dispatcharr: admin | standard | streamer
  server_version TEXT,
  server_tz     TEXT,
  caps          INTEGER NOT NULL DEFAULT 0,
  tls_insecure  INTEGER NOT NULL DEFAULT 0,
  last_sync_at  INTEGER,
  created_at    INTEGER NOT NULL
);

CREATE TABLE channel_group (
  id        INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  ext_id    TEXT NOT NULL,
  name      TEXT NOT NULL,
  UNIQUE (source_id, ext_id)
);

CREATE TABLE channel (
  id                INTEGER PRIMARY KEY,
  source_id         INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  ext_id            TEXT NOT NULL,         -- dispatcharr id, xtream stream_id, or synthetic id
  uuid              TEXT,                  -- dispatcharr channel uuid
  number            REAL,
  name              TEXT NOT NULL,
  group_id          INTEGER REFERENCES channel_group(id),
  tvg_id            TEXT,
  logo_key          TEXT,
  quality           TEXT,
  is_adult          INTEGER NOT NULL DEFAULT 0,
  is_radio          INTEGER NOT NULL DEFAULT 0,
  is_drm            INTEGER NOT NULL DEFAULT 0,
  catchup_days      INTEGER NOT NULL DEFAULT 0,
  catchup_template  TEXT,                  -- JSON: mode, source, time semantics, correction (m3u/xtream)
  stream_redirect   INTEGER NOT NULL DEFAULT 0,  -- effective stream profile is Redirect (dispatcharr)
  play_url          TEXT,                  -- m3u only; xtream URLs are built at play time
  play_headers      TEXT,                  -- JSON from EXTVLCOPT/EXTHTTP/pipe syntax
  epg_match         TEXT,                  -- id | name | display | manual | none
  sort_key          TEXT,
  UNIQUE (source_id, ext_id)
);
CREATE INDEX channel_group_idx  ON channel (source_id, group_id);
CREATE INDEX channel_number_idx ON channel (source_id, number);

CREATE TABLE channel_stream (               -- dispatcharr failover order
  channel_id INTEGER NOT NULL REFERENCES channel(id) ON DELETE CASCADE,
  stream_id  TEXT NOT NULL,
  position   INTEGER NOT NULL,
  PRIMARY KEY (channel_id, stream_id)
);

CREATE TABLE programme (
  channel_id  INTEGER NOT NULL REFERENCES channel(id) ON DELETE CASCADE,
  start_utc   INTEGER NOT NULL,
  end_utc     INTEGER NOT NULL,
  ext_id      TEXT,
  title       TEXT,
  subtitle    TEXT,
  description TEXT,
  season      INTEGER,
  episode     INTEGER,
  flags       INTEGER NOT NULL DEFAULT 0,   -- new | live | premiere | finale
  PRIMARY KEY (channel_id, start_utc)
);
CREATE INDEX programme_window_idx ON programme (start_utc, end_utc);

CREATE TABLE vod_item (
  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,                       -- movie | series | episode
  ext_id TEXT NOT NULL, parent_id INTEGER,  -- episode → series
  name TEXT NOT NULL, year INTEGER, rating TEXT, genre TEXT, description TEXT,
  season INTEGER, episode INTEGER, duration_secs INTEGER, art_key TEXT, updated_at INTEGER,
  UNIQUE (source_id, kind, ext_id)
);

CREATE TABLE recording (
  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  ext_id TEXT NOT NULL, channel_id INTEGER, start_utc INTEGER, end_utc INTEGER,
  state TEXT,                               -- scheduled | recording | completed | failed
  title TEXT, subtitle TEXT, rule_kind TEXT, -- one_off | series | recurring
  UNIQUE (source_id, ext_id)
);

-- local user state (mirrored to app-data, §7.5)
CREATE TABLE favourite (channel_id INTEGER PRIMARY KEY REFERENCES channel(id) ON DELETE CASCADE,
  position INTEGER, updated_at INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);
CREATE TABLE recent (channel_id INTEGER PRIMARY KEY REFERENCES channel(id) ON DELETE CASCADE,
  last_played_at INTEGER NOT NULL);
CREATE TABLE resume (
  source_id INTEGER NOT NULL, kind TEXT NOT NULL,      -- movie | episode | catchup | recording
  item_key TEXT NOT NULL, position_ms INTEGER NOT NULL, duration_ms INTEGER,
  updated_at INTEGER NOT NULL, PRIMARY KEY (source_id, kind, item_key));

CREATE TABLE open_session (                 -- for crash cleanup (§3.5, §5.2)
  source_id INTEGER NOT NULL, kind TEXT NOT NULL, session_id TEXT NOT NULL,
  PRIMARY KEY (source_id, kind, session_id));

CREATE TABLE image_cache (key TEXT PRIMARY KEY, path TEXT NOT NULL, bytes INTEGER NOT NULL,
  last_used INTEGER NOT NULL);

CREATE TABLE sync_state (source_id INTEGER NOT NULL, entity TEXT NOT NULL, last_ok INTEGER,
  last_error TEXT, PRIMARY KEY (source_id, entity));
```

Search: FTS5 external-content table `channel_fts(name, number_text, tvg_id)` with triggers; prefix
match ranked by number proximity, then `bm25`. Budget < 30 ms on a 150k-channel fixture.

### 7.2 Why

- Internal integer ids + `(source_id, ext_id)` uniqueness: multiple sources without collisions, and
  M3U/Xtream ids that aren't integers.
- UTC integers everywhere.
- Images keyed by URL hash dedupe across channels and sources.

### 7.3 Refresh policy

| Entity | When | Scope |
|---|---|---|
| Channels + groups | connect, manual, every 30 min while foreground | full paginated sync, diffed |
| Guide (visible) | connect, then every 15 min | now − 2 h … now + 12 h (`prev_days=1&days=1` for XMLTV-style outputs) |
| Guide (catch-up history) | after visible window | back to the longest `catchup_days` across channels (max 7 d by default) |
| Guide (ahead) | idle/background, every 6 h | forward to 7 days, day by day, cancellable |
| VOD | on section open | paged, cached 24 h |
| Recordings | on section open + every 60 s while visible | small |
| Session stats | only while info panel is open | 1 Hz |

Each sync writes one transaction per entity and emits `catalog://updated` with counts.

### 7.4 Limits

Image cache 512 MB LRU. Guide kept from (now − max catch-up days) to now + 7 days; older pruned.
DB target < 150 MB for 20k channels + guide.

### 7.5 Sync via Dispatcharr app-data

`GET/PUT /api/accounts/users/me/app-data/com.downlink.state.v1/` — 128 KiB per value.

```json
{
  "v": 1,
  "favourites": [{"s": "<source_uuid>", "c": "<channel ext_id>", "p": 0, "u": 1763543212, "d": false}],
  "resume":     [{"s": "...", "k": "movie", "i": "<item_key>", "ms": 4123000, "dur": 7200000, "u": 1763543212}],
  "recents":    [{"s": "...", "c": "<ext_id>", "u": 1763543212}],
  "prefs":      {"density": "comfortable", "u": 1763543212}
}
```

- Merge: every item is last-writer-wins by `u`. Deleted favourites stay as `d: true` for 30 days so
  deletes don't resurrect.
- Caps: 500 favourites, 200 recents, 500 resume entries; drop oldest resume entries first.
- Best-effort: local state is authoritative; a failed PUT retries with backoff.
- Only Dispatcharr sources sync. M3U/Xtream state is local.

### 7.6 Migrations

`sqlx::migrate!`, forward-only. Back up the DB before destructive migrations (keep two). If the DB is
newer than the app (downgrade after a dev-channel build), refuse to open and offer to archive it.

### 7.7 Offline matrix

| Action | Offline |
|---|---|
| Browse, guide, search | Works from cache, banner shows "as of hh:mm" |
| Favourites/recents/resume edits | Applied locally, synced later |
| Play live / catch-up / VOD | Fails with "server unreachable" + retry |
| DVR schedule | Disabled with "needs a connection to your server" |

---

## 8. UX and UI (Mantine)

Principle: **the content is the interface.** Logos, artwork and the guide carry the visual weight;
chrome recedes.

### 8.1 Information architecture

```
Home
├─ Continue watching        (last channel, VOD, catch-up, recordings)
├─ Favourites
├─ On now                   (current programme per favourite)
└─ Recently watched
Live TV
├─ Channels                 group sidebar, list
├─ Guide                    G
├─ Channel detail           now/next, catch-up window, record, favourite
└─ Multiview                2–9 live tiles (from the player or a channel's context menu)
Movies / Series             (absent if no source has VOD)
Recordings                  Upcoming · In progress · Completed  (absent if no source has DVR)
Settings
├─ Sources                  add/edit/remove: Dispatcharr · Xtream · M3U
├─ Playback                 hwdec, audio device/output, subtitles, live buffer, deinterlace
├─ Appearance               theme, density, 10-foot mode
├─ Updates                  channel (stable/dev), check now, auto-install on quit
└─ About & diagnostics      versions, hwdec, diagnostics bundle
```

Persistent left rail (collapsible) + one top bar per section. No hamburger. Back/Escape always goes
up one level and never quits.

### 8.2 Key screens

**Channels.** One row per channel (64 px comfortable / 44 px compact): logo (2:1 plate), number,
name, now/next with progress line, flags (catch-up, radio, HD/4K, DRM). Virtualised, ~60 DOM rows
max. Group sidebar with counts. Type-ahead search (`/` or just type) on name, number, `tvg-id`.

**Guide.** 30-minute ticks, day separators, "now" line. Sticky 180 px channel column and sticky time
header (two-axis virtualisation). Visible span 3/6/12/24 h. Past programmes with archive get a badge
and are playable; past programmes without archive are dimmed. Programme block: title (2 lines), time,
progress fill for the current one, new/live/finale badges, record dot when scheduled.
Click/Enter: now → play live; past with archive → catch-up; future → detail. Right-click/long-press:
record menu. `PgUp/PgDn` page, `Home` jumps to now. "No guide data for this channel" is a row.

**Player.** Video fills the window; controls fade after 3 s idle. Control bar: play/pause, restart
programme, ±10 s (seekable only), Go live (catch-up only), volume, audio track, subtitle track, PiP,
fullscreen, info, plus three menus: **audio** (tracks with language/codec), **subtitles** (off +
tracks, teletext included), and **stream** (Dispatcharr with `Caps::STREAM_SWITCH` only: provider
streams in failover order, current marked, a one-line note that switching affects everyone watching
the channel). Mini player and multiview buttons sit next to fullscreen. Zap overlay: number + logo + name pill for 1.2 s. Info panel (I): channel,
programme + progress, source/stream index, resolution, codec, bitrate, buffer, dropped frames,
`hwdec-current`. Errors are inline overlays with one action.
Back (control bar button or Esc) returns to the screen the player was opened from. From the guide,
playback continues in the preview pane's top-right box (mini player); clicking it reopens the player.
With nothing playing the box is a programme-art placeholder.

**Mini player (PiP).** `P` or the control-bar button turns the window into a compact, borderless,
always-on-top video window (default 480×270, remembers size and corner) with hover-only play/pause,
mute, restore and close. Restore (button, double-click or `P`) returns to the previous window size,
position and route. One surface, the same on all three OSes; no second window.

**Multiview.** As DESIGN.md §9: tiles on `--dl-multiview-canvas`, one audible, focused tile with an
inset accent ring, floating toolbar (layout, audio follows focus, stream budget, Exit), picker popover
on an empty slot. Entered from the player's grid button or "Add to multiview" on a channel.

**Recordings.** Upcoming (cancel), In progress (watch while recording, stop), Completed (play,
delete).

**Add source.** One screen, three tabs (Dispatcharr: URL + API key, with "use username/password"
link; Xtream: server/user/pass; M3U: URL or file + optional XMLTV URL). A **Test** button reports
channel and guide counts before saving. Then land on Live TV with the first channel focused. No tour.

### 8.3 10-foot mode

Toggle (also auto by window size and `--tv`): ~25 % larger type, taller rows, thicker focus ring,
directional navigation only, larger guide time scale, every critical action on arrows/OK/Back, no
hover-only affordances.

### 8.4 Keyboard map

| Key | Action |
|---|---|
| `↑/↓` | move selection |
| `←/→` | group (list) / time pan (guide) / seek (seekable player) |
| `Enter` | play / activate |
| `Space` | play/pause |
| `Esc` | back |
| `G` / `L` | guide / live list |
| `/` | search |
| `F` | favourite |
| `R` | record (guide context) |
| `S` | restart programme (catch-up) |
| `End` | go live |
| `I` | info panel |
| `A` / `T` | audio track / subtitle track menu (player) |
| `P` | mini player on/off |
| `M` | mute |
| `F11` / `⌃⌘F` | fullscreen |
| `0–9` | channel number entry (2 s timeout) |
| `Ctrl/Cmd+K` | command palette |

### 8.5 Visual system (brief for Claude Design)

Claude Design owns the actual tokens and outputs a Mantine `createTheme()` object plus any extra CSS
variables. Constraints it must respect:

- Dark by default (TV context); light theme optional.
- One UI sans with **tabular numerals** for times and channel numbers.
- Near-black neutral base, three elevation steps, one accent; green/red only for states.
- Restrained radii; not pill-shaped everything.
- Motion: 120–180 ms state changes, ~240 ms route transitions, no bounce, video fades in over 200 ms;
  nothing animates during scroll. Respect `prefers-reduced-motion`.
- Comfortable / compact density.
- Blur only on overlays, and **off on Linux** by default (WebKitGTK cost).
- Missing logo → channel initials on a deterministic colour from the channel id, never a grey box.
- Focus visible on every interactive element at all times (2 px accent ring + glow).
- Contrast ≥ 4.5:1 text, ≥ 3:1 large text and focus rings.

Screens for Claude Design to cover: Home, Channels, Guide (incl. past/catch-up/future states), Player
(live and catch-up control bars, zap overlay, info panel, error overlay), Add source, Recordings,
Settings, empty/loading/offline states, 10-foot variants of Channels/Guide/Player.

### 8.6 Mantine implementation notes

- **Versions**: React 19.3, Mantine 9 (all `@mantine/*` on the same 9.x), PostCSS preset.
- **Transparency**: MantineProvider paints `--mantine-color-body` on `body`. Set html, body and
  `#root` to transparent; paint the opaque background on the AppShell main/navbar; on the player route
  the video container is transparent so the native surface shows through.
- **Virtualisation**: `@tanstack/react-virtual` for the channel list and VOD grids. The guide is
  absolutely positioned blocks on **native** scroll (not `ScrollArea`), layout cached, no layout reads
  in the scroll handler. Do **not** use `@mantine/schedule` for the guide — it's an event calendar,
  not a 5k-row EPG.
- **Spatial navigation**: `@noriginmedia/norigin-spatial-navigation` from M1 for d-pad/remote focus;
  Mantine's own ring off (`focusRing: 'never'`) and our always-visible `--dl-focus-ring` applied via `:focus-visible` and `data-focused` (DESIGN.md §5).
- **Command palette**: `@mantine/spotlight`.
- **Time**: dayjs with `utc` and `timezone` plugins behind one `time.ts` module; no other date math
  in the UI.
- **Blur flag**: a theme-level `allowBlur` (false on Linux) gates every `Overlay blur` / backdrop
  filter.
- **Data**: TanStack Query wrapping Tauri `invoke`, invalidated by `catalog://updated`; Zustand for
  player/UI state fed by `player://state`.
- **Copy**: all strings through one `t()` layer (English only in 1.0); `Intl` for date/number format.

### 8.7 Empty, loading, error

- Empty: "Add a source" with the three tabs, plus where to find a Dispatcharr API key.
- Loading: skeletons in the real layout; a label after 300 ms ("Fetching guide…").
- Error: one sentence, cause if known, one action. No stack traces, no credential URLs.
- Degraded: quiet persistent banner, never a modal.

### 8.8 Performance budgets

| Interaction | Budget |
|---|---|
| Start → interactive (cached) | < 800 ms |
| Start → first frame (last channel) | < 3 s |
| Section switch | < 100 ms |
| Search keystroke → results | < 30 ms |
| Guide / list scroll | 60 fps, no long task > 16 ms |
| Zap | p50 < 1.2 s, p95 < 2.5 s |
| Memory (20k channels, 7 d guide) | < 400 MB RSS, video excluded |

### 8.9 Accessibility

Full keyboard operation, visible focus, ARIA roles on list/grid surfaces, reduced motion respected,
subtitle size/background control, screen-reader labels for channel rows (number, name, now, next).

---

## 9. Roadmap and acceptance criteria

Order matters more than durations; Claude Code builds it, so there are no day estimates.

```
M0 spike → M1 skeleton + release pipeline → M2 live TV + catch-up → M3 sources → M4 VOD + DVR → M5 polish
→ M5b multiview + pickers + mini player → M5c stabilise → M6 1.0 (Windows + Linux) → M7 macOS
```

- **M0 first**: compositing and engine bundling are the only existential risks.
- **Release pipeline in M1**, not at the end: the Mac testers get dev-channel builds with auto-update
  from the first usable build.
- **Catch-up in M2** with live TV, because it is a must-have and shares the guide.
- **Sources (M3) before VOD (M4)**, so VOD is written once against the `Source` trait.

### M0 — Feasibility spike

- Video renders under a translucent HTML control bar at 60 fps on Windows (`--wid` beneath WebView2),
  macOS (render API, NSView below WKWebView) and Linux (render API, GtkGLArea in GtkOverlay) — GNOME
  Wayland, KDE Wayland and one X11 session.
- Fullscreen, macOS Space switch, Windows taskbar auto-hide: video stays under the UI; input stays
  with the webview.
- Bundled libmpv loads on a clean machine/VM with no mpv installed, all three OSes (macOS via testers).
- A real channel from the reference server plays; zap measured p50/p95 over 20 zaps per platform.
- A Redirect-profile channel plays and the provider never receives `X-API-Key`.
- A catch-up session from the reference server plays and seeks.
- HEVC + HDR sample decodes with hwdec and correct colour.
- `gpu-next` availability through the render API recorded.
- One-page go/no-go record with numbers and, if needed, the per-platform fallback.

### M1 — Walking skeleton + release pipeline

Workspace and crates; CI build matrix; Dispatcharr client + `AppError`; keychain auth; SQLite schema +
migrations + FTS5; channel/group sync; player state machine + events; virtualised channel list with
play/pause and basic overlay; now/next; diagnostics panel + redaction; Mantine shell with theme from
Claude Design; spatial navigation wired; **full release workflow (§10) producing every artefact,
signed updater artefacts and `latest.json`; dev channel live**; macOS universal libmpv script.

Acceptance:

- Add a Dispatcharr source with an API key, see the channel count, play a channel.
- Killing the network after sync still shows the list.
- `cargo nextest` + `vitest` green on all three OSes.
- No credential in any log line (test).
- A dev build installed from the release auto-updates to the next dev build on Windows, macOS and
  Linux (AppImage + one of deb/rpm).

### M2 — Live TV + catch-up (Dispatcharr)

Guide grid (two-axis virtualisation, sticky axes); guide windowing + catch-up history fill +
timezones; search; favourites, recents, Continue Watching; **catch-up: guide playback, restart
programme, seek, go live, resume, session cleanup**; image pipeline; zap UX (pre-warm, local dwell
prefetch, overlay, failover); playback settings.

Acceptance:

- Guide opens < 300 ms from cache, scrolls at 60 fps with 5k channels, correct across timezones.
- Search finds by name or number in < 30 ms on a 150k-channel fixture.
- Favourites, recents, Continue Watching survive restart and follow the user to a second machine.
- §5.4 Dispatcharr criteria pass.
- Zap p50 < 1.2 s / p95 < 2.5 s on the reference server; failover visible and automatic.
- Browsing the guide for 60 s with the cursor moving generates **zero** requests to stream endpoints (test).

### M3 — Sources: M3U, XMLTV, Xtream

M3U lexer + aux lines + includes; XMLTV streaming parser + matching diagnostics + remap; Xtream
client; catch-up template engine (`default`, `append`, `shift`, `xc`); normalisation + stable ids;
add-source flow with Test; fixture corpus + fuzz target.

Acceptance: §6.8 and the M3U/Xtream items in §5.4.

### M4 — VOD + DVR

Movies and series browse/detail/play/resume/next episode; watch history; DVR schedule/cancel from the
guide (one-off, series); recordings list; in-progress playback; stop/extend; series/recurring rules
list with cancel.

Acceptance:

- Resume lands within 2 s of the saved position.
- A recording scheduled from the guide appears in Dispatcharr, starts, and is watchable while
  recording.
- Rules created in Dispatcharr's UI are visible and cancellable in Downlink.
- VOD/DVR failures produce a sentence and one action, never a blank screen.

### M5 — Polish, 10-foot, OS integration

10-foot mode; full keyboard map + command palette; motion pass; empty/error/offline states; logo
fallback; performance pass (startup, guide, memory); accessibility pass; tray, media keys, OS media
controls, single instance, deep links; diagnostics bundle.

Acceptance:

- Every key in §8.4 works; focus visible everywhere; reduced motion respected.
- §8.8 budgets hold.
- Media keys and OS media controls drive playback on all three OSes.
- A first-time user goes from install to a playing channel in under 60 s, unassisted.

### M5b — Multiview, pickers, mini player

Audio and subtitle track menus in the player; Dispatcharr stream picker behind `Caps::STREAM_SWITCH`;
compact always-on-top mini player; multiview (§4.11, DESIGN.md §9) with picker, layouts, audio focus,
stream budget and the stream-limit tile.

Acceptance:

- Track menus list every audio/subtitle track mpv reports and switching takes effect without a
  restart; "Off" disables subtitles; teletext pages appear as subtitle tracks when present.
- The stream menu appears only for a Dispatcharr admin account, lists streams in failover order and
  switching plays the chosen stream; a non-admin account never sees it.
- Mini player toggles both ways, stays on top, and restores the previous size, position and route.
- Four tiles play at once on the reference server with one audible; removing a tile, exiting
  multiview and quitting stop every tile's stream (test); the picker generates zero stream-endpoint
  requests while browsing (test); a limit failure shows the stream-limit tile.

### M5c — Stabilise

No new features. Work through how each flow actually behaves day to day on Linux, fix every bug found,
polish copy, motion and states, and keep §8.8 budgets. Dev-channel builds only. Ends when Sam calls
the app stable.

### M6 — 1.0 (Windows + Linux)

Stable channel opens; releases cover Windows and Linux only (`ENABLE_MACOS` stays off); signing as
§10.5; README, disclaimer, install notes; clean-machine QA on Windows 11 and Linux; Windows-specific
bugs (surface z-order, DPI, fullscreen, taskbar auto-hide, multiview HWNDs) fixed before 1.0 ships;
update path tested stable → stable and dev → stable.

Acceptance:

- Every Windows and Linux §10.1 artefact installs and launches on a clean machine.
- The updater moves a user from 1.0.0 to 1.0.1.
- No provider names or copyrighted screenshots in the repo.

### M7 — macOS

Turn on `ENABLE_MACOS`; Developer ID signing and notarisation (§10.5); universal libmpv; the macOS
surface reworked for several instances so multiview works; Mac testers verify the M0 checks and every
milestone's acceptance; fix what they find, then ship macOS on the stable channel.

### Definition of done (every milestone)

1. Works on Windows 11, macOS (tester-verified), Linux Wayland + X11 — or the gap is an open issue.
2. Unit tests for logic; one integration test per endpoint touched (against fixtures, not live calls).
3. No `unwrap()`/`expect()` on network or user-input paths.
4. §8.8 budgets still hold (measured).
5. Every new failure mode has a line in the debug panel.
6. This plan is updated in the same PR that invalidates it.

### Deliberately out of 1.0

Admin features of any kind, casting, downloads/offline VOD, phone remote, Stalker portals,
Flussonic catch-up, cross-source channel merging, gamepad, mobile/TV ports, DRM, aspect/zoom controls,
a detached floating PiP window. (macOS ships after 1.0, in M7.)

---

## 10. Releases, packaging, auto-update

Mirrors Apogee's release layout.

### 10.1 Artefacts per release

Versioned (with `.sig` updater signatures where listed):

```
Downlink_<ver>_x64-setup.exe          + .sig     Windows (NSIS)
Downlink_<ver>_universal.dmg                      macOS install
Downlink_<ver>_universal.app.tar.gz   + .sig     macOS updater payload
Downlink_<ver>_amd64.AppImage         + .sig     Linux
Downlink_<ver>_amd64.deb              + .sig     Linux (Debian/Ubuntu)
Downlink-<ver>-1.x86_64.rpm           + .sig     Linux (Fedora/RHEL)
latest.json                                       updater manifest
SHA256SUMS-linux-x86_64.txt                       checksums
```

Unversioned copies for stable "latest" download links
(`/releases/latest/download/<name>`):

```
Downlink_x64-setup.exe
Downlink_universal.dmg
Downlink_amd64.AppImage
Downlink_amd64.deb
Downlink-x86_64.rpm
```

Plus GitHub's source archives and a **release attestation** (`actions/attest-build-provenance`).

Not shipped: MSI, Windows arm64, separate per-arch macOS builds, Linux arm64, Flatpak, store
listings.

### 10.2 Build matrix (GitHub Actions)

| Runner | Produces |
|---|---|
| `windows-latest` | NSIS `.exe` + `.sig` |
| `macos-latest` (arm64), target `universal-apple-darwin` | `.dmg`, `.app.tar.gz` + `.sig` (universal libmpv from §4.10) |
| `ubuntu-22.04` (pinned container for libmpv collection) | AppImage, `.deb`, `.rpm` + `.sig`, `SHA256SUMS-linux-x86_64.txt` |

`tauri-apps/tauri-action` builds and uploads to a draft release; a final job generates `latest.json`
(`tools/release/make-latest-json.py`), copies the unversioned assets, creates the attestation and publishes.
Workflow layout follows Apogee: `release.yml` (push to `main`), `dev-prerelease.yml` (manual, `dev` branch),
`pr-ci.yml` (PRs to `dev`), plus reusable `ci.yml` and `build.yml`. The Linux runner label is the repo variable
`LINUX_RUNNER` (default `ubuntu-22.04`). macOS jobs run only when repo variable `ENABLE_MACOS` is `true`
(releases start as Windows + Linux; macOS follows with a Developer ID, §10.5). Without
`TAURI_SIGNING_PRIVATE_KEY` builds still publish installers, minus `.sig` files and `latest.json`.

`tauri.conf.json`: `bundle.targets = ["nsis", "dmg", "app", "appimage", "deb", "rpm"]`,
`bundle.createUpdaterArtifacts = true`, `bundle.macOS.signingIdentity = "-"` (ad-hoc) until a
Developer ID exists.

### 10.3 Auto-update

- `tauri-plugin-updater` with the minisign public key compiled in; private key and password in GitHub
  secrets (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`), never in the repo.
  The updater refuses any payload whose signature doesn't verify.
- `latest.json` platform keys: `windows-x86_64` (NSIS), `darwin-aarch64` and `darwin-x86_64` (both
  → the universal `.app.tar.gz`), `linux-x86_64` (AppImage), `linux-x86_64-deb`, `linux-x86_64-rpm`.
- **Channels**:
  - **stable**: endpoint `https://github.com/slamanna212/Downlink/releases/latest/download/latest.json`
    (GitHub's `latest` excludes pre-releases). Every push to `main` builds `vX.Y.Z` from the root
    `package.json` version unless that release already exists, and leaves it as a draft to publish by hand
    (a `-suffix` version is marked pre-release).
  - **dev**: pre-releases tagged `vX.Y.Z-dev.<shortsha>` (Apogee's scheme), built from the `dev` branch only
    when triggered by hand (`dev-prerelease.yml`). A rolling `dev` release always holds the newest dev
    `latest.json`; endpoint `…/releases/download/dev/latest.json`.
  - Chosen in Settings → Updates. Switching dev → stable only offers a build newer than the installed
    one (no automatic downgrade).
- **UX**: check on startup and every 6 h. Download in the background. Never interrupt playback:
  a quiet "Update ready — restart to install" banner plus an "install on quit" default. "What's new"
  from the release notes. No modal unless the user clicks.
- deb/rpm updates install with elevation (polkit prompt); AppImage replaces itself; NSIS runs
  passive; macOS swaps the `.app`.
- Downgrade guard: a DB newer than the app refuses to open (§7.6), so dev → stable never corrupts
  state.

### 10.4 Versioning and changelog

Semver. Conventional Commits; `git-cliff` generates release notes, user-visible section hand-edited.
The **root `package.json` `version` is the single source of truth** (bumped with
`tools/release/bump-version.sh`); `tauri.conf.json`, `Cargo.toml` and `apps/ui/package.json` are overwritten from it
at build time (`tools/release/set-version.mjs`) so they cannot drift. `[skip release]` in a commit message and
docs-only changes skip the release workflow.
README and About show "Works with Dispatcharr ≥ 0.32.0, tested with <latest>".

### 10.5 Signing (decide before 1.0)

| Platform | Dev channel / testers | Before public 1.0 |
|---|---|---|
| Windows | Unsigned; SmartScreen "More info → Run anyway" | Optional: Azure Trusted Signing (~$10/mo) to remove the warning |
| macOS | Ad-hoc signed universal build; testers run `xattr -dr com.apple.quarantine /Applications/Downlink.app` once | Optional: Apple Developer ID ($99/yr) + notarisation; requires inside-out signing of every bundled dylib with hardened runtime and a `codesign --verify --deep --strict` CI step |
| Linux | `.sig` + `SHA256SUMS` | same |

Updater signatures (minisign) are required regardless of OS code signing.

**Decided 2026-11-20 (Sam):** releases start with **Windows and Linux**; Windows stays unsigned for now.
**macOS** follows soon, signed and notarised with Sam's existing **Apple Developer ID** (the macOS release jobs
are gated behind the `ENABLE_MACOS` repo variable until then). Azure Trusted Signing is not planned.

---

## 11. Engineering handbook

### 11.1 Toolchain

| Thing | Choice |
|---|---|
| Rust | stable, MSRV pinned in `rust-toolchain.toml` |
| Node | 22 LTS, pnpm 10, committed lockfile |
| Tauri | 2.x stable, minor pinned |
| React | 19.3.x |
| Mantine | 9.x (all packages same version) |
| libmpv | pinned per platform (§4.10), recorded by version + source in `tools/mpv/PINNED.md` |
| SQLite | bundled via `sqlx` / `libsqlite3-sys` bundled |
| CI | GitHub Actions |

### 11.2 Repo conventions

Trunk-based, `main` always releasable (every push → dev build). Conventional Commits. PRs state what
changed, why, how verified, screenshots for UI, and whether they touch the player surface. ADRs in
`docs/adr/NNNN-title.md`.

### 11.3 Quality gates

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
cargo nextest run --workspace
pnpm -C apps/ui lint && pnpm -C apps/ui typecheck && pnpm -C apps/ui test
pnpm -C apps/ui build
```

- `unsafe` only in `downlink-player` / `downlink-platform` shims, each with a `// SAFETY:` comment.
- `cargo-deny` licence allowlist: MIT, Apache-2.0, BSD-2/3, ISC, MPL-2.0, Zlib, Unicode-3.0, plus GPL
  for the mpv boundary (GPL-3.0 project).

### 11.4 Testing

1. **Unit** (`downlink-core`): parsers, catch-up template expansion (with fake clocks and timezones),
   time math, pagination, merge, redaction, error mapping.
2. **Integration**: every command against a mock Dispatcharr (`wiremock`) built from captured,
   scrubbed JSON fixtures; Xtream and M3U fixtures for the other sources.
3. **Player**: fixture media server (ffmpeg-generated: H.264/AAC, HEVC/AC-3, 1080i, broken timestamps,
   mid-stream drop) with assertions on the state machine and decoded frames, all three OSes.
4. **Network-discipline test**: a guide-browsing scripted session asserts zero stream-endpoint requests.
5. **UI**: Vitest for logic/hooks; Playwright against the Vite dev server for flows.
6. **Manual matrix per release**: Windows 11, macOS (both testers), Ubuntu 24.04 Wayland, Fedora KDE
   Wayland, one X11 session.

### 11.5 Diagnostics

`tracing` to a rolling file log (5 × 5 MB), `info` default, `debug` toggle in Settings. **Diagnostics
bundle** button: redacted logs, app/server versions, player state, `hwdec-current`, OS/GPU, last 50
state transitions. Built in M1.

### 11.6 Dev environment

```bash
# test Dispatcharr — never point automated tests at the real server or real providers
docker run -d --name dispatcharr-test -p 9191:9191 -v dispatcharr_test:/data \
  ghcr.io/dispatcharr/dispatcharr:latest

pnpm install
pnpm tauri dev
```

`downlink-cli` exercises the core without the UI:
`cargo run -p downlink-cli -- --source http://localhost:9191 --key $KEY channels --search news`.

Local dev may use a system mpv/libmpv (`DOWNLINK_MPV_PATH`); release artefacts never do.

### 11.7 Running costs

| Item | Cost |
|---|---|
| GitHub Actions | $0 on a public repo |
| Test Dispatcharr | $0 (container on existing hardware) |
| Apple Developer ID | $99/yr — optional (§10.5) |
| Azure Trusted Signing | ~$120/yr — optional (§10.5) |

---

## 12. Risks, decisions, open questions

### 12.1 Risk register

| # | Risk | L | I | Mitigation | Trigger |
|---|---|---|---|---|---|
| R1 | Video-under-webview compositing fails on a Linux compositor or X11 | M | H | GtkOverlay/GtkGLArea path; test GNOME, KDE, X11 in M0; separate-window fallback per platform | M0 < 55 fps, black video, focus loss |
| R2 | macOS fullscreen/Space switch puts video above UI or loses it | M | M | M0 via testers; re-attach on screen change | Video over control bar after Space switch |
| R3 | API key leaks to a provider via redirect or logs | M | H | Redirect classification + no-follow resolve (§3.5); per-file headers; redaction tests | Key in mpv log or provider access log |
| R4 | Network probing burns provider connection slots | M | H | No stream-endpoint requests except play; network-discipline test | Ghost clients in Dispatcharr stats |
| R5 | Dispatcharr API drift | H | M | Tolerant deserialization, version probe, weekly schema check | 400/404 on a previously working call |
| R6 | Universal macOS libmpv bundle is broken or incomplete | M | H | `tools/mpv/macos.sh` in M1; `lipo -info` + `otool -L` checks in CI; testers on clean Macs | "Library not loaded" on launch |
| R7 | Bundled libmpv missing on Linux deb/rpm or AppImage env leaks | M | H | Private lib dir + rpath; env scrub; clean-container playback test | mpv fails to load / wrong OpenSSL |
| R8 | Catch-up plays the wrong hour | M | M | Time semantics as data; fake-clock tests per mode and timezone | Archive starts offset by whole hours |
| R9 | Zap slower than target on real providers | M | M | Pre-warm, TS over HLS, live buffer tuning | p95 > 2.5 s in M2 |
| R10 | Guide payload too large / slow | L | M | Windowed fetch, cache | Guide load > 5 s |
| R11 | Orphaned sessions burn connection limits | M | M | Explicit stop/delete on every exit path; crash cleanup table | Ghost clients after sleep/crash |
| R12 | M3U/XMLTV parser falls over on huge/malformed input | M | M | Streaming parser, per-entry isolation, fuzzing | Import aborts or RSS > 300 MB |
| R13 | Users bring DRM channels | M | L | Detect at import, label, exclude | Support questions about black screens |
| R14 | Unsigned builds deter non-technical users | M | L | Documented first-run steps; optional signing before 1.0 | Testers stuck at SmartScreen/Gatekeeper |
| R15 | Updater ships a broken build to everyone | L | H | Dev channel first; stable only from tagged releases; downgrade guard | Crash reports after a stable release |
| R16 | Licence mismatch (GPL mpv) | L | M | GPL-3.0 project | Packaging/licence review flags it |

### 12.2 Decision log

| ID | Decision | Status |
|---|---|---|
| D1 | Tauri 2 + React 19.3 + Mantine 9 + in-process libmpv under a transparent webview (ADR-0001) | **Decided (Sam)** |
| D2 | No custom A/V pipeline (ADR-0002) | Decided |
| D3 | API key primary, JWT fallback | Decided |
| D4 | GPL-3.0 + GPL mpv build | Proposed default — confirm |
| D5 | SQLite read model (ADR-0003) | Decided |
| D6 | Viewing client only; no admin console | **Decided (Sam)** |
| D7 | Linux: Wayland and X11 via the GTK surface path; per-platform fallback if M0 fails | Decided |
| D8 | Local-first state, Dispatcharr app-data sync | Decided |
| D9 | Name: **Downlink** | **Decided (Sam)** |
| D10 | M3U/XMLTV/Xtream sources in 1.0 (M3) | Decided |
| D11 | libmpv bundled in every installer (ADR-0006) | **Decided (Sam)** |
| D12 | Catch-up is a 1.0 must-have (Dispatcharr in M2, templates in M3) | **Decided (Sam)** |
| D13 | Release artefacts mirror Apogee; auto-update with stable + dev channels | **Decided (Sam)** |
| D14 | macOS universal, validated by two Mac testers | **Decided (Sam)** |
| D15 | Hand-written Dispatcharr client (no codegen) | Decided |
| D16 | Automated tests use a separate test Dispatcharr container, never real providers | Decided |
| D17 | Multiview is a 1.0 feature: 4 tiles, 9 when capable (§4.11) | **Decided (Sam)** |
| D18 | Stream picker switches the server's shared channel session; shown only to accounts allowed to (`Caps::STREAM_SWITCH`) | **Decided (Sam)** |
| D19 | PiP is a compact always-on-top mode of the main window, not a second window | **Decided (Sam)** |
| D20 | Release order: stabilise on Linux → 1.0 on Windows + Linux → macOS (M7) | **Decided (Sam)** |

### 12.3 Open questions

1. **Licence** — GPL-3.0 (default) or something else? A permissive licence forces an LGPL mpv build
   and loses mpv's X11 output.
2. **Signing before 1.0** — decided: Windows unsigned, macOS via Apple Developer ID when Mac builds start (§10.5).
3. **Multi-user on one machine** — default: the OS user account is the profile in 1.0.
4. **Repo name** — default `slamanna212/Downlink` (used in the updater endpoints above).

---

## 13. Kickoff

### 13.1 Day one

1. Create `slamanna212/Downlink`, add this plan and `CLAUDE.md`.
2. Generate the updater keypair (`pnpm tauri signer generate`), store the private key + password as
   repo secrets, put the public key in `tauri.conf.json`.
3. Stand up the test Dispatcharr container with a synthetic M3U (ffmpeg-generated channels served by
   a local web server), one dummy EPG with past programmes, catch-up enabled on at least one channel,
   one Redirect-profile channel, and an API key.
4. Send the testers the first dev build once M1's release workflow is green.

### 13.2 Toolchain

```bash
rustup toolchain install stable && rustup component add clippy rustfmt
corepack enable && corepack prepare pnpm@10 --activate
cargo install tauri-cli --version "^2" --locked
cargo install cargo-nextest cargo-deny git-cliff --locked

# Linux build deps (Ubuntu 24.04 dev box)
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libmpv-dev ffmpeg pkg-config build-essential
```

### 13.3 Synthetic streams

```bash
ffmpeg -re -f lavfi -i testsrc2=size=1920x1080:rate=30 -f lavfi -i sine=frequency=440 \
  -c:v libx264 -preset veryfast -g 60 -c:a aac -f mpegts out.ts
```

Variants: HEVC + AC-3, 1080i, broken timestamps, a stream that drops mid-playback. Commit the
recipes, not the media.

### 13.4 M0 pass/fail

| Check | Pass | Fail → action |
|---|---|---|
| Video under UI, per platform | 60 fps, no tearing, controls clickable | Separate video window on that platform; re-test |
| Fullscreen / Space switch | Video survives, stays under UI | Fix surface ordering before M1 |
| Clean machine, no mpv | Plays to first frame on all three OSes | Bundling broken — fix before M1 |
| Zap against reference server | Recorded; p50 < 2 s to proceed | Tune buffer/pre-warm; investigate server acquire time |
| Redirect channel | Key never reaches a non-server host | Fix §3.5 handling before M1 |
| Catch-up session | Plays and seeks | Investigate session URL/auth before M2 |
| HEVC/HDR sample | hwdec, correct colour | Document per-GPU; `hwdec=no` path |

If Linux fails on every compositor, Windows and macOS proceed and Linux follows with the
separate-window fallback. If all three fail, switch the shell to Flutter + media_kit; the core crates
are unchanged.
