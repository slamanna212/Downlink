// Pure helpers for the sports follow document and match lists (SPORTS §3, §6). No React, no IPC, so the
// toggling rules, session defaults and home-row ordering are unit-tested on their own.
import type { SportsCatalog, SportsFollow, SportsGroup, SportsLeague, SportsMatch, SportsSession, SportsTeam } from '../../api/types';
import { dayOffset, isLiveNow } from '../../lib/time';

/** Race, qualifying and practice are on for a newly followed series; shows are off (SPORTS §2.3). */
export const DEFAULT_SESSIONS: readonly SportsSession[] = ['race', 'qualifying', 'practice'];

export const SESSION_ORDER: readonly SportsSession[] = ['race', 'qualifying', 'practice', 'shows'];

export const GROUP_ORDER: readonly SportsGroup[] = ['open_wheel', 'stock_car', 'sports_car', 'motorcycle', 'rally', 'drag_dirt'];

export const emptyFollow = (): SportsFollow => ({ version: 1, includeReplays: false, leagues: {}, series: {}, priority: [] });

export function setLeagueFollowed(f: SportsFollow, league: string, on: boolean): SportsFollow {
  const leagues = { ...f.leagues };
  if (on) leagues[league] ??= { teams: [] };
  else delete leagues[league];
  return { ...f, leagues };
}

/** Adds or removes one team. Picking teams narrows the league; removing the last goes back to all games. */
export function toggleTeam(f: SportsFollow, league: string, team: string): SportsFollow {
  const current = f.leagues[league]?.teams ?? [];
  const teams = current.includes(team) ? current.filter((x) => x !== team) : [...current, team];
  return { ...f, leagues: { ...f.leagues, [league]: { teams } } };
}

/** "All games": the league stays followed with no team filter. */
export const allGames = (f: SportsFollow, league: string): SportsFollow => ({ ...f, leagues: { ...f.leagues, [league]: { teams: [] } } });

export function setSeriesFollowed(f: SportsFollow, series: string, on: boolean): SportsFollow {
  const next = { ...f.series };
  if (on) next[series] ??= { sessions: [...DEFAULT_SESSIONS] };
  else delete next[series];
  return { ...f, series: next };
}

export function toggleSession(f: SportsFollow, series: string, session: SportsSession): SportsFollow {
  const current = f.series[series]?.sessions ?? [];
  const sessions = current.includes(session) ? current.filter((x) => x !== session) : [...current, session];
  sessions.sort((a, b) => SESSION_ORDER.indexOf(a) - SESSION_ORDER.indexOf(b));
  return { ...f, series: { ...f.series, [series]: { sessions } } };
}

export const setIncludeReplays = (f: SportsFollow, includeReplays: boolean): SportsFollow => ({ ...f, includeReplays });

/** True when anything in the follow is still in the catalog (unknown keys are kept but ignored, SPORTS §3). */
export function followsAnything(f: SportsFollow | undefined, catalog?: SportsCatalog): boolean {
  if (!f) return false;
  const keys = [...Object.keys(f.leagues), ...Object.keys(f.series)];
  if (!catalog) return keys.length > 0;
  const known = new Set(catalog.leagues.map((l) => l.key));
  return keys.some((k) => known.has(k));
}

/** One rankable thing the user follows: a team, a league's "all games", or a series. */
export interface RankEntry {
  key: string;
  league: SportsLeague;
  team: SportsTeam | null;
}

/** Everything followed that the catalog knows, as rank entries in catalog order. */
function followedEntries(f: SportsFollow, catalog: SportsCatalog): RankEntry[] {
  const out: RankEntry[] = [];
  for (const league of catalog.leagues) {
    if (league.kind === 'motorsport') {
      if (f.series[league.key]) out.push({ key: league.key, league, team: null });
      continue;
    }
    const picked = f.leagues[league.key];
    if (!picked) continue;
    const teams = league.teams.filter((x) => picked.teams.includes(x.key));
    if (teams.length === 0) out.push({ key: league.key, league, team: null });
    for (const team of teams) out.push({ key: `${league.key}:${team.key}`, league, team });
  }
  return out;
}

/**
 * The command center order (COMMAND_CENTER §3): ranked entries first, in `priority` order, then the
 * rest in catalog order. Stale keys (unfollowed, or gone from the catalog) are skipped.
 */
export function rankEntries(f: SportsFollow, catalog: SportsCatalog): RankEntry[] {
  const all = followedEntries(f, catalog);
  const priority = f.priority ?? [];
  const index = (e: RankEntry) => {
    const i = priority.indexOf(e.key);
    return i < 0 ? priority.length : i;
  };
  return all.map((e, i) => ({ e, i })).sort((a, b) => index(a.e) - index(b.e) || a.i - b.i).map(({ e }) => e);
}

/** Moves one entry up (`-1`) or down (`+1`); the saved priority becomes the whole current order. */
export function moveRank(f: SportsFollow, catalog: SportsCatalog, key: string, delta: -1 | 1): SportsFollow {
  const keys = rankEntries(f, catalog).map((e) => e.key);
  const i = keys.indexOf(key);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= keys.length) return f;
  [keys[i], keys[j]] = [keys[j] as string, keys[i] as string];
  return { ...f, priority: keys };
}

/** Motorsport series grouped in SPORTS §2.2 order; unknown groups follow, in catalog order. */
export function motorsportGroups(leagues: SportsLeague[]): { group: string; series: SportsLeague[] }[] {
  const byGroup = new Map<string, SportsLeague[]>();
  for (const l of leagues) {
    if (l.kind !== 'motorsport') continue;
    const g = l.group ?? 'other';
    byGroup.set(g, [...(byGroup.get(g) ?? []), l]);
  }
  const known = GROUP_ORDER.filter((g) => byGroup.has(g));
  const rest = [...byGroup.keys()].filter((g) => !(GROUP_ORDER as readonly string[]).includes(g));
  return [...known, ...rest].map((group) => ({ group, series: byGroup.get(group) ?? [] }));
}

/** Teams whose name, nickname or abbreviation contains the query (case-insensitive). */
export function filterTeams<T extends { name: string; short: string; abbr: string }>(teams: T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return teams;
  return teams.filter((t) => t.name.toLowerCase().includes(q) || t.short.toLowerCase().includes(q) || t.abbr.toLowerCase().includes(q));
}

/** When a match starts: the programme's start, or the time named by an event channel. */
export const matchStart = (m: SportsMatch): number | null => m.programme?.startUtc ?? m.eventStartUtc;

/**
 * On air at `now`. An event channel counts as on now unless its name gives a start still ahead
 * (SPORTS §5).
 */
export const onAirNow = (m: SportsMatch, now: number): boolean =>
  m.programme ? isLiveNow(m.programme.startUtc, m.programme.endUtc, now) : m.eventStartUtc == null || m.eventStartUtc <= now;

/** On air first, then by start time (channel-only matches before timed ones), then by score. */
export function sortMatches(list: SportsMatch[], now: number): SportsMatch[] {
  return [...list].sort((a, b) => {
    const air = Number(onAirNow(b, now)) - Number(onAirNow(a, now));
    if (air) return air;
    const sa = matchStart(a) ?? -Infinity;
    const sb = matchStart(b) ?? -Infinity;
    if (sa !== sb) return sa < sb ? -1 : 1;
    return b.score - a.score;
  });
}

/**
 * The matched teams for a label: the followed ones when there are any, else every team that matched,
 * as nicknames from the catalog (keys unknown to the catalog are skipped).
 */
export function matchTeams(m: SportsMatch, catalog: SportsCatalog | undefined): string[] {
  const league = catalog?.leagues.find((l) => l.key === m.league);
  const keys = m.followedTeams.length ? m.followedTeams : m.teams;
  return keys.flatMap((k) => {
    const team = league?.teams.find((x) => x.key === k);
    return team ? [team.short] : [];
  });
}

/** What the user follows that this match is for: "Bears · NFL", or the series name. */
export function followLabel(m: SportsMatch, catalog: SportsCatalog | undefined): string {
  const teams = matchTeams(m, catalog);
  return teams.length ? `${teams.join(' · ')} · ${m.leagueName}` : m.leagueName;
}

/**
 * Home row content (SPORTS §6 phase 2): what is on now, then what starts within `horizon` seconds,
 * on-air first. One card per channel + programme (a programme matched for two leagues shows once).
 */
export function homeSportsItems(list: SportsMatch[], now: number, horizon: number, limit = 9): SportsMatch[] {
  const seen = new Set<string>();
  const out: SportsMatch[] = [];
  for (const m of sortMatches(list, now)) {
    if (m.programme && (m.programme.endUtc <= now || m.programme.startUtc >= now + horizon)) continue;
    if (!m.programme && m.eventStartUtc != null && m.eventStartUtc >= now + horizon) continue;
    const key = `${m.channel.id}:${m.programme?.id ?? ''}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(m);
    if (out.length >= limit) break;
  }
  return out;
}

/** Which time group a match falls in (settings preview): on air, later today, tomorrow, or a later day. */
export type MatchWhen = 'now' | 'today' | 'tomorrow' | 'later';

export function matchWhen(m: SportsMatch, now: number): MatchWhen {
  if (onAirNow(m, now)) return 'now';
  const days = dayOffset(matchStart(m) ?? now, now);
  return days <= 0 ? 'today' : days === 1 ? 'tomorrow' : 'later';
}

/** Sorted matches split into time groups, in order; empty groups are left out. */
export function groupByWhen(list: SportsMatch[], now: number): { when: MatchWhen; matches: SportsMatch[] }[] {
  const order: MatchWhen[] = ['now', 'today', 'tomorrow', 'later'];
  const by = new Map<MatchWhen, SportsMatch[]>();
  for (const m of sortMatches(list, now)) {
    const w = matchWhen(m, now);
    by.set(w, [...(by.get(w) ?? []), m]);
  }
  return order.filter((w) => by.has(w)).map((when) => ({ when, matches: by.get(when) ?? [] }));
}

/** Picker tab order (SPORTS §2): team sports in catalog sport order, motorsport, then anything newer. */
export const SPORT_ORDER: readonly string[] = ['football', 'basketball', 'hockey', 'baseball', 'soccer', 'motorsport', 'other'];

/** The sports present in the catalog, in picker order (unknown sports after, in catalog order). */
export function catalogSports(leagues: SportsLeague[]): string[] {
  const present = [...new Set(leagues.map((l) => l.sport))];
  return [...SPORT_ORDER.filter((s) => present.includes(s)), ...present.filter((s) => !SPORT_ORDER.includes(s))];
}

/** One league block in the add dialog: the league, and the teams that match the search. */
export interface PickerBlock {
  league: SportsLeague;
  teams: SportsTeam[];
}

/**
 * Add-dialog content: leagues in `sport` (or all), filtered by `query`. A query that names the league
 * keeps every team; otherwise only matching teams show, and leagues with none drop out. Series match
 * on their name only.
 */
export function pickerBlocks(leagues: SportsLeague[], sport: string | null, query: string): PickerBlock[] {
  const q = query.trim().toLowerCase();
  const out: PickerBlock[] = [];
  for (const league of leagues) {
    if (sport && league.sport !== sport) continue;
    const leagueHit = !q || league.name.toLowerCase().includes(q) || league.key.toLowerCase() === q;
    if (league.kind === 'motorsport') {
      if (leagueHit) out.push({ league, teams: [] });
      continue;
    }
    const teams = leagueHit ? league.teams : filterTeams(league.teams, q);
    if (teams.length) out.push({ league, teams });
  }
  return out;
}

/** Removes one followed team; removing the last one unfollows the league (the chip list never shows "all"). */
export function removeTeam(f: SportsFollow, league: string, team: string): SportsFollow {
  const current = f.leagues[league]?.teams ?? [];
  const teams = current.filter((x) => x !== team);
  return teams.length ? { ...f, leagues: { ...f.leagues, [league]: { teams } } } : setLeagueFollowed(f, league, false);
}

/** Removes one session; removing the last one unfollows the series. */
export function removeSession(f: SportsFollow, series: string, session: SportsSession): SportsFollow {
  const sessions = (f.series[series]?.sessions ?? []).filter((x) => x !== session);
  return sessions.length ? { ...f, series: { ...f.series, [series]: { sessions } } } : setSeriesFollowed(f, series, false);
}
