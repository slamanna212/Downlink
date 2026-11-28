// Saved command centers (COMMAND_CENTER §6.1): pure helpers for opening a center on the wall and for
// turning the wall back into a saved tile list.

/** Slots run 0 (main player) .. 8. */
export const MAX_SLOTS = 9;

export interface SavedTile {
  slot: number;
  channelId: number;
}

export interface OpenPlan {
  /** Tiles to start, compacted into slots 0, 1, 2… in saved slot order. */
  start: SavedTile[];
  /** Channels that did not fit the budget, in saved slot order (the rail's "Not on screen"). */
  overflow: number[];
  /** Where the saved audible tile ends up (0 when it did not fit). */
  audible: number;
}

/**
 * How a saved center goes on the wall: saved tiles in slot order fill slots 0.. up to the budget
 * (`min(maxTiles, stream limit)`), the rest overflow. The audible tile follows its channel.
 */
export function planOpen(tiles: SavedTile[], audibleSlot: number, budget: number): OpenPlan {
  const ordered = [...tiles].sort((a, b) => a.slot - b.slot);
  const cap = Math.max(1, Math.min(budget, MAX_SLOTS));
  const start = ordered.slice(0, cap).map((x, i) => ({ slot: i, channelId: x.channelId }));
  const overflow = ordered.slice(cap).map((x) => x.channelId);
  const audibleChannel = ordered.find((x) => x.slot === audibleSlot)?.channelId;
  const audible = start.find((x) => x.channelId === audibleChannel)?.slot ?? 0;
  return { start, overflow, audible };
}

/**
 * The tile list to save: tiles on screen keep their slots, then `keep` (channels off screen that
 * belong to the center) take the free slots in order. Duplicates are dropped; at most nine tiles.
 */
export function wallTiles(onScreen: ReadonlyMap<number, { channelId: number }>, keep: readonly number[] = []): SavedTile[] {
  const out: SavedTile[] = [...onScreen]
    .filter(([slot]) => slot >= 0 && slot < MAX_SLOTS)
    .sort(([a], [b]) => a - b)
    .map(([slot, x]) => ({ slot, channelId: x.channelId }));
  const used = new Set(out.map((x) => x.slot));
  const seen = new Set(out.map((x) => x.channelId));
  let free = 0;
  for (const channelId of keep) {
    if (seen.has(channelId)) continue;
    while (used.has(free)) free++;
    if (free >= MAX_SLOTS) break;
    out.push({ slot: free, channelId });
    used.add(free);
    seen.add(channelId);
  }
  return out.sort((a, b) => a.slot - b.slot);
}
