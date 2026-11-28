// News feeds on a saved command center (docs/COMMAND_CENTER.md §7.x). The center's `rail` document is
// UI-owned JSON: `feeds: [{ id }]` in display order and `ticker: boolean`. Unknown keys are kept.
// Pure helpers so the document rules are testable.
import type { CommandCenter, CommandCenterRail, CommandCenterSave } from '../../../api/types';

/** Feed ids attached to the center, in order, without duplicates or junk. */
export function railFeedIds(rail: CommandCenterRail | undefined): number[] {
  const out: number[] = [];
  for (const f of rail?.feeds ?? []) {
    const id = (f as { id?: unknown } | null)?.id;
    if (typeof id === 'number' && Number.isInteger(id) && id > 0 && !out.includes(id)) out.push(id);
  }
  return out;
}

export const withFeeds = (rail: CommandCenterRail, ids: readonly number[]): CommandCenterRail => ({
  ...rail,
  feeds: [...new Set(ids)].map((id) => ({ id })),
});

export const attachFeed = (rail: CommandCenterRail, id: number): CommandCenterRail => withFeeds(rail, [...railFeedIds(rail), id]);

export const detachFeed = (rail: CommandCenterRail, id: number): CommandCenterRail => withFeeds(rail, railFeedIds(rail).filter((x) => x !== id));

/** The ticker shows when the center has feeds and it was not switched off. */
export const tickerOn = (rail: CommandCenterRail | undefined): boolean => rail?.ticker !== false && railFeedIds(rail).length > 0;

export const withTicker = (rail: CommandCenterRail, on: boolean): CommandCenterRail => ({ ...rail, ticker: on });

/**
 * The save document for a rail-only change: the center as stored, with a new rail. The wall on screen
 * (hand edits, not saved yet) is left out on purpose; "Save changes" covers that.
 */
export const railSave = (center: CommandCenter, rail: CommandCenterRail): CommandCenterSave => ({
  id: center.id,
  name: center.name,
  layoutStyle: center.layoutStyle,
  audible: center.audible,
  rail,
  tiles: center.tiles.map((x) => ({ slot: x.slot, channelId: x.channel.id })),
});
