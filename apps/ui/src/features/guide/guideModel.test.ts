import { describe, expect, it } from 'vitest';
import type { Channel, Programme } from '../../api/types';
import {
  archiveAvailable, blockState, enterAction, indexAt, isCovered, mergeProgrammes, moveFocus, neededChunks,
  ppsFor, scrollToReveal, sliceRange, timeToX, visibleRange, withMargin, xToTime, type Geometry,
} from './guideModel';

const NOW = 1_000_000;
const g: Geometry = { rangeStart: NOW - 86400, rangeEnd: NOW + 86400, pps: ppsFor(1180, 180, 3), colW: 180, headerH: 40, rowH: 64 };
const prog = (id: string, ch: number, s: number, e: number): Programme => ({
  id, channelId: ch, startUtc: s, endUtc: e, title: id, subtitle: null, description: null, season: null, episode: null,
  isNew: false, isLive: false, isPremiere: false, isFinale: false, isScheduled: false,
});
const chan = (over: Partial<Channel> = {}): Channel => ({
  id: 1, sourceId: 1, uuid: null, number: 1, name: 'A', groupId: 1, logo: null, quality: null, isCatchup: true, catchupDays: 7,
  isRadio: false, hasDrm: false, isFavourite: false, epgMatch: 'id', ...over,
});

describe('geometry', () => {
  it('round-trips time and x', () => {
    expect(xToTime(g, timeToX(g, NOW))).toBeCloseTo(NOW, 6);
    expect(timeToX(g, g.rangeStart)).toBe(180);
  });
  it('fits the span in the visible area', () => {
    expect((3 * 3600) * g.pps).toBeCloseTo(1000, 6);
  });
});

describe('render range', () => {
  const v = { scrollLeft: timeToX(g, NOW) - 180, scrollTop: 0, viewW: 1180, viewH: 600 };
  it('computes visible rows and times', () => {
    const r = visibleRange(g, v, 5000);
    expect(r.r0).toBe(0);
    expect(r.r1).toBe(Math.ceil((600 - 40) / 64));
    expect(r.t0).toBeCloseTo(NOW, 3);
    expect(r.t1 - r.t0).toBeCloseTo(3 * 3600, 3);
  });
  it('adds margin and detects coverage', () => {
    const vis = visibleRange(g, v, 5000);
    const cached = withMargin(vis, 5000);
    expect(isCovered(cached, vis)).toBe(true);
    const moved = visibleRange(g, { ...v, scrollLeft: v.scrollLeft + 200 }, 5000);
    expect(isCovered(cached, moved)).toBe(true);
    const far = visibleRange(g, { ...v, scrollLeft: v.scrollLeft + 5000 }, 5000);
    expect(isCovered(cached, far)).toBe(false);
    expect(isCovered(null, vis)).toBe(false);
  });
  it('lists the chunks to fetch', () => {
    const c = neededChunks({ t0: g.rangeStart + 1, t1: g.rangeStart + 7 * 3600, r0: 38, r1: 82 }, g.rangeStart);
    expect(c.c).toEqual([0, 1, 2]);
    expect(c.t).toEqual([0, 1]);
  });
});

describe('block state and enter action', () => {
  const ch = chan();
  it('classifies programmes', () => {
    expect(blockState(prog('a', 1, NOW - 100, NOW + 100), ch, NOW)).toBe('now');
    expect(blockState(prog('b', 1, NOW + 100, NOW + 200), ch, NOW)).toBe('future');
    expect(blockState(prog('c', 1, NOW - 3600, NOW - 100), ch, NOW)).toBe('past-archive');
    expect(blockState(prog('d', 1, NOW - 9 * 86400, NOW - 9 * 86400 + 600), ch, NOW)).toBe('past-none');
    expect(blockState(prog('e', 1, NOW - 3600, NOW - 100), chan({ isCatchup: false }), NOW)).toBe('past-none');
  });
  it('respects the archive window', () => {
    expect(archiveAvailable(prog('x', 1, NOW - 8 * 86400, NOW - 8 * 86400 + 60), ch, NOW)).toBe(false);
    expect(archiveAvailable(prog('x', 1, NOW - 6 * 86400, NOW - 6 * 86400 + 60), ch, NOW)).toBe(true);
  });
  it('maps state to Enter', () => {
    expect(enterAction('now')).toBe('play-live');
    expect(enterAction('past-archive')).toBe('play-catchup');
    expect(enterAction('future')).toBe('details');
    expect(enterAction('past-none')).toBe('none');
  });
});

describe('lists', () => {
  const list = [prog('1', 1, 0, 100), prog('2', 1, 100, 250), prog('3', 1, 250, 300)];
  it('finds the programme covering a time', () => {
    expect(indexAt(list, 0)).toBe(0);
    expect(indexAt(list, 99)).toBe(0);
    expect(indexAt(list, 100)).toBe(1);
    expect(indexAt(list, 299)).toBe(2);
    expect(indexAt(list, 300)).toBe(-1);
  });
  it('slices a range', () => {
    expect(sliceRange(list, 90, 260).map((p) => p.id)).toEqual(['1', '2', '3']);
    expect(sliceRange(list, 100, 250).map((p) => p.id)).toEqual(['2']);
  });
  it('merges chunks, deduping and sorting', () => {
    const m = mergeProgrammes([[list[1]!, list[0]!], [list[1]!, list[2]!, prog('z', 2, 5, 9)]]);
    expect(m.get(1)!.map((p) => p.id)).toEqual(['1', '2', '3']);
    expect(m.get(2)).toHaveLength(1);
  });
});

describe('moveFocus', () => {
  const rows = new Map([[0, [prog('a', 1, 0, 100), prog('b', 1, 100, 200)]], [1, [prog('c', 2, 0, 300)]]]);
  const listFor = (r: number) => rows.get(r);
  it('hops to neighbouring programmes', () => {
    expect(moveFocus({ row: 0, t: 10 }, 'ArrowRight', 2, 5, listFor, 0)).toEqual({ row: 0, t: 100 });
    expect(moveFocus({ row: 0, t: 150 }, 'ArrowLeft', 2, 5, listFor, 0)).toEqual({ row: 0, t: 0 });
    expect(moveFocus({ row: 0, t: 150 }, 'ArrowRight', 2, 5, listFor, 0)).toEqual({ row: 0, t: 150 });
  });
  it('keeps time across rows, clamped to the view, and clamps rows', () => {
    expect(moveFocus({ row: 0, t: 10 }, 'ArrowDown', 2, 5, listFor, 50)).toEqual({ row: 1, t: 50 });
    expect(moveFocus({ row: 1, t: 10 }, 'ArrowDown', 2, 5, listFor, 0).row).toBe(1);
    expect(moveFocus({ row: 1, t: 10 }, 'PageUp', 2, 5, listFor, 0).row).toBe(0);
  });
  it('handles rows with no data', () => {
    expect(moveFocus({ row: 5, t: 10 }, 'ArrowRight', 9, 5, listFor, 0)).toEqual({ row: 5, t: 10 });
  });
});

describe('scrollToReveal', () => {
  const v = { scrollLeft: 5000, scrollTop: 640, viewW: 1180, viewH: 600 };
  it('returns nothing when visible', () => {
    const t = xToTime(g, 5000 + 400);
    expect(scrollToReveal(g, v, { start: t, end: t + 600, row: 12 })).toEqual({});
  });
  it('scrolls left/right and up/down as needed', () => {
    expect(scrollToReveal(g, v, { start: xToTime(g, 4000), end: xToTime(g, 4200), row: 12 }).left).toBeLessThan(4000);
    expect(scrollToReveal(g, v, { start: xToTime(g, 6300), end: xToTime(g, 6500), row: 12 }).left).toBeGreaterThan(5000);
    expect(scrollToReveal(g, v, { start: xToTime(g, 5400), end: xToTime(g, 5500), row: 2 }).top).toBe(128);
    expect(scrollToReveal(g, v, { start: xToTime(g, 5400), end: xToTime(g, 5500), row: 40 }).top).toBeGreaterThan(640);
  });
});
