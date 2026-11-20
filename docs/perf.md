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

### Core-side fixes (after the findings below)
- Programme search: FTS now yields only rowids (limit applied inside FTS, no bm25 over every match), then a primary-key probe, the time/visibility filter and a start-time order. `search_bench::programme_search_p95_under_30ms_on_672k_programmes` (release, 20k channels x 34 programmes, every keystroke of eight queries): p50 9.7 ms, p95 24.2 ms (was p95 188 ms).
- `get_guide`: one `channel_id IN (json_each(...))` range query for the whole page.
- First import: `task://progress` per channel page (`channels`, done = fetched, total = server count or 0) and per XMLTV batch (`guide`, done = programmes written, total 0); `catalog://updated` fires as soon as the channel list is committed, before the guide.

### Service-level search (the app path)
The app calls `Service::search`, not `catalog::search_programmes`: channel leg + programme leg + channel DTOs (logo, favourites) + scheduled flags. In the real app (release, 20k channels / 672k programmes) it measured p50 21 ms / p95 61 ms against the core bench's p95 24 ms. Causes found and fixed:
- Every channel logo was re-registered with an `INSERT OR IGNORE` write on every search, even when the in-memory `registered_logos` set already knew it (the `known` flag was computed but unused). Known logos now return their `dlimg://` URL with no DB access.
- `to_channel_dtos` ran twice per search (each reloads favourites and the source list): now once for both legs.
- `catalog::channels_by_ids` was one query per id: now one `json_each` IN query.
- The two legs ran one after the other: now `tokio::join!` on the 4-connection pool.

Method: `search_bench::service_search_p95_under_30ms_on_672k_programmes` (ignored; fixture channels carry logos; 61 keystroke queries). Debug profile, same machine, before then after: p50 20.6 to 16.9 ms, p95 59.1 to 47.3 ms. The debug programme leg alone is p95 about 46 ms, so the Service path now costs about the same as its dominant leg; release should land near the core bench's 24 ms and still needs a release re-measure when disk allows. Run: `cargo test -p downlink-core --test search_bench -- --ignored --nocapture service_search` (add `--release` for the real number; `SEARCH_BUDGET_MS` overrides the 30 ms assert, which debug exceeds).
Not done: a separate read-only pool (readers and writers share the pool; the bench has no concurrent writes).

### Core-side findings (original, kept for the record)
- Unified `search` programme leg (FTS5 over 672k programmes) is the one budget miss: p95 188 ms. Candidates: a prefix-only query for short inputs, a lower per-keystroke limit, or a cap on the FTS candidate set before the join with visible channels.
- `get_guide` scroll chunks p95 43 ms are fine, but they query per channel (`epg::window` loops over channel ids); one query with `channel_id IN (...)` would cut it.
- First import is a single 58 s step with no incremental visibility; chunked commits would let the list appear earlier.

### Resume last channel
Setting `resumeLastChannel` (boolean, default off, stored with the other machine settings; the UI has no toggle yet) plays
the most recently watched channel as soon as the engine is up. Never blocks the UI.

### Re-measurement after "window first" start-up and core's search/guide work

Start-up is now two background stages (`apps/desktop/src/boot.rs`): `setup` only creates the state and registers deep
links, then returns; service + logging (stage 1) and libmpv, engine pre-warm, surface, tray and media session (stage 2)
run afterwards and every command waits on the stage it needs (`state.rs`). Release build, same 20k-channel / 672k-programme
library, three runs on the warm cache:

| Budget | Result | Verdict |
|---|---|---|
| Start -> interactive (cached) < 800 ms | `setup` returns at 0.25-0.28 s, service and engine ready at 0.26-0.30 s, webview navigation starts at 0.48-0.52 s, `#root` has content 235-260 ms later: **about 0.72-0.78 s** after process start (was ~0.90 s) | pass, with little margin on this VM |
| Guide first chunk < 300 ms | 16 ms cold, 15 ms warm p50; scrolling all channels p50 22 ms, p95 35 ms | pass |
| Section switch < 100 ms | p50 13 ms, p95 19 ms | pass |
| Search keystroke < 30 ms | channel list p95 19 ms; unified search (with core's faster programme search) p50 21 ms, p95 61 ms, max 89 ms (was p95 188 ms) | channel search passes; unified search p95 still above 30 ms |
| RSS < 400 MB | app process 258 MiB (was 319 MiB); process tree with WebKit helpers 587 MiB | pass |

The first launch after a core schema migration is slower (core ready at 1.1 s in the run that migrated the old cache);
that cost is paid once. The remaining ~0.25 s before `setup` runs and ~0.2 s between `setup` returning and the webview
navigating are Tauri/GTK/WebKit initialisation.
