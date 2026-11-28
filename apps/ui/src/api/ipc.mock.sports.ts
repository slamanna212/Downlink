// Sports tracking for the mock backend. SYNTHETIC data only (CLAUDE rule 16): a trimmed catalog with a
// few teams per league, and invented fixtures placed on mock channels relative to "now".
import type { Channel, Programme, SportsCatalog, SportsFollow, SportsLeague, SportsMatch, SportsSession, SportsTeam } from './types';

const team = (key: string, name: string, short: string, abbr: string): SportsTeam => ({ key, name, short, abbr });
const teamLeague = (key: string, name: string, sport: string, teams: SportsTeam[]): SportsLeague => ({ key, name, kind: 'team', group: null, sport, teams });
const series = (key: string, name: string, group: string): SportsLeague => ({ key, name, kind: 'motorsport', group, sport: 'motorsport', teams: [] });

export const MOCK_SPORTS_CATALOG: SportsCatalog = {
  sessions: ['race', 'qualifying', 'practice', 'shows'],
  leagues: [
    teamLeague('nfl', 'NFL', 'football', [
      team('chi', 'Chicago Bears', 'Bears', 'CHI'), team('gb', 'Green Bay Packers', 'Packers', 'GB'),
      team('det', 'Detroit Lions', 'Lions', 'DET'), team('min', 'Minnesota Vikings', 'Vikings', 'MIN'),
      team('kc', 'Kansas City Chiefs', 'Chiefs', 'KC'), team('buf', 'Buffalo Bills', 'Bills', 'BUF'),
    ]),
    teamLeague('mlb', 'MLB', 'baseball', [
      team('chc', 'Chicago Cubs', 'Cubs', 'CHC'), team('stl', 'St. Louis Cardinals', 'Cardinals', 'STL'),
      team('sea', 'Seattle Mariners', 'Mariners', 'SEA'), team('ath', 'Athletics', 'Athletics', 'ATH'),
    ]),
    teamLeague('nba', 'NBA', 'basketball', [
      team('bos', 'Boston Celtics', 'Celtics', 'BOS'), team('den', 'Denver Nuggets', 'Nuggets', 'DEN'),
      team('lal', 'Los Angeles Lakers', 'Lakers', 'LAL'), team('mia', 'Miami Heat', 'Heat', 'MIA'),
    ]),
    teamLeague('nhl', 'NHL', 'hockey', [
      team('uta', 'Utah Mammoth', 'Mammoth', 'UTA'), team('sea', 'Seattle Kraken', 'Kraken', 'SEA'),
      team('lak', 'Los Angeles Kings', 'Kings', 'LAK'), team('nyr', 'New York Rangers', 'Rangers', 'NYR'),
    ]),
    teamLeague('mls', 'MLS', 'soccer', [
      team('atl', 'Atlanta United', 'Atlanta', 'ATL'), team('sd', 'San Diego FC', 'San Diego', 'SD'),
      team('sea', 'Seattle Sounders', 'Sounders', 'SEA'), team('clb', 'Columbus Crew', 'Crew', 'CLB'),
    ]),
    teamLeague('epl', 'Premier League', 'soccer', [
      team('ars', 'Arsenal', 'Arsenal', 'ARS'), team('che', 'Chelsea', 'Chelsea', 'CHE'),
      team('liv', 'Liverpool', 'Liverpool', 'LIV'), team('sun', 'Sunderland', 'Sunderland', 'SUN'),
      team('hul', 'Hull City', 'Hull', 'HUL'), team('cov', 'Coventry City', 'Coventry', 'COV'),
    ]),
    series('f1', 'Formula 1', 'open_wheel'),
    series('f2', 'F2', 'open_wheel'),
    series('indycar', 'IndyCar', 'open_wheel'),
    series('formula_e', 'Formula E', 'open_wheel'),
    series('nascar_cup', 'NASCAR Cup', 'stock_car'),
    series('nascar_truck', 'NASCAR Craftsman Truck Series', 'stock_car'),
    series('wec', 'WEC', 'sports_car'),
    series('imsa', 'IMSA', 'sports_car'),
    series('motogp', 'MotoGP', 'motorcycle'),
    series('wsbk', 'WorldSBK', 'motorcycle'),
    series('wrc', 'WRC', 'rally'),
    series('nhra', 'NHRA', 'drag_dirt'),
  ],
};

/** `?mock=sports` starts with something followed so the home row shows. */
export function mockSportsFollow(prefilled: boolean): SportsFollow {
  return prefilled
    ? { version: 1, includeReplays: false, leagues: { nfl: { teams: ['chi'] }, epl: { teams: [] } }, series: { f1: { sessions: ['race', 'qualifying', 'practice'] }, nascar_cup: { sessions: ['race', 'qualifying', 'practice'] } } }
    : { version: 1, includeReplays: false, leagues: {}, series: {} };
}

interface Fixture {
  league: string;
  teams?: string[];
  session?: SportsSession;
  /** Minutes from now (negative = already on). */
  start: number;
  minutes: number;
  title: string;
  subtitle?: string;
  /** Index into the channel list. */
  channel: number;
  live: boolean;
  liveInferred?: boolean;
  /** Channel-name-only match on an event channel with this name (no programme). */
  eventChannel?: string;
  /** Other channels (indexes) showing the same event: the backend's grouped copies. */
  alternates?: number[];
}

const FIXTURES: Fixture[] = [
  { league: 'nfl', teams: ['chi', 'gb'], start: -45, minutes: 210, title: 'NFL Football', subtitle: 'Bears at Packers', channel: 2, live: true, alternates: [3, 74] },
  { league: 'nfl', teams: ['det', 'min'], start: 200, minutes: 200, title: 'NFL Football', subtitle: 'Vikings at Lions', channel: 20, live: true },
  { league: 'nfl', teams: ['kc', 'chi'], start: 26 * 60, minutes: 200, title: 'NFL Football', subtitle: 'Chiefs at Bears', channel: 38, live: true },
  { league: 'nfl', teams: ['chi', 'buf'], start: 300, minutes: 180, title: 'NFL Greatest Games', subtitle: 'Bears vs Bills (classic)', channel: 56, live: false },
  { league: 'nfl', teams: ['chi', 'det'], start: 0, minutes: 0, title: '', channel: 74, live: true, eventChannel: 'NFL 03: Bears vs Lions 1:00 PM' },
  { league: 'epl', teams: ['ars', 'che'], start: -20, minutes: 115, title: 'Premier League', subtitle: 'Arsenal v Chelsea', channel: 92, live: true, liveInferred: true },
  // A fifth game on now, so the command center has overflow at the default 4-tile cap.
  { league: 'epl', teams: ['liv', 'cov'], start: -10, minutes: 115, title: 'Premier League', subtitle: 'Liverpool v Coventry City', channel: 100, live: true, liveInferred: true },
  { league: 'epl', teams: ['sun', 'hul'], start: 22 * 60, minutes: 115, title: 'Premier League', subtitle: 'Sunderland v Hull City', channel: 92, live: true },
  { league: 'nba', teams: ['bos', 'den'], start: 90, minutes: 150, title: 'NBA Basketball', subtitle: 'Celtics at Nuggets', channel: 110, live: true },
  { league: 'nhl', teams: ['lak', 'uta'], start: 330, minutes: 150, title: 'NHL Hockey', subtitle: 'Kings at Mammoth', channel: 128, live: true },
  { league: 'f1', session: 'qualifying', start: 180, minutes: 75, title: 'Formula 1 Qualifying', subtitle: 'Desert Grand Prix', channel: 13, live: true, alternates: [14] },
  { league: 'f1', session: 'race', start: 27 * 60, minutes: 120, title: 'Formula 1', subtitle: 'Desert Grand Prix', channel: 13, live: true },
  { league: 'f1', session: 'practice', start: 60, minutes: 60, title: 'Formula 1 Practice 3', subtitle: 'Desert Grand Prix', channel: 31, live: false },
  { league: 'f1', session: 'shows', start: 150, minutes: 30, title: 'F1 Pre-race Show', channel: 13, live: true },
  { league: 'nascar_cup', session: 'race', start: -70, minutes: 240, title: 'NASCAR Cup Series', subtitle: 'Harbor 400', channel: 49, live: true, liveInferred: true },
  { league: 'motogp', session: 'race', start: 16 * 60, minutes: 60, title: 'MotoGP', subtitle: 'Valley Grand Prix', channel: 67, live: true },
  { league: 'wec', session: 'race', start: 40 * 60, minutes: 360, title: 'WEC 6 Hours of the Coast', channel: 85, live: true },
];

/**
 * Matches for the follow in [startUtc, endUtc), filtered the way the backend filters (followed teams,
 * sessions, replays). `base` anchors the fixtures; `programmes` lets the details modal find them by id.
 */
export function sportsMatchesFor(follow: SportsFollow, channels: Channel[], startUtc: number, endUtc: number, now: number, base: number): {
  matches: SportsMatch[];
  programmes: Map<string, Programme>;
} {
  const programmes = new Map<string, Programme>();
  const matches: SportsMatch[] = [];
  for (const f of FIXTURES) {
    const league = MOCK_SPORTS_CATALOG.leagues.find((l) => l.key === f.league);
    const ch = channels[f.channel];
    if (!league || !ch) continue;
    if (!f.live && !follow.includeReplays) continue;
    let followedTeams: string[] = [];
    if (league.kind === 'team') {
      const picked = follow.leagues[f.league];
      if (!picked) continue;
      followedTeams = (f.teams ?? []).filter((t) => picked.teams.includes(t));
      if (picked.teams.length && !followedTeams.length) continue;
    } else {
      const picked = follow.series[f.league];
      if (!picked || !f.session || !picked.sessions.includes(f.session)) continue;
    }
    let programme: Programme | null = null;
    let channel = ch;
    if (f.eventChannel) {
      channel = { ...ch, name: f.eventChannel, logo: null };
    } else {
      // Fixtures sit on 5-minute marks so ids stay stable between refetches.
      const start = base + f.start * 60;
      const end = start + f.minutes * 60;
      if (end <= startUtc || start >= endUtc) continue;
      programme = {
        id: `${ch.id}:${start}`, channelId: ch.id, startUtc: start, endUtc: end, title: f.title, subtitle: f.subtitle ?? null,
        description: 'Synthetic sports listing generated for the mock backend.', season: null, episode: null,
        isNew: false, isLive: f.live && !f.liveInferred, isPremiere: false, isFinale: false, isScheduled: false,
      };
      programmes.set(programme.id, programme);
    }
    matches.push({
      channel, programme, league: f.league, leagueName: league.name, teams: f.teams ?? [], followedTeams,
      session: f.session ?? null, live: f.live, liveInferred: !!f.liveInferred, source: programme ? 'programme' : 'channel', score: 80,
      eventStartUtc: null,
      alternates: (f.alternates ?? []).flatMap((i) => (channels[i] ? [channels[i]] : [])),
    });
  }
  // Live first, then start time (the backend's order).
  const airing = (m: SportsMatch) => !m.programme || (m.programme.startUtc <= now && m.programme.endUtc > now);
  matches.sort((a, b) => Number(airing(b)) - Number(airing(a)) || (a.programme?.startUtc ?? 0) - (b.programme?.startUtc ?? 0));
  return { matches, programmes };
}
