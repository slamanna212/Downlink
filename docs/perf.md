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
