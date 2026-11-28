// Sports command center tile assignment (docs/COMMAND_CENTER.md §3). Pure: the page feeds it the
// followed matches, what is on screen and the user's intent, and gets back the minimal set of tile
// changes plus the games that did not fit. No React, no IPC.
import type { SportsMatch } from '../../api/types';
import { onAirNow } from '../sports/follow';

/** What the user follows that a match is for, most specific first: `league:team`, then the league. */
export const followKeys = (m: SportsMatch): string[] => [...m.followedTeams.map((t) => `${m.league}:${t}`), m.league];

/**
 * Rank of a match, lower is better: its best entry in `priority`; anything unranked comes after
 * every ranked entry, in catalog order of its league (`leagueOrder`).
 */
export function rankOf(m: SportsMatch, priority: readonly string[], leagueOrder: ReadonlyMap<string, number>): number {
  let best = Infinity;
  for (const k of followKeys(m)) {
    const i = priority.indexOf(k);
    if (i >= 0 && i < best) best = i;
  }
  return Number.isFinite(best) ? best : priority.length + (leagueOrder.get(m.league) ?? leagueOrder.size);
}

/**
 * Live games worth a tile, best first: one per event (copies were grouped by the backend; a
 * programme matched for two leagues shows once). Live broadcasts before replays, then the user's
 * rank, then confirmed live before "likely live", then match score.
 */
export function liveCandidates(
  matches: readonly SportsMatch[],
  now: number,
  priority: readonly string[],
  leagueOrder: ReadonlyMap<string, number>,
): SportsMatch[] {
  const seen = new Set<number>();
  const live = matches.filter((m) => {
    if (!onAirNow(m, now) || seen.has(m.channel.id)) return false;
    seen.add(m.channel.id);
    return true;
  });
  const key = (m: SportsMatch) => [m.live ? 0 : 1, rankOf(m, priority, leagueOrder), m.liveInferred ? 1 : 0, -m.score];
  return live.sort((a, b) => {
    const ka = key(a);
    const kb = key(b);
    for (let i = 0; i < ka.length; i++) if (ka[i] !== kb[i]) return (ka[i] ?? 0) - (kb[i] ?? 0);
    return 0;
  });
}

/** Every channel showing a match: the lead channel and its alternates. */
export const matchChannels = (m: SportsMatch): number[] => [m.channel.id, ...m.alternates.map((c) => c.id)];

export interface ScreenTile {
  slot: number;
  channelId: number;
  /** Placed by the user (picker, swap, add): never replaced or removed automatically. */
  pinned: boolean;
  /** Failed tiles stay as they are; the stream-limit tile offers its own actions. */
  failed: boolean;
}

export interface AssignInput {
  /** From `liveCandidates`, best first. */
  candidates: readonly SportsMatch[];
  tiles: readonly ScreenTile[];
  /** Slots that exist on this machine: 0 (main player) .. maxTiles - 1. */
  maxTiles: number;
  /** How many tiles to fill at most: the stream budget, lowered when the user removes tiles. */
  capacity: number;
  /** Channels the user took off screen this session; auto-fill never brings them back. */
  dismissed: ReadonlySet<number>;
}

export interface AssignPlan {
  /** Tiles to (re)assign, slot 0 first. Slot 0 is the main player (`play_channel`). */
  set: { slot: number; channelId: number }[];
  /** Tiles to stop. Never slot 0: the main player keeps its last channel. */
  remove: number[];
  /** Matches with a tile, by slot. */
  onScreen: Map<number, SportsMatch>;
  /** Live games without a tile, best first ("Also live"). */
  overflow: SportsMatch[];
}

/**
 * Minimal changes that keep the wall full of the best live games:
 * - pinned and failed tiles stay; an unpinned tile stays while its game is live (no reshuffling when
 *   a better game starts: that one goes to overflow instead),
 * - an unpinned tile whose game is over is reused for the next game, else stopped,
 * - free slots fill in slot order up to `capacity`.
 */
export function assign({ candidates, tiles, maxTiles, capacity, dismissed }: AssignInput): AssignPlan {
  // A channel's own match wins over one that lists it as an alternate.
  const byChannel = new Map<number, SportsMatch>();
  for (const m of candidates) if (!byChannel.has(m.channel.id)) byChannel.set(m.channel.id, m);
  for (const m of candidates) for (const c of m.alternates) if (!byChannel.has(c.id)) byChannel.set(c.id, m);

  const kept: ScreenTile[] = [];
  const freed: number[] = [];
  for (const tile of [...tiles].sort((a, b) => a.slot - b.slot)) {
    if (tile.pinned || tile.failed || byChannel.has(tile.channelId)) kept.push(tile);
    else freed.push(tile.slot);
  }

  const onScreen = new Map<number, SportsMatch>();
  const covered = new Set<SportsMatch>();
  for (const tile of kept) {
    const m = byChannel.get(tile.channelId);
    if (m && !covered.has(m)) {
      onScreen.set(tile.slot, m);
      covered.add(m);
    }
  }

  const waiting = candidates.filter((m) => !covered.has(m) && !matchChannels(m).some((id) => dismissed.has(id)));
  const used = new Set(kept.map((x) => x.slot));
  // Reuse freed slots first (their tile already sits in the grid), then empty ones in slot order.
  const empty = Array.from({ length: maxTiles }, (_, i) => i).filter((s) => !used.has(s) && !freed.includes(s));
  const slots = [...freed, ...empty];
  const room = Math.max(0, Math.min(capacity, maxTiles) - kept.length);

  const set: AssignPlan['set'] = [];
  let next = 0;
  for (const slot of slots) {
    if (set.length >= room || next >= waiting.length) break;
    const m = waiting[next++] as SportsMatch;
    set.push({ slot, channelId: m.channel.id });
    onScreen.set(slot, m);
  }
  const filled = new Set(set.map((x) => x.slot));
  const remove = freed.filter((s) => s !== 0 && !filled.has(s));
  set.sort((a, b) => a.slot - b.slot);
  return { set, remove, onScreen, overflow: waiting.slice(next) };
}
