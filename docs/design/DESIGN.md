# Downlink — design spec

Implements PLAN §8.5. Tokens and component styling live in `apps/ui/src/theme.ts`,
`apps/ui/src/styles/Button.module.css` and `apps/ui/src/styles/global.css`. Visual reference (open in a
browser): `docs/design/reference/`:
- `design-language-v1.html`: foundations (type, spacing, radius, density, motion, guide block states)
- `design-language-v2.html`: colour detail, approved text pairs, buttons, uniformity rules
- `screens.html`: 12 screens at 1440×900 (home, channels, guide, player live/catch-up/error, add source,
  recordings, settings, 10-foot, multiview 2×2, multiview 1+3 + picker)

The reference uses synthetic channel names. Where this file and the reference disagree, this file wins.

**Decided 2026-11-19:** accent = **Iris violet `#9D8CFF`**, neutrals = **cool blue-grey**.

---

## 1. Principles
1. The content is the interface. Chrome recedes; logos, artwork and video supply colour (~92% neutral UI).
2. One accent with one meaning: *focused, selected, or how far along*. Never decoration.
3. Status hues are reserved: red = live / recording / destructive, green = success, amber = degraded.
4. Elevation is a lighter surface, not a shadow.
5. Focus is always visible. Every screen works with keyboard and remote alone.

## 2. Colour

### Neutrals (`colors.dark`)
| step | hex | token | role |
|---|---|---|---|
| 0 | #E8ECF1 | --dl-text | titles, body, values |
| 1 | #C3CAD3 | --dl-text-2 | section headings, secondary values |
| 2 | #A3ADB9 | --dl-text-dim | metadata, times, help, inactive nav |
| 3 | #7C8692 | --dl-text-faint | ≥ 12 px caps labels, column headers only |
| – | #5F6873 | --dl-text-disabled | disabled, past-without-archive |
| 4 | #2C323A | --dl-border | selected segment, hard dividers |
| 5 | #20252B | --dl-surface-3 | active nav, menus, secondary button, logo plate |
| 6 | #191D22 | --dl-surface-2 | inputs, programme blocks, segmented track |
| – | #121519 | --dl-surface-1 | rail, panels, cards |
| 7 | #0C0E11 | --dl-surface-0 | app background, guide grid |
| 8 | #08090B | --dl-letterbox | video letterbox |
| 9 | #050607 | --dl-multiview-canvas | behind multiview tiles |

### Accent: Iris (`colors.iris`, primary)
`--dl-accent #9D8CFF` (focus ring, progress, selected indicator, primary fill) ·
`--dl-accent-hover #B5A8FF` · `--dl-accent-pressed #8571F5` · `--dl-accent-text #BDB2FF` (accent text/icons:
catch-up, playing row, links) · `--dl-on-accent #120A33` (text on accent fill, never elsewhere) ·
`--dl-accent-bar` (accent at 55%: info panel buffer graph bars only; a data mark, not a tint).

### Third-party brand
- Plex `--dl-plex` (`#E5A00D`): the Plex chevron mark on the player's open-in-Plex icon button (`IconPlex`). A
  brand colour exception to the accent rules; used nowhere else.

### Status
- Live `#F2495C`: dot, now line, REC dot. Text `#FF8B97` on 18% tint. Never solid red with white text.
- OK `#34D399` (text `#6EE7B7`): connected, test passed.
- Warn `#F5B544`: offline, stale guide, name-matched EPG, stream limit.
- Notice `--dl-notice` (`#9D8CFF`, the Iris value): the 8 px "update waiting" dot on the rail's Settings item and
  the Settings → Updates nav row. Its only use. Iris, not green, so it never reads as the source "connected"
  dot directly below it in the rail. While an update waits, the rail item links straight to Settings → Updates,
  which shows an update card (surface-2 panel, hairline border, radius 12) holding the release notes and the
  view's single primary button, Restart and update.

### States and alpha
- Interaction = white overlays on any surface: hover 6%, pressed 10%, focus = ring + 4%. Selected = surface-3.
- Tokens added during M1 UI scaffold: `--dl-live-banner` (live 10%, error banners), `--dl-outline-badge` (white 18%, outline badges).
- Hue tints at **6 / 10 / 14 / 18 % only**: wash (playing row, audio tile) / banner / badge + toggled
  icon / live badge. Glow 24% is for the focus halo only.
- Approved text pairs with computed contrast ratios are listed in design-language-v2 §04. Use only those.

### Logo fallback
Initials (first letter of the first two words) on `logoPalette[hash(channelId) % 8]`, text `--dl-text`.
Real logos sit on a surface-3 plate. Never a grey box or a broken image.

## 3. Type
Hanken Grotesk 400/500/600/700 (UI), Geist Mono 400/500 (diagnostics, keys, URLs). Bundled via
`@fontsource`. `tabular-nums` on body.

| use | size / weight | colour |
|---|---|---|
| Page title (h1) | 28 / 650, −1.5% | text |
| Player title | 30 / 650 | on-video |
| Zap number | 40 / 650 | on-video |
| Section heading | 15 / 600 | text-2 |
| Row title | 15 / 600 | text |
| Body | 14 / 500, lh 1.5 | text |
| Meta | 13 / 500 | text-dim |
| Caps label | 12 / 600, +6% | text-faint |
| Badge | 11 / 700, +5%, caps | per badge |

10-foot: everything × 1.25.

## 4. Space, radius, density
- Spacing 4 px base: xs 4 · sm 8 · md 12 · lg 16 · xl 24 (32, 40 for page padding).
- Radius: 4 badges · 6 buttons, inputs, rows · 8–10 tiles, menus · 12 panels, modals, popovers.
  Round only for status dots and the scrub thumb.
- Density (`theme.other.density`): channel row 44 / 64 / 88, guide row 48 / 64 / 96, logo 48×24 / 72×36 /
  112×56, control 32 / 36 / 56 (compact / comfortable / 10-foot).

## 5. Focus and motion
- Ring `--dl-focus-ring` (2 px accent + 6 px glow). On primary fills: 2 px surface gap + 2 px accent. On
  video: `--dl-focus-ring-video` (dark gap ring underneath). 10-foot: `--dl-focus-ring-tv` (3 + 9).
- Applied via `:focus-visible` and norigin `data-focused="true"` (global.css). Mantine's own ring is off.
- Motion: 120 ms hover/press · 160 ms toggles/menus · 240 ms route (fade + 8 px) · 200 ms video fade-in ·
  ease `cubic-bezier(0.2,0,0,1)`. No bounce, nothing animates during scroll. Reduced motion → opacity only.
- Blur only on video overlays, gated by `theme.other.allowBlur` (false on Linux); fallback `--dl-glass-solid`.

## 6. Buttons
| variant | Mantine `variant` | use |
|---|---|---|
| Primary | `filled` | the one main action of a view |
| Secondary | `default` (default prop) | supporting actions |
| Subtle | `subtle` | cancel, tertiary, toolbars |
| Live | `live` + 7 px live dot | Go live, Record |
| Destructive | `danger` | delete, remove source, cancel series |
| Overlay | `overlay` | anything on video |

Sizes: xs 28 · sm 32 · **md 36 (default)** · lg 44 (on video, empty states) · tv 56 (automatic via
`<html data-density="tv">`). Spec per size in `theme.other.buttonSizes`.

- States: rest, hover, pressed (darker, never moves), focus-visible, disabled, loading (width locked, loader
  replaces icon, label stays).
- Label 600, sentence case, verb first, ≤ 3 words ("Watch live", "Add source", "Delete recording").
  Never "OK" / "Submit".
- Icon left only, 16 px at md. Filled glyph only for play/pause/star-on.
- Optional key hint (Geist Mono 11 px, 70% opacity), at most one per view.
- Placement: 8 px gap. Dialogs: primary last (right). Content/hero: primary first. One primary per view
  (a modal is its own view). Destructive is never filled and never the default Enter target.
- Icon buttons (ActionIcon): always with Tooltip + aria-label. Toggled = accent tint + accent-text,
  `aria-pressed`. On video: 44 px overlay ghost, play/pause has a resting 10% fill.
- Feature code uses thin wrappers `<DLButton>` / `<DLIconButton>` with typed `variant` and `size` enums,
  not Mantine's Button directly.

## 7. Other components
- **Badges** (20 px, 11/700 caps, radius 4): LIVE/REC = live tint + dot · CATCH-UP = accent tint +
  IconHistory · NEW/HD/4K/RADIO/PREMIERE/FINALE = outline (hairline 18%) text-2 · content rating (`cert`, text as
  broadcast, e.g. TV-14) = outline · DRM/REPEAT = surface-3, text-dim.
- **Episode labels** (`lib/episode.ts`): `S2 E5`, and `S2 E5 · Episode title` wherever a programme, recording or
  episode is listed (guide block time line, preview, channel rows, channel drawer, search, recordings, home, player).
- **Details modal** (`features/info`, one instance in the shell, opened via `store/info`): 880 px, radius 12,
  surface-1. Hero = episode still or backdrop under `--dl-scrim` (none: compact header with the channel logo
  plate); title or TMDB title logo; the **slate** (episode code 40/650 tabular, episode title 18/600 text-2) is the
  one large element; slot line (channel, day + range, minutes left). A 2 px ribbon on the hero's lower edge shows
  progress (accent) only while airing. Body: badge row, actions (one primary: Watch live / Play from start / Play
  recording, focused on open), then prose (tagline, description, About the show, crew, cast strip of 2:3
  portraits, radius 8, initials fallback) beside a 14 rem facts list (12/600 text-faint label, 14/500 text-2 value,
  hairline left rule). TMDB attribution line at the foot whenever TMDB contributed. Clicking a guide block opens
  it; Enter still plays; `I` opens it from the keyboard.
- **Inputs**: 40 px (md), surface-2, hairline 10% border; focused = accent border + 3 px glow.
- **Segmented**: track surface-2 (or surface-0 inside cards), 3 px padding, active `--dl-border` bg.
- **Nav rows** (rail, groups, settings): 36–40 px, radius 6, active = surface-3 + 600 + accent icon (rail).
- **Settings row**: label 15/600 + help 13 text-dim on the left, 260 px control on the right, hairline separators.
- **Banners**: 40 px strip, tint 10% of the status hue, icon, one sentence, one text action. Never modal.
- **Skeletons**: `#1A1E23` (`--dl-row-focus`) blocks in the real row grid; label after 300 ms.
- **Errors (player)**: centred 56 px tinted icon circle, 24/650 title, one sentence, primary + "Copy diagnostics".

## 8. Screens (see reference/screens.html)
- **Rail** 224 expanded / 68 collapsed / 96 10-foot. Same size on every route for a given density; it collapses only when the user toggles it. Order:
  Search ⌘K, Home, Guide, Channels, Movies, Series, Recordings · Settings · source status dot.
  Hide Movies/Series/Recordings when no source has the cap.
- **Channels**: group sidebar 220 + virtualised rows (logo 72×36 · number · name + badges · now + 2 px
  progress · next · catch-up days + fav). Playing row: accent wash, accent-text name, speaker icon.
- **Guide**: preview pane (260 px, 384×216 art / mini player, updates after 300 ms dwell) · 40 px sticky time header · 180 px sticky
  channel column · 64 px rows · red now line + time pill. Blocks: past+archive `--dl-guide-past` + history
  icon · past no archive dashed, disabled text · now `--dl-guide-now` + 10% fill + 2 px bar · future
  `--dl-guide-future` · scheduled red dot · focused = `--dl-row-focus` tint, no ring.
- **Player**: full-bleed video, bottom `--dl-scrim` with badge row, 30/650 title, timeline, 44 px controls.
  One control bar for live, catch-up, VOD and recordings, a `1fr auto 1fr` grid of three groups: left = back,
  mute + volume; centre = transport, with play/pause dead centre whatever else is shown (start over, −10 s on
  its left; +10 s, go live on its right); right = audio, subtitles, stream, open in Plex, info,
  multiview, mini player, full screen. A control that does nothing for the current content is not rendered
  (never shown disabled): skip needs a seekable stream, start over needs a channel or a VOD/recording past
  its start, go live needs catch-up or live behind the edge, stream and multiview are live only, audio and
  subtitles need tracks. The seek slot holds the channel timeline (live/catch-up) or a position bar (VOD/recording).
  Live and catch-up share one channel timeline: a 1 h window of wall-clock time sliding under a thumb
  fixed at the centre, so there is always room to rewind. Gaps (0.25rem) only at EPG programme
  boundaries; no guide data = no gaps. Layers: unplayable (before the buffer/archive, after the live edge)
  white 6% · rewindable white 18% · watched (oldest → thumb) accent · ahead to the live edge white 32% ·
  2 px `--dl-live` tick at the live edge. Hovered segment grows ×1.75; tooltip = clock + programme title.
  Drag pans and seeks on release; click seeks. Left label playhead clock, right label `−m:ss` behind
  live. ←/→ ±10 s, Shift ±60 s, repeats gathered into one seek. Go live (live variant, `End`) whenever
  behind live. VOD/recordings keep the position bar: position (accent), buffered (white 32%), thumb + time
  tooltip. Info panel 380 px right, mono values, then
  a BUFFER · n s graph (last 30 samples at 1 Hz, 36 px bars, `--dl-accent-bar`). Channel logo (tv size) left of the badge row and title; no separate zap pill.
  Number entry pill top-right with accent caret.

## 9. Multiview
In 1.0 (PLAN §1.5, §4.11, D17).
- Automatic layout: the grid follows the tiles on screen, in slot order, main player first. One tile fills
  the view, two sit side by side, more reflow into a balanced grid with a short last row centred
  (3 = 2+1, 5 = 3+2, 7 = 3+2+2, 9 = 3×3). No empty slots; removing a tile closes the gap. Counts with a
  second arrangement also offer "featured", one large tile plus the rest: 3–4 = side column, 5 = bottom
  strip, 6 and 8 = L border. Max 4 tiles (9 if hardware allows). 4 px gaps on `--dl-multiview-canvas`.
  The canvas covers the whole view except the rects of tiles showing video (one SVG path with holes), so
  gaps, centred short rows and loading tiles never show the desktop through the transparent window.
- One tile audible: AUDIO badge (accent tint) top-right; others show a muted icon. "Audio follows focus" on.
- Active tile: **no ring, border or wash** in any state (rest, hover, keyboard focus, idle, TV). The tile
  marks itself with `data-active`, which the global focus ring does not match; only the audio badge shows it.
- Tile label pill top-left (`--dl-glass`, bare logo 48×24 with no plate, number, name). Programme + progress on scrim at bottom.
- Hover / keyboard-focus actions centred: listen, make main, full screen, replace, remove (remove in live-text).
  Buttons sit on `--dl-tile-btn` (hover `--dl-tile-btn-hover`, pressed `--dl-tile-btn-pressed`) with a hairline.
- Idle chrome: after `controlsIdleMs` without input the audio indicator, tile actions and toolbar
  fade out and the cursor hides; any mouse/key input brings them back. The toolbar stays while the pointer
  is on it or a picker is open.
- Stream budget: picker footer only, "Uses 1 stream · 3 of 4 in use". The toolbar shows no count.
- Stream-limit tile: amber gauge icon, "Your server is at its stream limit", one sentence, Replace channel / Remove tile.
- Add picker: popover anchored to the toolbar "Add channel" button (or the single empty slot when nothing
  plays). Each row has a checkbox on the right, checked for channels on screen; checking adds the channel
  to the first free slot, unchecking removes its tile (the main player's channel is checked and locked).
  The popover stays open across picks and closes on an outside click, Esc, or the Add channel button.
  Unchecked rows disable when the budget is used up. Search + Favourites / Recent / All; bare logos (no
  plate). Each row shows the programme on now under the name (12 px, dim).
- Replace picker (tile action / stream-limit tile): same list, single pick, closes on pick; on-screen
  channels disabled with "Already in slot N".
- Tiles never show subtitles or captions (the tile profile sets `sid=no`); make main to use them.
- Floating toolbar bottom centre: Add channel (N), layout segmented (icons; only when the count has more than
  one layout), audio-follows-focus switch, Exit (Esc).
- Keys: 1–9 focus · A audio · Enter make main · F full screen · Del remove · N add channel · L cycle layout · Esc exit.
- Entry: grid icon in the player control bar; "Add to multiview" in the channel context menu.

## 10. Copy
Sentence case; caps only in badges and 12 px labels. Errors: one sentence, cause if known, one action.
Waits > 300 ms say what they wait on ("Connecting to …"). Never show URLs with credentials, codes or
stack traces in the UI; those go to diagnostics. All copy through `t()`.

## 11. Uniformity rules (PR checklist)
| # | rule | enforced by |
|---|---|---|
| 1 | No raw colours in components: tokens or Mantine colour keys only. Hex only in theme.ts. | stylelint `color-no-hex` (except theme.ts), ESLint `no-restricted-syntax` on `#` literals in style props |
| 2 | No raw spacing, radius or font sizes: Mantine scale keys or `theme.other` constants. | stylelint `declaration-property-unit-allowed-list` |
| 3 | One primary button per view. | review |
| 4 | Five button sizes only; tv comes from the density context. | `<DLButton>` size enum |
| 5 | Every interactive element has rest, hover, pressed, focus-visible, disabled (+ loading if async). | Storybook story per component showing the state row |
| 6 | Focus = shared ring tokens. No `outline: none` without the ring. | stylelint + review |
| 7 | Accent = focused / selected / progress only. | review |
| 8 | Red, green, amber = status only. | review |
| 9 | Hue tints at 6 / 10 / 14 / 18 % only (tokens). | token lint |
| 10 | Elevation = lighter surface; shadows only on floating layers. | review |
| 11 | Tabular numerals everywhere (body). | global.css |
| 12 | Tabler icons, stroke 1.75, sizes 14/16/20/24 (26 in 10-foot). | `<Icon>` wrapper size enum |
| 13 | Over video: `--dl-on-video` on scrim/glass; blur only when `allowBlur`. | review |
| 14 | Copy rules (§10). | review + i18n keys |
| 15 | Logo fallback deterministic. | unit test on hash |
| 16 | New pattern → add token/component + story first, then use it. | PR template checkbox |
