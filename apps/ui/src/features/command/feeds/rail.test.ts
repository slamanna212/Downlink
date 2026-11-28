import { describe, expect, it } from 'vitest';
import type { CommandCenter } from '../../../api/types';
import { attachFeed, detachFeed, railFeedIds, railSave, tickerOn, withTicker } from './rail';

describe('command center feeds rail', () => {
  it('reads feed ids in order and skips duplicates and junk', () => {
    expect(railFeedIds({ feeds: [{ id: 3 }, { id: 1 }, { id: 3 }, { id: -2 }, { id: 'x' } as never, null as never] })).toEqual([3, 1]);
    expect(railFeedIds(undefined)).toEqual([]);
  });

  it('attaches and detaches feeds, keeping other keys', () => {
    const rail = attachFeed({ layoutNote: 'keep', feeds: [{ id: 1 }] }, 2);
    expect(rail).toEqual({ layoutNote: 'keep', feeds: [{ id: 1 }, { id: 2 }] });
    expect(attachFeed(rail, 2).feeds).toEqual([{ id: 1 }, { id: 2 }]);
    expect(detachFeed(rail, 1)).toEqual({ layoutNote: 'keep', feeds: [{ id: 2 }] });
  });

  it('shows the ticker only with feeds, on by default', () => {
    expect(tickerOn({})).toBe(false);
    expect(tickerOn({ feeds: [{ id: 1 }] })).toBe(true);
    expect(tickerOn(withTicker({ feeds: [{ id: 1 }] }, false))).toBe(false);
  });

  it('saves the stored center with the new rail', () => {
    const center = {
      id: 7, name: 'Desk', layoutStyle: 'featured', audible: 1, rail: {}, createdUtc: 1, updatedUtc: 2,
      tiles: [{ slot: 0, channel: { id: 11 } }, { slot: 1, channel: { id: 12 } }],
    } as unknown as CommandCenter;
    expect(railSave(center, { feeds: [{ id: 4 }] })).toEqual({
      id: 7, name: 'Desk', layoutStyle: 'featured', audible: 1, rail: { feeds: [{ id: 4 }] },
      tiles: [{ slot: 0, channelId: 11 }, { slot: 1, channelId: 12 }],
    });
  });
});
