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

Interactive / FCP / LCP. "Before" = everything in one 923 kB entry chunk; "after" = route-level splitting with
the router's own `lazy` (final state).

| Route | Cache | Before | After |
|---|---|---|---|
| `/` Home | cold | 301 / 268 / 384 | 252 / 224 / 516 |
| `/` Home | warm | 104 / 92 / 376 | 111 / 96 / 384 |
| `/channels` | cold | 329 / 296 / 600 | 272 / 244 / 548 |
| `/channels` | warm | 117 / 112 / 436 | 139 / 136 / 456 |
| `/guide` (direct landing) | cold | 378 / 304 / 596 | 314 / 280 / 584 |
| `/guide` (direct landing) | warm | 214 / 176 / 428 | 159 / 136 / 424 |

Lesson learned: the first split used `React.lazy` + `<Suspense>`. Landing on a lazy route then took 683 ms cold
and 525 ms warm even though the chunk arrived 10 ms after the entry: React holds back revealing a resolved
Suspense boundary for about 300 ms after it committed the fallback. Routes now use react-router's `lazy`, which
fetches the chunk before rendering the route, so there is no fallback and no throttle. (A `modulepreload` /
early `import()` did not help for the same reason.) Client-side navigation to a not-yet-loaded route waits for
the chunk instead of showing a skeleton; idle preloading makes that rare.

## Bundle

| | Before | After |
|---|---|---|
| Entry JS (`index-*.js`) | 923 kB (284 kB gzip) | 487 kB (152 kB gzip) |
| Initial chunks | entry | entry + `time-*` (dayjs, 190 kB, 63 kB gzip) |

Split per route with react-router `lazy`: Guide, Movies/Series browse, VOD detail, Recordings, Settings, Search,
Player. The command palette (`@mantine/spotlight`) and the channel detail drawer are code-split and mounted
on idle (an early Ctrl/Cmd+K is queued and replayed). Shell, Home, Channels and Add source stay in the
initial chunk. After the first list is up, Player and Guide are preloaded on idle, then the rest.

`pnpm -C apps/ui analyze` writes `dist/stats.html` (treemap with gzip sizes).
