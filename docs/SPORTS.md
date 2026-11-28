# Sports tracking — spec

Users pick the sports, teams and racing series they follow. Downlink finds matching broadcasts in
local guide data and channel names. This spec covers the shared foundation: the catalog, the
settings page and the matcher. A home-page row and alerts come later and read the same matches.

## 1. Scope

- **Team sports:** pick a league, then optionally pick teams. A league with no teams picked means
  "every game in this league".
- **Motorsport:** pick a series, then the session types to follow. No teams or drivers.
- **Live only by default.** A global "include replays" setting widens it.
- **Local only.** Matching reads SQLite (`programme`, `programme_fts`, `channel`, `channel_group`).
  No outside schedule APIs, and no stream endpoint is ever touched (hard rule 4).
- **Categories are a bonus.** Most guide data has no XMLTV `<category>`. When it's present it
  raises the score. It's never required.

## 2. Catalog

The catalog is a data file embedded in `downlink-core` (`src/sports/catalog.toml`, loaded with
`include_str!`). Updating rosters, promotions/relegations or series renames is a data change, not a
code change.

The header comment in `catalog.toml` is the format reference. In short:

- **League:** `key`, `name`, `kind` (team | motorsport), `group` (motorsport), `tokens` (strong),
  `context` (risky tokens that need league context), `events` (marquee aliases, strong),
  `exclude` (veto phrases, also used to hand off to a more specific series).
- **Team:** `key`, `name` (strong), `short`, `abbr` (versus patterns only), `city` (weak),
  `aliases` (strong), `weak`.
- **`[sessions]`:** the session term lists. **`[words]`:** the replay, talk, sports-group and
  category word lists.

Strong aliases are unique enough on their own ("49ers", "Tottenham Hotspur"). Weak ones are a
city, or a nickname shared across leagues ("Giants", "Kings"). A `short` nickname found in more
than one league is made weak automatically at load time. §5 says how
each kind scores. Logos aren't shipped; the UI uses abbreviations on a neutral chip.

### 2.1 Team sports

| League | Key | Teams |
|---|---|---|
| NFL | `nfl` | 32 |
| MLB | `mlb` | 30 (Athletics have no city name) |
| NBA | `nba` | 30 |
| NHL | `nhl` | 32 (Utah Mammoth) |
| MLS | `mls` | 30 |
| Premier League | `epl` | 20 |

**MLS 2026.** East: Atlanta United, Charlotte FC, Chicago Fire, FC Cincinnati, Columbus Crew,
D.C. United, Inter Miami, CF Montréal, Nashville SC, New England Revolution, New York City FC,
New York Red Bulls, Orlando City, Philadelphia Union, Toronto FC. West: Austin FC, Colorado Rapids,
FC Dallas, Houston Dynamo, LA Galaxy, LAFC, Minnesota United, Portland Timbers, Real Salt Lake,
San Diego FC, San Jose Earthquakes, Seattle Sounders, Sporting Kansas City, St. Louis City SC,
Vancouver Whitecaps.

**Premier League 2026–27.** Arsenal, Aston Villa, Bournemouth, Brentford, Brighton & Hove Albion,
Chelsea, Coventry City, Crystal Palace, Everton, Fulham, Hull City, Ipswich Town, Leeds United,
Liverpool, Manchester City, Manchester United, Newcastle United, Nottingham Forest, Sunderland,
Tottenham Hotspur. The roster gets updated each summer after promotion and relegation.

NFL, MLB, NBA and NHL use the current franchise lists.

**Shared nicknames (always weak):** Giants (NFL/MLB), Cardinals (NFL/MLB), Rangers (MLB/NHL),
Kings (NBA/NHL), Panthers (NFL/NHL), Jets (NFL/NHL), Athletics, City and United (soccer), and
every city name (e.g. "New York", "Los Angeles", "Chicago", "Manchester").

### 2.2 Motorsport

| Group | Series (key) |
|---|---|
| Open wheel | Formula 1 (`f1`), F2 (`f2`), F3 (`f3`), F1 Academy (`f1_academy`), IndyCar (`indycar`), Indy NXT (`indy_nxt`), Formula E (`formula_e`), Super Formula (`super_formula`) |
| Stock car | NASCAR Cup (`nascar_cup`), NASCAR O'Reilly Auto Parts Series (`nascar_oreilly`), NASCAR Craftsman Truck Series (`nascar_truck`), ARCA (`arca`) |
| Sports car, GT and touring | WEC (`wec`), IMSA (`imsa`), Australian Supercars (`supercars`), BTCC (`btcc`), DTM (`dtm`), GT World Challenge (`gtwc`), Super GT (`super_gt`) |
| Motorcycles | MotoGP (`motogp`), Moto2 (`moto2`), Moto3 (`moto3`), WorldSBK (`wsbk`), AMA Supercross (`supercross`), Pro Motocross (`pro_motocross`), MotoAmerica (`motoamerica`) |
| Rally and off-road | WRC (`wrc`), Dakar (`dakar`), Nitro Rallycross (`nrx`), Extreme H (`extreme_h`) |
| Drag and dirt | NHRA (`nhra`), World of Outlaws (`woo`), High Limit Racing (`high_limit`) |

Marquee events are aliases under their series, not separate entries: Le Mans → WEC; Rolex 24 and
Petit Le Mans → IMSA; Indy 500 → IndyCar; Daytona 500 and Coca-Cola 600 → NASCAR Cup;
Bathurst 1000 → Supercars; Monaco GP → F1.

Former names stay in the catalog as aliases (Xfinity → O'Reilly, Extreme E → Extreme H).

**Tokens that are risky alone** need league context (the series name in any field, including the
channel name, or a matching event alias; a generic sports group or category is not enough): "Cup", "Indy", "F2", "F3", "GT", "Final", "Truck", "Superbike".

### 2.3 Session types (motorsport)

Each followed series has its own checkboxes. Race, qualifying and practice are on by default;
shows are off.

| Session | Terms |
|---|---|
| Race | Race, Grand Prix, GP, Sprint, Sprint Race, Feature, Feature Race, Main Event, Heat, Heat Race, "24 Hours", "6 Hours", "12 Hours", "Hours of", Duel, Final, and number names ("500", "400", "Daytona 500", "Coca-Cola 600") |
| Qualifying | Qualifying, Quali, Q1, Q2, Q3, Sprint Qualifying, Sprint Shootout, Pole, Pole Day, Pole Qualifying, Time Trials, Hyperpole, Superpole, Top 10 Shootout, Last Chance Qualifier, LCQ |
| Practice | Practice, Free Practice, FP1, FP2, FP3, P1, P2, P3, Warm-up, Happy Hour, Carb Day, Shakedown, Test, Testing |
| Shows | Pre-race, Pre-Race Show, Countdown, Grid Walk, The Grid, Post-race, Hub, Race Hub, Notebook, Highlights, Review, Magazine, Inside, Raceday, Preview, Weekly |
| Neutral | Grand Final, Grand Finale, Season Final, Season Finale: event names, never a session; terms inside them ("Final") don't count |

Terms are checked in this order: shows, qualifying, practice, race. That stops "Sprint Qualifying"
landing in race and "Pre-race" landing in race. A title that matches a series but has no session
term counts as **race** when the programme is flagged `LIVE`, and is otherwise left out.

Stored matches record the rules that built them (`sports.rules` in `app_kv`: the catalog text hash
plus `matcher::MATCHER_VERSION`). At startup a mismatch triggers a background rebuild, so an update
that changes aliases or scoring takes effect at once, not at the next guide refresh. Bump
`MATCHER_VERSION` with any scoring or session change.

## 3. Settings

A new **Sports** section on the settings page, placed after Playback.

- **Leagues.** One row per league with a switch. Turning a team league on expands a team picker:
  a searchable chip grid of abbreviation + name, plus "All games" (the default when nothing is
  picked).
- **Motorsport.** Grouped by §2.2 group. Each series has a switch. A series that's on shows
  session checkboxes (race, qualifying, practice, shows).
- **Include replays.** Off by default.
- **Preview.** "In your guide now": the next 24 hours of matches for the current selections, with
  the matched team or series, the session, live or replay, the channel and the start time. It
  exists for tuning, and phase 2's home row reuses the list component.

Selections are stored as one JSON document in `app_kv` under `sports.follow`:

```json
{
  "version": 1,
  "includeReplays": false,
  "leagues": { "nfl": { "teams": ["chi", "gb"] }, "epl": { "teams": [] } },
  "series": { "f1": { "sessions": ["race", "qualifying", "practice"] } }
}
```

Unknown keys (a team or series removed from the catalog) are kept but ignored, so a catalog update
never silently wipes a user's picks.

## 4. Data flow

```
catalog.toml ──┐
sports.follow ─┼─▶ sports::matcher ──▶ sports_match table ──▶ commands ──▶ UI (settings preview,
programme/fts ─┤    (after EPG sync,                                        home row, alerts)
channel names ─┘     on settings change)
```

- `downlink-core/src/sports/`: `catalog.rs` (parse and validate the TOML, unit-tested for
  duplicate keys and alias collisions), `matcher.rs`, `store.rs`.
- Matching runs after every EPG sync (the same hook that emits `catalog://updated`) and when
  `sports.follow` changes. It covers now through +7 days and replaces the rows in `sports_match`.
- Candidate rows come from `programme_fts`, using a single OR query built from every token and
  strong alias of the followed leagues and series. Scoring (§5) runs in Rust on those candidates.
- Hidden, adult and DRM channels are filtered in `catalog/` (hard rule 10). The matcher calls the
  existing catalog filters instead of reimplementing them.

```sql
CREATE TABLE sports_match (
  channel_id  INTEGER NOT NULL REFERENCES channel(id) ON DELETE CASCADE,
  start_utc   INTEGER,               -- NULL for channel-name-only matches
  end_utc     INTEGER,
  league      TEXT NOT NULL,         -- catalog key
  teams       TEXT NOT NULL DEFAULT '',  -- comma-separated team keys that matched (followed or not)
  session     TEXT,                  -- motorsport only: race | qualifying | practice | shows
  live        INTEGER NOT NULL,      -- 1 live, 0 replay or unknown
  score       INTEGER NOT NULL,
  source      TEXT NOT NULL,         -- programme | channel
  PRIMARY KEY (channel_id, league, start_utc)
);
CREATE INDEX sports_match_time_idx ON sports_match (start_utc);
```

Commands (typed DTOs, no HTTP in the UI):

- `sports_catalog() -> SportsCatalogDto`: leagues, teams, series and session types.
- `sports_follow_get() / sports_follow_set(SportsFollowDto)`: setting it triggers a re-match.
- `sports_matches(window_start, window_end) -> Vec<SportsMatchDto>`: for the preview and,
  later, the home page.

## 5. Matching and scoring

The matcher builds a normalized haystack per candidate: title, subtitle, description, channel name
and group name. Normalizing means lowercase, no diacritics, "&" → "and", and punctuation collapsed
to spaces. Tokens match on word boundaries only ("F1" must not match "F150").

| Signal | Points |
|---|---|
| League or series token in title | +40 |
| League or series token in subtitle, description or channel name | +25 |
| Marquee event alias | +40 |
| Strong team alias | +30 per team, max 2 teams |
| Weak team alias, only with league context already ≥ 25 | +10 per team |
| Versus pattern between two team aliases (`at`, `vs`, `v`, `@`) | +20 |
| Sports category present (Sports, Sports event, Football, Motorsport, …) | +15 |
| Channel group name contains a sports word (sports, racing, motorsport, NFL, …) | +10 |
| `LIVE` flag | +10 |
| `NEW` or `PREMIERE` flag | +5 |
| League `exclude` phrase | −100 |
| Replay words (Classic, Replay, Encore, Rewind, "Greatest Games", "from 19xx/20xx") | −40, and mark as a replay |
| Talk or magazine words (Live from, Tonight, Countdown, Postgame, Pregame, Insider, Report, Draft) | −30 |

- **Threshold:** 50. Below that the candidate is dropped.
- **Team leagues:** a match counts when the league score passes and, if teams are picked, at least
  one followed team matched. A league-only match ("NFL Football", teams unknown) counts only when
  the user follows "All games".
- **Live:** the `LIVE` flag sets live. A guide that never sets flags (detected per source: no
  `LIVE` flag anywhere in the window) falls back to "live unless it has replay words". Those
  matches show "likely live" instead of "live". A source that *does* set flags is trusted: no
  flag means replay.
- **Channel-name matches:** event channels (e.g. `NFL 03: Bears vs Packers 1:00 PM`) are scored
  with the same rules applied to the channel name. They're stored with `start_utc = NULL` and
  shown as "On now / today". If the name includes a time, it's parsed in the source's time zone
  into `start_utc`, best effort.
- **Ties across leagues:** when one programme scores for two leagues, the higher score wins. An
  exact tie keeps both. This happens rarely and is visible in the preview.

Every scoring rule has a fixture test using invented titles in real formats, with no real provider
data (hard rule 16). That includes the shared-nickname cases (Giants, Cardinals, Rangers, Kings),
motorsport session classification for every term in §2.3, and replay and talk-show rejection.

### 5.1 One event, many channels

`sports_matches` groups copies of the same event (`sports::group`), so a game on three channels is
one match, and later one alert:

- **Same event:** same league and session, and the airtimes overlap (no fixed minute windows:
  copies of one event are on at the same time, even when one channel opens with its own build-up).
  Teams must overlap; when both name two or more teams, the sets must be equal. Teamless listings
  need equal normalized titles. An event channel joins the airing its named start falls in, or
  the one on air now when its name has no time.
- **Tape delays (motorsport):** a later, non-overlapping airing of a series session the guide
  marks live, starting within 12 hours of an earlier live airing of that session, is a
  broadcaster's delay and folds away. Once the original ends and leaves the list, the delayed
  airing shows on its own. Sessions a day apart (sprint vs. main qualifying) stay separate.
- **Which channel leads:** favorite, then quality (4K > FHD > HD > unknown > SD), then guide data
  over a name-only match, then score, then the lowest channel number.
- The rest go in `alternates` (best first). The card shows "+N"; its menu ("Also on") plays any
  of them.

## 6. Phases

1. **Foundation:** catalog, `sports.follow`, matcher, the `sports_match` table, commands, and the
   Sports settings section with preview.
2. **Home row:** "Your sports" on the home page, showing live now, then the next 48 hours, grouped
   by team or series. Selecting an item plays the channel.
3. **Alerts:** notifications N minutes before a followed match starts (default 10) and when a
   followed match goes live. These fire only while the app is running, via the Tauri notification
   plugin.
4. **Command center:** every followed live game on one multiview wall, ranked by the user, with a
   rail of what else is live and what is next. See `docs/COMMAND_CENTER.md`. Adds `priority` (an
   ordered list of `league:team` and league/series keys) to `sports.follow`; documents without it
   load with an empty ranking.
5. **Later:** college football, more leagues, optional outside schedule data to confirm start
   times, auto-record rules built on top of matches.

## 7. Out of scope

Following drivers or players, scores and results, spoiler hiding, and fetching team logos.

## 8. Implementation notes (phase 1 + 2)

Where the build refines §2–§5:

- The catalog has 39 leagues: 6 team leagues and 33 motorsport series.
- **Live markers.** Many guides mark live airings in the title instead of the XMLTV flag: a
  superscript "ᴸᶦᵛᵉ" suffix, "(Live)", "[Live]", a "LIVE:" prefix. A source whose sports listings
  carry a marker (or a LIVE flag) is trusted: an unmarked airing is a replay. Sources with no
  markers fall back to "likely live", and later repeats of the same title on the same channel in
  the window are replays.
- **The listing must name the sport.** A programme matches only when its title or subtitle names
  the league, an event or a team. A team channel's name or a description adds confidence but
  never makes a match on its own ("No Game Today" on a team channel).
- **Filler and feeds** (`[words] filler`, `feeds`): placeholder listings ("No Game Today",
  "NO EVENT") never match; event-channel secondary feeds (on-board cameras, timing, trackers)
  are dropped.
- **Motorsport session words** add +15 next to the series name. A bare series title with no
  session word counts as the race only when the guide marks it live, never on inference.
- **Event channels** read a date in the name ("@ 27 Nov 11:50 AM ET"); one whose named start
  is more than 4 hours past is over and dropped; a future one shows its start time
  (`eventStartUtc`).
- **Live vs. on air.** `live` on a match means "a live broadcast, not a replay", not "on air
  now". The UI works out "on air" from programme times (channel-only matches count as on air) and
  sorts on air first. The live badge shows only while a match is on air.
- **Replay words** are checked in title and subtitle only. A replay word inside the league's own
  token or event alias is ignored ("Winter Classic" is live).
- **Session classification:** a term inside a longer matched term doesn't count; then the order
  shows → qualifying → practice → race decides. So "Superpole Race" is race and "Grand Prix
  Countdown" is shows. The talk penalty is skipped for titles classified as shows.
- **Cross-league:** every league is scored and the best wins (exact ties keep both) before follow
  filtering. That's how "F1 Academy" goes to `f1_academy`, not `f1`.
- **Teams** match in title and subtitle, and in the description only when neither names one. The
  cap is 2 teams in total. `abbr` counts only inside a versus pattern with league context.
- **Channel-name matches:** a team league needs a team in the name ("NFL Network" alone is not a
  match). Motorsport needs a session term or an event. A channel that already has a programme
  match for the league gets no channel row.
- **Follow rules:** a series with an empty session list shows nothing. Picked teams no longer in
  the catalog don't widen to "All games".
- **Rebuild triggers:** full sync, guide refresh (visible or single), content-filter change and
  `sports_follow_set`. A mutex keeps it to one rebuild at a time. Each rebuild emits
  `catalog://updated` with scope `sports`.
- **Categories:** the +15 category signal exists in the matcher, but categories aren't stored
  yet, so it's inert until a migration adds `programme.category`.
