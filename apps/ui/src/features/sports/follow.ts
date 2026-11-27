// Pure helpers for the sports follow document and match lists (SPORTS §3, §6). No React, no IPC, so the
// toggling rules, session defaults and home-row ordering are unit-tested on their own.
import type { SportsCatalog, SportsFollow, SportsGroup, SportsLeague, SportsMatch, SportsSession } from '../../api/types';
import { isLiveNow } from '../../lib/time';

/** Race, qualifying and practice are on for a newly followed series; shows are off (SPORTS §2.3). */
export const DEFAULT_SESSIONS: readonly SportsSession[] = ['race', 'qualifying', 'practice'];

export const SESSION_ORDER: readonly SportsSession[] = ['race', 'qualifying', 'practice', 'shows'];

export const GROUP_ORDER: readonly SportsGroup[] = ['open_wheel', 'stock_car', 'sports_car', 'motorcycle', 'rally', 'drag_dirt'];

export const emptyFollow = (): SportsFollow => ({ version: 1, includeReplays: false, leagues: {}, series: {} });

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
export function homeSportsItems(list: SportsMatch[], now: number, horizon: number, limit = 12): SportsMatch[] {
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
