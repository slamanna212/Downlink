import { describe, expect, it } from 'vitest';
import type { Channel, Programme, SportsMatch } from '../../api/types';
import { assign, liveCandidates, rankOf, type ScreenTile } from './assign';

const NOW = 1_800_000_000;
const H = 60 * 60;

const channel = (id: number): Channel => ({
  id, sourceId: 1, uuid: null, number: id, name: `Channel ${id}`, groupId: null, logo: null, quality: null,
  isCatchup: false, catchupDays: 0, isRadio: false, hasDrm: false, isFavourite: false, epgMatch: 'id',
});
const programme = (channelId: number, start: number, end: number): Programme => ({
  id: `${channelId}:${start}`, channelId, startUtc: start, endUtc: end, title: 'Game', subtitle: null, description: null,
  season: null, episode: null, isNew: false, isLive: true, isPremiere: false, isFinale: false, isScheduled: false,
});
/** A game on channel `id`, on air now unless `start` says otherwise. */
const game = (id: number, extra: Partial<SportsMatch> = {}, start = NOW - H, end = NOW + H): SportsMatch => ({
  channel: channel(id), programme: programme(id, start, end), league: 'nfl', leagueName: 'NFL',
  teams: [], followedTeams: [], session: null, live: true, liveInferred: false, source: 'programme', score: 60, eventStartUtc: null, alternates: [], ...extra,
});
const tile = (slot: number, channelId: number, extra: Partial<ScreenTile> = {}): ScreenTile => ({ slot, channelId, pinned: false, failed: false, ...extra });
const ORDER = new Map([['nfl', 0], ['nba', 1], ['f1', 2]]);
const ids = (ms: SportsMatch[]) => ms.map((m) => m.channel.id);
const base = { maxTiles: 4, capacity: 4, dismissed: new Set<number>() };

describe('rankOf', () => {
  it('uses the best priority entry: a ranked team beats its ranked league', () => {
    const m = game(1, { followedTeams: ['chi'] });
    expect(rankOf(m, ['f1', 'nfl', 'nfl:chi'], ORDER)).toBe(1);
    expect(rankOf(m, ['nfl:chi', 'f1'], ORDER)).toBe(0);
  });

  it('puts unranked matches after every ranked one, in catalog order', () => {
    expect(rankOf(game(1, { league: 'nba' }), ['f1'], ORDER)).toBe(1 + 1);
    expect(rankOf(game(1, { league: 'f1' }), ['f1'], ORDER)).toBe(0);
    expect(rankOf(game(1, { league: 'zzz' }), [], ORDER)).toBe(ORDER.size);
  });
});

describe('liveCandidates', () => {
  it('keeps only games on air, one per channel', () => {
    const later = game(2, {}, NOW + H, NOW + 2 * H);
    const ended = game(3, {}, NOW - 3 * H, NOW - H);
    const twice = game(1, { league: 'nba' });
    expect(ids(liveCandidates([game(1), later, ended, twice], NOW, [], ORDER))).toEqual([1]);
  });

  it('orders live before replay, then rank, then confirmed before likely live, then score', () => {
    const replay = game(1, { live: false, league: 'f1' });
    const nba = game(2, { league: 'nba' });
    const likely = game(3, { liveInferred: true });
    const low = game(4, { score: 55 });
    const high = game(5, { score: 90 });
    expect(ids(liveCandidates([replay, nba, likely, low, high], NOW, ['f1', 'nfl'], ORDER))).toEqual([5, 4, 3, 2, 1]);
  });
});

describe('assign', () => {
  it('fills an empty wall in slot order, main player first, up to the budget', () => {
    const plan = assign({ ...base, candidates: [game(1), game(2), game(3), game(4), game(5)], tiles: [] });
    expect(plan.set).toEqual([{ slot: 0, channelId: 1 }, { slot: 1, channelId: 2 }, { slot: 2, channelId: 3 }, { slot: 3, channelId: 4 }]);
    expect(ids(plan.overflow)).toEqual([5]);
    expect(plan.remove).toEqual([]);
  });

  it('respects a budget of one and of nine', () => {
    const many = Array.from({ length: 10 }, (_, i) => game(i + 1));
    expect(assign({ ...base, maxTiles: 4, capacity: 1, candidates: many, tiles: [] }).set).toHaveLength(1);
    const nine = assign({ ...base, maxTiles: 9, capacity: 9, candidates: many, tiles: [] });
    expect(nine.set).toHaveLength(9);
    expect(ids(nine.overflow)).toEqual([10]);
  });

  it('does not reshuffle: a better game that starts goes to overflow', () => {
    const plan = assign({ ...base, capacity: 2, candidates: [game(9), game(1), game(2)], tiles: [tile(0, 1), tile(1, 2)] });
    expect(plan.set).toEqual([]);
    expect(ids(plan.overflow)).toEqual([9]);
  });

  it('advances a tile whose game ended, reusing its slot', () => {
    const plan = assign({ ...base, capacity: 2, candidates: [game(1), game(3)], tiles: [tile(0, 1), tile(1, 2)] });
    expect(plan.set).toEqual([{ slot: 1, channelId: 3 }]);
    expect(plan.remove).toEqual([]);
  });

  it('stops an ended tile when nothing is left, but never the main player', () => {
    const plan = assign({ ...base, candidates: [], tiles: [tile(0, 1), tile(2, 2)] });
    expect(plan.set).toEqual([]);
    expect(plan.remove).toEqual([2]);
  });

  it('keeps pinned and failed tiles even when their channel has no live game', () => {
    const plan = assign({ ...base, capacity: 3, candidates: [game(5), game(6)], tiles: [tile(0, 1, { pinned: true }), tile(1, 2, { failed: true })] });
    expect(plan.set).toEqual([{ slot: 2, channelId: 5 }]);
    expect(ids(plan.overflow)).toEqual([6]);
  });

  it('counts a tile on an alternate channel as showing that game', () => {
    const m = game(1, { alternates: [channel(7)] });
    const plan = assign({ ...base, capacity: 2, candidates: [m, game(2)], tiles: [tile(0, 7)] });
    expect(plan.onScreen.get(0)).toBe(m);
    expect(plan.set).toEqual([{ slot: 1, channelId: 2 }]);
  });

  it("prefers a channel's own match over one listing it as an alternate", () => {
    const main = game(1, { alternates: [channel(7)] });
    const event = game(7);
    const plan = assign({ ...base, capacity: 2, candidates: [main, event], tiles: [tile(0, 1), tile(1, 7)] });
    expect(plan.onScreen.get(0)).toBe(main);
    expect(plan.onScreen.get(1)).toBe(event);
    expect(plan.overflow).toEqual([]);
  });

  it('never brings back a dismissed channel and honors a lowered capacity', () => {
    const plan = assign({ ...base, capacity: 2, dismissed: new Set([2]), candidates: [game(1), game(2), game(3)], tiles: [tile(0, 1)] });
    expect(plan.set).toEqual([{ slot: 1, channelId: 3 }]);
    expect(ids(plan.overflow)).toEqual([]);
  });

  it('drops unpinned tiles beyond a lowered capacity only when their game is over', () => {
    const plan = assign({ ...base, capacity: 1, candidates: [game(1), game(2)], tiles: [tile(0, 1), tile(1, 2)] });
    expect(plan.set).toEqual([]);
    expect(plan.remove).toEqual([]);
  });
});
