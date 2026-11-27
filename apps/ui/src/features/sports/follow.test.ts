import { describe, expect, it } from 'vitest';
import type { Channel, Programme, SportsMatch } from '../../api/types';
import { MOCK_SPORTS_CATALOG } from '../../api/ipc.mock.sports';
import {
  allGames,
  DEFAULT_SESSIONS,
  emptyFollow,
  filterTeams,
  followLabel,
  followsAnything,
  homeSportsItems,
  motorsportGroups,
  onAirNow,
  setIncludeReplays,
  setLeagueFollowed,
  setSeriesFollowed,
  sortMatches,
  toggleSession,
  toggleTeam,
} from './follow';

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
const match = (id: number, start: number | null, end: number | null, extra: Partial<SportsMatch> = {}): SportsMatch => ({
  channel: channel(id), programme: start != null && end != null ? programme(id, start, end) : null, league: 'nfl', leagueName: 'NFL',
  teams: [], followedTeams: [], session: null, live: true, liveInferred: false, source: start != null ? 'programme' : 'channel', score: 60, eventStartUtc: null, alternates: [], ...extra,
});

describe('follow toggling', () => {
  it('follows a league with all games, and drops it when switched off', () => {
    const on = setLeagueFollowed(emptyFollow(), 'nfl', true);
    expect(on.leagues).toEqual({ nfl: { teams: [] } });
    expect(setLeagueFollowed(on, 'nfl', false).leagues).toEqual({});
  });

  it('keeps picked teams when a followed league is switched on again', () => {
    const f = toggleTeam(setLeagueFollowed(emptyFollow(), 'nfl', true), 'nfl', 'chi');
    expect(setLeagueFollowed(f, 'nfl', true).leagues.nfl).toEqual({ teams: ['chi'] });
  });

  it('narrows to picked teams, and falls back to all games when the last team is removed', () => {
    let f = setLeagueFollowed(emptyFollow(), 'nfl', true);
    f = toggleTeam(f, 'nfl', 'chi');
    f = toggleTeam(f, 'nfl', 'gb');
    expect(f.leagues.nfl?.teams).toEqual(['chi', 'gb']);
    f = toggleTeam(f, 'nfl', 'chi');
    f = toggleTeam(f, 'nfl', 'gb');
    expect(f.leagues.nfl).toEqual({ teams: [] });
  });

  it('"All games" clears the team filter but keeps the league followed', () => {
    const f = allGames(toggleTeam(setLeagueFollowed(emptyFollow(), 'nfl', true), 'nfl', 'chi'), 'nfl');
    expect(f.leagues).toEqual({ nfl: { teams: [] } });
  });

  it('never mutates the document it is given', () => {
    const before = emptyFollow();
    const snapshot = structuredClone(before);
    setSeriesFollowed(toggleTeam(setLeagueFollowed(before, 'nfl', true), 'nfl', 'chi'), 'f1', true);
    expect(before).toEqual(snapshot);
  });

  it('keeps unknown keys from an older catalog untouched', () => {
    const f = { ...emptyFollow(), leagues: { gone: { teams: ['x'] } } };
    expect(setLeagueFollowed(f, 'nfl', true).leagues.gone).toEqual({ teams: ['x'] });
  });

  it('sets include replays', () => {
    expect(setIncludeReplays(emptyFollow(), true).includeReplays).toBe(true);
  });
});

describe('session defaults', () => {
  it('turns on race, qualifying and practice for a newly followed series, not shows', () => {
    const f = setSeriesFollowed(emptyFollow(), 'f1', true);
    expect(f.series.f1?.sessions).toEqual(['race', 'qualifying', 'practice']);
    expect(DEFAULT_SESSIONS).not.toContain('shows');
  });

  it('does not reset sessions when an already followed series is switched on again', () => {
    const f = toggleSession(setSeriesFollowed(emptyFollow(), 'f1', true), 'f1', 'practice');
    expect(setSeriesFollowed(f, 'f1', true).series.f1?.sessions).toEqual(['race', 'qualifying']);
  });

  it('toggles sessions and keeps them in catalog order', () => {
    let f = setSeriesFollowed(emptyFollow(), 'f1', true);
    f = toggleSession(f, 'f1', 'race');
    f = toggleSession(f, 'f1', 'shows');
    f = toggleSession(f, 'f1', 'race');
    expect(f.series.f1?.sessions).toEqual(['race', 'qualifying', 'practice', 'shows']);
  });

  it('starts from the defaults again after the series is switched off and on', () => {
    let f = toggleSession(setSeriesFollowed(emptyFollow(), 'f1', true), 'f1', 'shows');
    f = setSeriesFollowed(setSeriesFollowed(f, 'f1', false), 'f1', true);
    expect(f.series.f1?.sessions).toEqual([...DEFAULT_SESSIONS]);
  });
});

describe('followsAnything', () => {
  it('is false for an empty follow and for keys the catalog no longer has', () => {
    expect(followsAnything(undefined)).toBe(false);
    expect(followsAnything(emptyFollow(), MOCK_SPORTS_CATALOG)).toBe(false);
    expect(followsAnything({ ...emptyFollow(), series: { retired: { sessions: ['race'] } } }, MOCK_SPORTS_CATALOG)).toBe(false);
  });

  it('is true once a known league or series is followed', () => {
    expect(followsAnything(setSeriesFollowed(emptyFollow(), 'f1', true), MOCK_SPORTS_CATALOG)).toBe(true);
    expect(followsAnything(setLeagueFollowed(emptyFollow(), 'nfl', true), MOCK_SPORTS_CATALOG)).toBe(true);
  });
});

describe('catalog helpers', () => {
  it('groups motorsport in spec order and leaves team leagues out', () => {
    const groups = motorsportGroups(MOCK_SPORTS_CATALOG.leagues);
    expect(groups.map((g) => g.group)).toEqual(['open_wheel', 'stock_car', 'sports_car', 'motorcycle', 'rally', 'drag_dirt']);
    expect(groups[0]?.series.map((s) => s.key)).toEqual(['f1', 'f2', 'indycar', 'formula_e']);
    expect(groups.flatMap((g) => g.series).some((s) => s.kind === 'team')).toBe(false);
  });

  it('puts unknown groups after the known ones', () => {
    const groups = motorsportGroups([
      { key: 'x', name: 'X', kind: 'motorsport', group: 'hovercraft', teams: [] },
      { key: 'f1', name: 'F1', kind: 'motorsport', group: 'open_wheel', teams: [] },
    ]);
    expect(groups.map((g) => g.group)).toEqual(['open_wheel', 'hovercraft']);
  });

  it('filters teams by name, nickname or abbreviation', () => {
    const nfl = MOCK_SPORTS_CATALOG.leagues.find((l) => l.key === 'nfl')!.teams;
    expect(filterTeams(nfl, 'bea').map((t) => t.key)).toEqual(['chi']);
    expect(filterTeams(nfl, 'GB').map((t) => t.key)).toEqual(['gb']);
    expect(filterTeams(nfl, '  ')).toHaveLength(nfl.length);
  });

  it('labels a match by the followed teams, else every matched team, else the league', () => {
    expect(followLabel(match(1, NOW, NOW + H, { teams: ['chi', 'gb'], followedTeams: ['chi'] }), MOCK_SPORTS_CATALOG)).toBe('Bears · NFL');
    expect(followLabel(match(1, NOW, NOW + H, { teams: ['chi', 'gb'] }), MOCK_SPORTS_CATALOG)).toBe('Bears · Packers · NFL');
    expect(followLabel(match(1, NOW, NOW + H, { league: 'f1', leagueName: 'Formula 1' }), MOCK_SPORTS_CATALOG)).toBe('Formula 1');
  });
});

describe('sorting and the home row', () => {
  it('puts what is on air first, then by start time', () => {
    const later = match(1, NOW + 3 * H, NOW + 4 * H);
    const soon = match(2, NOW + H, NOW + 2 * H);
    const onNow = match(3, NOW - H, NOW + H);
    const eventChannel = match(4, null, null);
    expect(sortMatches([later, soon, onNow, eventChannel], NOW).map((m) => m.channel.id)).toEqual([4, 3, 2, 1]);
  });

  it('keeps on-now and the next 48 hours, drops ended and later matches, and shows each airing once', () => {
    const ended = match(1, NOW - 3 * H, NOW - H);
    const onNow = match(2, NOW - H, NOW + H);
    const tieOtherLeague = { ...onNow, league: 'nba', leagueName: 'NBA' };
    const tomorrow = match(3, NOW + 30 * H, NOW + 32 * H);
    const tooLate = match(4, NOW + 49 * H, NOW + 50 * H);
    const items = homeSportsItems([tooLate, tomorrow, ended, onNow, tieOtherLeague], NOW, 48 * H);
    expect(items.map((m) => m.channel.id)).toEqual([2, 3]);
  });

  it('caps the row', () => {
    const many = Array.from({ length: 20 }, (_, i) => match(i + 1, NOW + i * H, NOW + (i + 1) * H));
    expect(homeSportsItems(many, NOW, 48 * H, 12)).toHaveLength(12);
  });

  it('is empty when nothing matches', () => {
    expect(homeSportsItems([], NOW, 48 * H)).toEqual([]);
  });
});

describe('event channels with a named start', () => {
  it('count as on now without a time, and as upcoming when the named start is ahead', () => {
    const noTime = match(1, null, null);
    const later = match(2, null, null, { eventStartUtc: NOW + 2 * H });
    const started = match(3, null, null, { eventStartUtc: NOW - H });
    expect(onAirNow(noTime, NOW)).toBe(true);
    expect(onAirNow(later, NOW)).toBe(false);
    expect(onAirNow(started, NOW)).toBe(true);
    // Upcoming event channels sort by their named start among timed matches.
    const timed = match(4, NOW + H, NOW + 3 * H);
    expect(sortMatches([later, timed, noTime], NOW).map((m) => m.channel.id)).toEqual([1, 4, 2]);
    // And drop out of the home row when they start past the horizon.
    expect(homeSportsItems([match(5, null, null, { eventStartUtc: NOW + 72 * H })], NOW, 48 * H)).toEqual([]);
  });
});
