import { describe, expect, it } from 'vitest';
import { planOpen, wallTiles } from './saved';

const tiles = (...pairs: [number, number][]) => pairs.map(([slot, channelId]) => ({ slot, channelId }));

describe('planOpen', () => {
  it('starts tiles in slot order up to the budget and overflows the rest', () => {
    const p = planOpen(tiles([3, 13], [0, 10], [1, 11], [5, 15], [2, 12]), 0, 3);
    expect(p.start).toEqual(tiles([0, 10], [1, 11], [2, 12]));
    expect(p.overflow).toEqual([13, 15]);
  });

  it('compacts gaps and moves the audible slot with its channel', () => {
    const p = planOpen(tiles([1, 11], [4, 14]), 4, 4);
    expect(p.start).toEqual(tiles([0, 11], [1, 14]));
    expect(p.audible).toBe(1);
  });

  it('falls back to the main player when the audible tile overflowed', () => {
    expect(planOpen(tiles([0, 10], [1, 11], [2, 12]), 2, 2).audible).toBe(0);
  });

  it('always starts at least one tile', () => {
    expect(planOpen(tiles([0, 10], [1, 11]), 0, 0).start).toEqual(tiles([0, 10]));
  });
});

describe('wallTiles', () => {
  it('keeps on-screen slots and puts off-screen channels in the free slots', () => {
    const screen = new Map([[2, { channelId: 12 }], [0, { channelId: 10 }]]);
    expect(wallTiles(screen, [20, 10, 21])).toEqual(tiles([0, 10], [1, 20], [2, 12], [3, 21]));
  });

  it('caps the list at nine tiles', () => {
    const screen = new Map(Array.from({ length: 8 }, (_, i) => [i, { channelId: 100 + i }] as const));
    expect(wallTiles(screen, [1, 2, 3])).toHaveLength(9);
  });
});
