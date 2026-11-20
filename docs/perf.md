# Performance notes

## Zap time (PLAN §4.6), measured through the real command path

Driver: `DOWNLINK_E2E=zap` (feature `e2e`, `apps/desktop/src/e2e.rs`) calls `Playback::play_channel`, i.e. the same
code as the `play_channel` command, then waits for the player's first frame. The pre-warmed mpv instance is reused;
the zap path awaits only the play plan (SQLite) before `loadfile` (the programme lookup runs after it).

Machine: Debian 13 VM, software GL (llvmpipe), no GPU, libmpv 0.40.0, local test Dispatcharr 0.34.1 (docker) with
two channels kept warm by holding a client open on each (`curl -N`), 20 alternating zaps, 1.5 s dwell.

| Phase | p50 | p95 |
|---|---|---|
| Call -> `loadfile` issued (play plan from SQLite, source caps) | 4 ms | 10 ms |
| `loadfile` -> first frame (H.264/HEVC 1080p30, local server) | 356 ms | 867 ms |

20 of 20 zaps reached Playing. Targets: p50 < 1.2 s, p95 < 2.5 s. This is the warm-channel number for the local
synthetic server, not the reference server.

Cold channels are dominated by the server: Dispatcharr takes about 5.1 s to first byte when it has to start a channel
(`curl` TTFB), which gave p50 5.5 s earlier in M1 verification. Redirect-profile channels (the server only answers a 302)
started in 0.57-0.86 s. Direct HTTP TS from the local stream server: p50 251 ms, p95 389 ms (see `docs/M0-go-no-go.md`).

## Network discipline

`crates/downlink-core/tests/network_discipline.rs`: 60 s of simulated guide browsing at 10 focus changes per second
(now/next, play plan and logo prefetch per row, guide windows, groups, search, home) makes zero requests to stream
endpoints and no API requests after the initial sync (only logo fetches).

## PLAN §8.8 budgets: harness and results

Harness: `apps/desktop/src/e2e.rs` (feature `e2e`), release build (`cargo build --release -p downlink-desktop
--features e2e,custom-protocol`, embedded UI from `apps/ui/dist`).

Data: `tools/fixtures/gen-large-m3u.py 20000` plus `tools/fixtures/gen-large-xmltv.py 4000 7` (4,000 channels x 7 days
of hourly programmes = 672,000 programmes, ~137 MB of XML; both outputs are gitignored). Imported as an M3U source
from local files with `DOWNLINK_E2E=perf-setup`; measured in a fresh process on the warm SQLite cache with
`DOWNLINK_E2E=perf`. `DOWNLINK_E2E=launch` measures start to first frame with `resumeLastChannel` on.

Machine: Debian 13 VM, software GL, no GPU, no monitor attached (webview `visibilityState` hidden, so no paint
entries). Numbers are from this VM; a real desktop should be at least as fast.

| Budget | Result | Verdict |
|---|---|---|
| Start -> interactive (cached) < 800 ms | Rust side: setup done at 0.46 s, then the first-screen commands (sources, counts, groups, first 100 channels, home, first guide chunk) take 86 ms in total. Webview navigation starts 0.64 s after process start; `#root` first renders 262 ms after that, i.e. **about 0.90 s** after process start (no FCP: window hidden) | **misses by ~0.1 s on this VM**; 0.46 s of it is `setup` (SQLite open and migrations, libmpv load, surface attach, tray, media session), the rest is webview start |
| Guide open from cache (first chunk) < 300 ms | 22 ms cold, 18 ms p50 / 23 ms p95 warm (30 channels x 8 h = 240 programmes). Scrolling the whole 20k-channel list across days: 667 chunks, p50 27 ms, p95 43 ms, max 63 ms | pass |
| Section switch < 100 ms | commands behind each screen: p50 12 ms, p95 20 ms, max 22 ms | pass (command latency) |
| Search keystroke < 30 ms | channel list query on 20k channels: p50 10 ms, p95 18 ms, max 31 ms. **Unified `search` (channels + programme FTS over 672k programmes): p50 50 ms, p95 188 ms, max 382 ms** | channel search passes; **programme search misses** (core) |
| RSS (20k channels, 7 d guide) < 400 MB, video excluded | app process 319 MiB after the whole browsing run (378 MiB right after the import). Process tree including the WebKit helper processes: 653 MiB | pass for the app process; the tree figure includes the webview |
| Start -> first frame (last channel) < 3 s | with `resumeLastChannel` on and the channel warm on the local Dispatcharr: `loadfile` issued 0.49 s after start, first frame 0.61 s later: **1.10 s** | pass (cold channels add the server's ~5 s start-up) |
| Zap | see above: p50 356 ms, p95 867 ms warm | pass |
| List / guide scroll 60 fps | not measured here (needs a display) | pending |

Initial import of the 20k-channel playlist and 672k-programme guide: 58 s.

### Core-side findings
- Unified `search` programme leg (FTS5 over 672k programmes) is the one budget miss: p95 188 ms. Candidates: a prefix-only query for short inputs, a lower per-keystroke limit, or a cap on the FTS candidate set before the join with visible channels.
- `get_guide` scroll chunks p95 43 ms are fine, but they query per channel (`epg::window` loops over channel ids); one query with `channel_id IN (...)` would cut it.
- First import is a single 58 s step with no incremental visibility; chunked commits would let the list appear earlier.

### Resume last channel
Setting `resumeLastChannel` (boolean, default off, stored with the other machine settings; the UI has no toggle yet) plays
the most recently watched channel as soon as the engine is up. Never blocks the UI.
