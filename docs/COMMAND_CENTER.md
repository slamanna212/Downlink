# Command center — spec

A command center is a multiview wall plus a side rail: the live things you care about on screen at
once, with a list of what else is on and what is next. "My sports" (the sports command center) is
built in; saved command centers (§6) are walls the user builds by hand. News feed panels are §7.

## 1. Concept

One engine, two kinds of tile source:

| | My sports (built in) | Saved (§6) |
|---|---|---|
| Tiles | Picked by a **rule**: followed games on air now | Picked **by hand**: a fixed channel list |
| Rail | On screen, Also live, Up next | On screen, Not on screen, Headlines (phase 3) |

A command center is hosted by the multiview screen (`MultiviewScreen` in
`apps/ui/src/features/multiview/MultiviewPage.tsx`) with a rail on the right. Every multiview rule
applies unchanged (PLAN §4.11): one mpv instance per tile, slot 0 is the main player, one audible tile,
the stream budget, and every tile stops on exit (rule 6).

## 2. Entry

- **Command center** in the left nav (after Recordings, always shown) opens `/command`, a regular
  app page (not a video route: nav rail and opaque background). It shows a grid of cards: **My sports**
  always first, then one card per saved center (§6). Nothing streams from this page.
- My sports: route `/multiview/sports` (under `/multiview`, so the shell treats it as a video route).
  Its card says how many followed games are live; with nothing followed it says "Choose sports to fill
  it" and opens Settings → Sports instead. Also the "Command center" link on the home "Your sports"
  row and "My sports" in the command palette.
- Saved centers: route `/multiview/c/:id`, from their card or a "Command center: <name>" palette entry.
- Opening a center **plays it straight away**. Navigating to it is an explicit play (rule 4). Nothing
  is requested before the user opens it, and the rail browses SQLite only.

## 3. Filling the wall

The practical cap is usually **4** tiles: 9 needs hardware decoding *and* a stream limit of at
least 9. So more live games than tiles is the common case, not an edge case.

`features/command/assign.ts` (pure, unit-tested) turns followed matches + the wall + the user's
intent into the fewest tile changes:

- **Candidates:** followed matches on air now, one per channel (copies of an event are already
  grouped by `sports::group`). Order: live broadcasts before replays, then the user's **rank**,
  then confirmed live before "likely live", then match score.
- **Rank:** `sports.follow.priority`, an ordered list of `league:team` keys and league/series keys
  (for "all games" and motorsport). A match takes its best entry. Anything followed but unranked
  comes after, in catalog order. Edited in Settings → Sports → "Command center order" (up/down
  buttons). The rail's "Rank" button opens it.
- **Keep:** pinned tiles, failed tiles (the stream-limit tile has its own actions), and any tile
  whose game is still live. Games are **never reshuffled**: a better game starting later goes to
  "Also live" with a swap action.
- **Advance:** an unpinned tile whose game ended is reused for the next game; with nothing left it
  stops (the main player is never stopped, it keeps its last channel).
- **Fill:** free slots in slot order (main player first), up to the capacity.
- A tile on a channel counts as showing that channel's own match first, else the match that lists it as
  an alternate.

**User intent (per visit, not saved):**

| Action | Effect |
|---|---|
| Replace from a tile's picker, add from the toolbar, swap / add from the rail | Tile is **pinned** (auto-fill leaves it alone) |
| Pin toggle on a rail row | Pins or unpins the tile |
| Remove a tile (tile action, Del, rail, unchecking in the add picker) | The channel is **dismissed** (never auto-added again this visit), and the capacity drops by one |
| Add a tile by hand | Capacity goes back up by one |

Capacity is `max(1, budget − tiles removed + tiles added)`. A channel whose play fails is dismissed so
it isn't retried in a loop.

Assignments in flight (sent, not yet in `multiview://state`) are tracked, and a slot with one in flight
is left alone until it lands (15 s timeout), so a slow state event never starts a tile twice.

## 4. Rail

`dl.layout.commandRail` (320 px) on the right, on `--dl-surface-1`. Toggle with **R** or the toolbar
button. The grid and toolbar sit to its left (`sideWidth` on `MultiviewScreen`). Sections:

- **On screen:** one row per tile, the focused tile marked with an accent bar. Select focuses the tile.
  Actions: listen, pin/unpin, remove.
- **Also live:** overflow, best first. Select or swap puts it in the focused tile; add (+) appears while
  a slot and stream are free.
- **Up next:** the next 48 hours (up to 24), not on air yet. Select opens the programme details.
  A game that starts joins on its own if there is room.

Rows (`RailRow`) lead with the programme subtitle when there is one ("Arsenal v Chelsea"), then
team/series · session · time, and a live / likely live / replay badge. Actions show on hover or
keyboard focus.

## 5. Not following anything

The rail shows "Follow teams and series…" with a **Choose sports** button. No stream is started.

## 6. Saved command centers

A wall the user built by hand in multiview, saved under a name and opened again in one go.

### 6.1 Storage (migration 0025)

- `command_center(id, name, layout_style grid | featured, audible slot, rail_json, created_utc,
  updated_utc)` and `command_center_tile(center_id → command_center ON DELETE CASCADE, slot 0..8,
  channel_id → channel ON DELETE CASCADE, PK(center_id, slot))`. My sports is built in, not a row.
- **Channels across resyncs:** tiles keep the channel row id. Channel ids are stable across a resync
  (channels upsert on source + ext id, like favorites and recents); a channel that leaves the source
  takes its tile with it (cascade). Hidden channels (adult/radio/DRM filters, `catalog/`) are left out
  when a center is loaded or counted, and come back when the filter allows them again.
- `rail_json` is an opaque JSON object the UI owns, at most 64 KiB. Phase 3 stores the center's feeds
  there as `{ "feeds": [{ "id": <feed id> }] }`; unknown keys are kept on save.
- Validation (core, `command::validate`): name trimmed, 1–80 characters; layout `grid` or `featured`;
  audible slot 0..8; at most 9 tiles on unique slots 0..8; rail is an object within the size cap. A
  tile on a channel id that does not exist is skipped.

### 6.2 Commands

| Command | Args | Result |
|---|---|---|
| `command_centers_list` | – | `CommandCenterSummary[]` (`id, name, tileCount, channelIds` (≤ 4, for the card preview)`, updatedUtc`), newest change first |
| `command_center_get` | `id` | `CommandCenter` (`id, name, layoutStyle, audible, rail, tiles: [{ slot, channel }], createdUtc, updatedUtc`) |
| `command_center_save` | `center: { id \| null, name, layoutStyle, audible, rail, tiles: [{ slot, channelId }] }` | id (create when `id` is null, else replace) |
| `command_center_delete` | `id` | – (deleting a missing one is not an error) |

UI hooks: `useCommandCenters`, `useCommandCenter(id)`, `useSaveCommandCenter`,
`useDeleteCommandCenter` (`api/queries.ts`); channel changes (`catalog://updated`) refresh them.

### 6.3 Saving

- Plain multiview toolbar: **Save as command center** (disabled with nothing on screen) asks for a name
  and saves the tiles on screen (slot 0 = the main player's channel), the layout and the audible slot.
  The URL stays `/multiview` (moving would stop and restart every tile); the wall now counts as that
  center, so the toolbar offers **Save changes** (after a hand edit) and **Save as new**.
- In a saved center: hand edits (picker, add, remove, layout) show **Save changes**, which replaces
  the center with the tiles on screen plus its "Not on screen" channels in the free slots. **Save as
  new** saves a copy and moves the URL to it without restarting the wall.

### 6.4 Opening (`features/command/SavedCommandPage.tsx`, `saved.ts`)

- Waits for the center and the real tile cap (`loaded`), then once per center: the saved tiles in
  slot order go to slots 0, 1, 2… up to the budget (`min(maxTiles, stream limit)`); the rest are
  **Not on screen**. Tiles already showing the right channel are left alone; tiles of a center opened
  before (palette jump) that are not in the plan stop. The saved layout is applied and the audible
  tile (followed by channel) gets the sound once it lands. Assignments in flight are tracked so
  nothing starts twice; a tile that fails to start moves to Not on screen.
- A center refetch (rename, channel sync) never restarts the wall. Leaving stops every tile (rule 6).
- A missing center shows "This command center no longer exists." with a way back; nothing starts.

### 6.5 Rail

- Head: the center name and a menu: **Rename** (name dialog; renames only, unsaved wall edits stay
  unsaved) and **Delete** (confirm modal, then back to `/command`).
- **On screen:** one row per tile (focused tile marked). Actions: listen, remove (not the main player).
- **Not on screen:** the overflow. Select or swap puts it in the focused tile (the channel it replaces
  moves here); add (+) appears while a slot and stream are free.
- **Extension point for Phase 3:** `features/command/RailExtras.tsx` exports `RailExtras({ center, api })`
  (`RailExtrasProps`), rendered below the two sections; it renders nothing today. The Headlines
  section goes there, reading `center.rail.feeds`, built with `Section` (`RailParts.tsx`) and the rail
  styles. `SavedCommandPage` also takes an `extraSections` component prop (default `RailExtras`).

### 6.6 News on a saved center

A saved center can show news headlines from its feeds (§7): see §7.7.

## 7. News feeds

User-added RSS/Atom feeds, fetched and polled in Rust (`downlink-core/src/feeds/`). Feed fetches
are not stream endpoints (rule 4 does not apply). The UI never talks HTTP (rule 2); fixtures and tests
use invented feeds on `.example` hosts and a local mock server (rule 16).

### 7.1 Storage (migration 0024)

- `feed(id, url UNIQUE, secret_ref, title, site_url, etag, last_modified, last_fetch_utc,
  last_ok_utc, last_error, created_utc)`.
- `feed_item(feed_id → feed ON DELETE CASCADE, guid, title, link, summary, published_utc,
  fetched_utc, PK(feed_id, guid))`, indexed on `(feed_id, published_utc)`. UTC seconds throughout.
- Retention after every fetch: items older than 48 h (publish time, else first-seen time) are
  dropped, then only the newest 200 per feed are kept.
- Which feeds a center shows lives in its rail JSON (`rail.feeds: [{ id }]`, §6.1). Feeds are global
  so two centers can share one.

### 7.2 URLs and secrets (rules 7, 8)

- Input is trimmed, at most 2048 characters, `http`/`https` with a host; the fragment is dropped.
- A URL with userinfo (`user:pass@`) or a credential query parameter (`token`, `key`, `apikey`,
  `api_key`, `api-key`, `access_token`, `auth_token`, `private_token`, `client_secret`, `access_key`,
  `passkey`, `auth`, `secret`, `sig`, …, the `redact/` list) is **secret-bearing**: the full URL goes
  to the OS keychain (`downlink/feed/<uuid>/url`) and SQLite stores only the display form (no
  userinfo, credential values replaced by `***`) in `url`, with the keychain reference in
  `secret_ref`. Removing the feed deletes the keychain entry. The UI gets the display form and
  `hasSecret`; to change such a feed, remove it and add it again.
- Errors are URL-free (reqwest errors drop the URL) and additionally scrubbed of the feed's literal
  secrets, then redacted before they reach `last_error`, logs or the UI. Redaction tests cover the
  feed URL shapes (`redact/tests.rs`, `news_feed_url_shapes_redact`).

### 7.3 Fetching

- One GET with our User-Agent, no Referer, `If-None-Match` / `If-Modified-Since` from the last 200,
  15 s timeout, at most 5 redirects, body capped at 5 MiB (streamed). 304 only bumps
  `last_fetch_utc` / `last_ok_utc`. 401/403 are plain feed errors, never blocking app auth errors.
- Parsing: `feed-rs` (RSS 0.9x/1.0/2.0, Atom). Titles and summaries are HTML-stripped to one line
  of plain text (titles ≤ 300 characters, summaries ≤ 400 with an ellipsis). Links are http(s) only.
  Guid: the feed's id/guid, else the link, else a hash of title + publish time. Dates in the future
  are clamped to now; unparseable dates leave the item undated (sorted by first-seen time).
- `feed_add` fetches once before saving: a failing first fetch returns the redacted error and saves
  nothing, so the user can fix the address straight away.

### 7.4 Polling

- Only feeds a view watches are polled. A view calls `feeds_watch(key, feedIds)` (replacing what
  that key watched; `[]` stops) and `feeds_unwatch(key)` on close. Keys are view-chosen; the UI hook
  uses a fresh key per effect run so a late unwatch never cancels a newer watch.
- Interval: setting `feedRefreshMinutes` (default 3, clamped 1–60), read on each watch. A newly
  watched feed is fetched at once when never fetched or older than the interval.
- Errors back off: interval × 2 per consecutive failure, capped at 30 min, or longer when the server
  sends `Retry-After`. A success or manual refresh resets it. At most 4 fetches run at once.
- The poller starts with the app (idle until something is watched) and stops on quit.
- When a fetch adds or changes items, core emits `feeds://updated` with `{ feedIds: number[] }`.

### 7.5 Commands

| Command | Args | Result |
| --- | --- | --- |
| `feeds_list` | — | `Feed[]` (by title) |
| `feed_add` | `{ url }` | `Feed` |
| `feed_remove` | `{ id }` | `null` |
| `feed_items` | `{ feedIds, limit }` (1–500) | `FeedItem[]`, newest first |
| `feed_refresh` | `{ id }` | `Feed` |
| `feeds_watch` | `{ key, feedIds }` | `null` |
| `feeds_unwatch` | `{ key }` | `null` |

`Feed { id, url, hasSecret, title, siteUrl, lastFetchUtc, lastOkUtc, lastError, itemCount }`,
`FeedItem { feedId, guid, title, link, summary, publishedUtc }` (`publishedUtc` falls back to the
first-seen time). Item links open in the system browser through the existing `open_external_link`
command (http/https only).

### 7.6 UI hooks (`api/queries.ts`)

`useFeeds()`, `useFeedItems(feedIds, limit = 100)`, `useAddFeed()`, `useRemoveFeed()`,
`useRefreshFeed()`, and `useFeedsWatch(feedIds)`, called once by an open center: it watches on mount
and on change, unwatches on unmount, and invalidates feed queries on `feeds://updated`. The browser
mock (`ipc.mock.feeds.ts`) seeds three invented feeds (one failing) and adds a headline every 30 s
while watched; a host containing `fail` makes `feed_add` reject.

### 7.7 Command center UI (`features/command/feeds/`)

- **Rail document.** `rail.feeds: [{ id }]` (display order) and `rail.ticker: boolean` (absent = on).
  Unknown keys are kept. Changing feeds or the ticker saves the center right away as a *rail-only*
  save: the stored tiles go back unchanged, and hand edits on the wall are left for "Save changes"
  (`rail.ts`, `useCenterRail.ts`).
- **Watching.** `SavedCommandPage` calls `useFeedsWatch(feedIds)` while the center is open, whether
  the rail is shown or not, so the ticker keeps updating. Leaving unwatches; polling stops when no
  open center watches a feed.
- **Headlines section** (`RailExtras` → `HeadlinesSection`): below On screen / Not on screen. Newest 30
  across the center's feeds; each row is the headline (two lines at most), then source · age ("4 min
  ago", via `ageOf` in `time.ts`). Selecting opens the story in the system browser through
  `open_external_link`. A gear button opens the feed manager. With no feeds: one sentence and a
  "Manage feeds" button.
- **Ticker** (`HeadlineTicker`): a 36 px strip under the wall (`bottomInset` on `MultiviewScreen` keeps
  it clear of the video tiles and lifts the toolbar above it). A "Headlines" label, then the newest 12
  headlines crawling right to left. Hover or focus pauses it; reduced motion stops the crawl and leaves
  the strip scrollable. Shown when the center has feeds and it isn't switched off with the toolbar's
  news button.
- **Feed manager** (`FeedManager` modal): add a feed by address (it joins the feed library and this
  center), refresh or detach this center's feeds, attach a library feed, or delete a library feed
  everywhere. Each line shows the title, the display URL (a key icon when the address holds a secret
  kept in the keychain), and the headline count or the last (redacted) error.

