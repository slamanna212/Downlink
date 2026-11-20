# UI startup performance (PLAN §8.8: start to interactive < 800 ms, cached)

"Interactive" is `performance.mark('dl:interactive')`, set when the first list (Home, Channels or Guide)
renders with data. The UI also reports it to the shell with `report_ui_timing {name:'dl:interactive', ms}`
(ms since navigation start) so the desktop perf harness can read it.

## How measured

Headless Chromium (Playwright), production build (`pnpm build`) served by `vite preview`, mock backend,
1440x900, no throttling. Median of 7 fresh browser contexts. "Cold" = empty cache; "warm" = reload with the
HTTP cache. Real numbers in the Tauri webview differ (assets are local, no network, a different GPU path);
treat this as a regression guard, not the budget proof. Script: see git history (`startup.mjs`, scratch).

## Results (ms)

| Route | Cache | Before: interactive / FCP / LCP | After: interactive / FCP / LCP |
|---|---|---|---|
| `/` Home | cold | 301 / 268 / 384 | 248 / 220 / 280 |
| `/` Home | warm | 104 / 92 / 376 | 98 / 84 / 372 |
| `/channels` | cold | 329 / 296 / 600 | 241 / 236 / 564 |
| `/channels` | warm | 117 / 112 / 436 | 131 / 128 / 440 |
| `/guide` | cold | 378 / 304 / 596 | 683 / 208 / 820 |
| `/guide` | warm | 214 / 176 / 428 | 525 / 100 / 652 |

Landing directly on a lazy route (the guide) now pays a chunk waterfall: the route chunk plus the shared
Mantine chunks it needs. From Home or Channels the chunk is already preloaded on idle, so the guide opens
without it. If the app ever starts on the guide, move it back into the initial chunk or `modulepreload` it.

## Bundle

| | Before | After |
|---|---|---|
| Entry JS (`index-*.js`) | 923 kB (284 kB gzip) | 487 kB (152 kB gzip) |
| Initial chunks | entry | entry + `time-*` (dayjs, 190 kB, 63 kB gzip) |

Split per route with `React.lazy`: Guide, Movies/Series browse, VOD detail, Recordings, Settings, Search,
Player. The command palette (`@mantine/spotlight`) and the channel detail drawer are code-split and mounted
on idle (an early Ctrl/Cmd+K is queued and replayed). Shell, Home, Channels and Add source stay in the
initial chunk. After the first list is up, Player and Guide are preloaded on idle, then the rest.

`pnpm -C apps/ui analyze` writes `dist/stats.html` (treemap with gzip sizes).
